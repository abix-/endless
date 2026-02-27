//! Combat systems - Attack processing using GPU targeting results

use bevy::prelude::*;
use crate::components::*;
use crate::messages::{DamageMsg, ProjGpuUpdate, ProjGpuUpdateMsg};
use crate::resources::{CombatDebug, GpuReadState, ProjSlotAllocator, ProjHitState, TowerState, SystemTimings, GameTime, EntityMap, MovementIntents, MovementPriority};
use crate::systems::stats::{TownUpgrades, resolve_town_tower_stats};
use crate::gpu::ProjBufferWrites;
use crate::world::{WorldData, BuildingKind, is_alive};

/// ECS queries for attack_system (bundled to stay under 16-param limit).
#[derive(bevy::ecs::system::SystemParam)]
pub struct AttackQueries<'w, 's> {
    pub manual_target_q: Query<'w, 's, &'static ManualTarget>,
    pub squad_id_q: Query<'w, 's, &'static SquadId>,
    pub activity_q: Query<'w, 's, &'static Activity>,
    pub cached_stats_q: Query<'w, 's, &'static CachedStats>,
    pub combat_state_q: Query<'w, 's, &'static mut CombatState>,
    pub timer_q: Query<'w, 's, &'static mut AttackTimer>,
}

/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    entity_map: Res<EntityMap>,
    mut debug: ResMut<CombatDebug>,
    timings: Res<SystemTimings>,
    mut timer_q: Query<(&EntitySlot, &mut AttackTimer)>,
) {
    let _t = timings.scope("cooldown");
    let dt = game_time.delta(&time);

    let mut first_timer_before = -99.0f32;
    let mut timer_count = 0usize;

    for (es, mut timer) in timer_q.iter_mut() {
        let Some(npc) = entity_map.get_npc(es.0) else { continue };
        if npc.dead { continue; }
        if timer_count == 0 {
            first_timer_before = timer.0;
        }
        timer_count += 1;

        if timer.0 > 0.0 {
            timer.0 = (timer.0 - dt).max(0.0);
        }
    }

    debug.sample_timer = first_timer_before;
    debug.cooldown_entities = timer_count;
    debug.frame_delta = dt;
}

/// Process attacks using GPU targeting results.
/// GPU finds nearest enemy, Bevy checks range and applies damage.
pub fn attack_system(
    mut intents: ResMut<MovementIntents>,
    mut proj_updates: MessageWriter<ProjGpuUpdateMsg>,
    mut damage_events: MessageWriter<DamageMsg>,
    mut debug: ResMut<CombatDebug>,
    gpu_state: Res<GpuReadState>,
    npc_gpu: Res<crate::gpu::EntityGpuState>,
    entity_map: Res<EntityMap>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    timings: Res<SystemTimings>,
    squad_state: Res<crate::resources::SquadState>,
    mut commands: Commands,
    mut aq: AttackQueries,
) {
    let _t = timings.scope("attack");
    let positions = &gpu_state.positions;
    let combat_targets = &gpu_state.combat_targets;

    let mut attackers = 0usize;
    let mut targets_found = 0usize;
    let mut attacks = 0usize;
    let mut chases = 0usize;
    let mut in_combat_added = 0usize;
    let mut sample_target = -99i32;
    let mut bounds_failures = 0usize;
    let mut sample_dist = -1.0f32;
    let mut in_range_count = 0usize;
    let mut timer_ready_count = 0usize;
    let mut sample_timer = -1.0f32;

    // Collect NPC slots to avoid borrow conflict (need &building from same EntityMap)
    let npc_slots: Vec<usize> = entity_map.iter_npcs()
        .filter(|n| !n.dead)
        .map(|n| n.slot)
        .collect();

    for slot in npc_slots {
        // Read NPC data (immutable borrow scope)
        let (entity, i, cached_range, cached_damage, cached_cooldown, cached_proj_speed, cached_proj_lifetime, faction_id, job, activity_skip, manual_target_clone, squad_id_val, is_fighting) = {
            let npc = entity_map.get_npc(slot).unwrap();
            let activity_skip = aq.activity_q.get(npc.entity).is_ok_and(|a| matches!(
                *a,
                Activity::Returning { .. } | Activity::GoingToRest | Activity::Resting
                    | Activity::GoingToHeal | Activity::HealingAtFountain { .. }
            ));
            let stats = aq.cached_stats_q.get(npc.entity).unwrap();
            let is_fighting = aq.combat_state_q.get(npc.entity).is_ok_and(|cs| cs.is_fighting());
            (npc.entity, npc.slot, stats.range, stats.damage,
             stats.cooldown, stats.projectile_speed,
             stats.projectile_lifetime, npc.faction, npc.job,
             activity_skip, aq.manual_target_q.get(npc.entity).ok().cloned(),
             aq.squad_id_q.get(npc.entity).ok().map(|s| s.0), is_fighting)
        };

        attackers += 1;

        // Don't auto-engage while NPC is in survival/transit states.
        if activity_skip {
            if is_fighting {
                if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::None; }
            }
            continue;
        }

        // Manual target override
        let mut target_idx = if let Some(ref mt) = manual_target_clone {
            match mt {
                ManualTarget::Npc(t) => {
                    let dead = gpu_state.health.get(*t).map_or(true, |&h| h <= 0.0);
                    if dead {
                        commands.entity(entity).remove::<ManualTarget>();
                        combat_targets.get(i).copied().unwrap_or(-1)
                    } else {
                        *t as i32
                    }
                }
                ManualTarget::Building(_) | ManualTarget::Position(_) => {
                    combat_targets.get(i).copied().unwrap_or(-1)
                }
            }
        } else {
            let hold = squad_id_val.and_then(|sid| squad_state.squads.get(sid as usize))
                .map_or(false, |sq| sq.hold_fire);
            if hold { -1 } else { combat_targets.get(i).copied().unwrap_or(-1) }
        };

        // Sticky building target
        if target_idx >= 0 {
            let ti = target_idx as usize;
            if entity_map.get_instance(ti).is_some() && i * 2 + 1 < npc_gpu.targets.len() {
                let current_target = Vec2::new(npc_gpu.targets[i * 2], npc_gpu.targets[i * 2 + 1]);
                if let Some(curr_inst) = entity_map.find_by_position(current_target) {
                    if curr_inst.faction >= 0 && curr_inst.faction != faction_id {
                        target_idx = curr_inst.slot as i32;
                    }
                }
            }
        }

        if attackers == 1 { sample_target = target_idx; }

        if target_idx < 0 {
            if is_fighting { if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::None; } }
            continue;
        }

        let ti = target_idx as usize;
        targets_found += 1;

        if ti == i {
            if is_fighting { if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::None; } }
            continue;
        }

        if i * 2 + 1 >= positions.len() { bounds_failures += 1; continue; }
        let (x, y) = (positions[i * 2], positions[i * 2 + 1]);
        if x < -9000.0 { continue; }

        // ── Building target ──
        if let Some(inst) = entity_map.get_instance(ti) {
            if !matches!(job, Job::Archer | Job::Crossbow | Job::Raider) { continue; }
            if inst.faction == faction_id { continue; }

            let dx = inst.position.x - x;
            let dy = inst.position.y - y;
            let dist = (dx * dx + dy * dy).sqrt();
            let inst_pos = inst.position;

            let close_chase_radius = cached_range + 120.0;
            if dist <= cached_range {
                intents.submit(entity, Vec2::new(x, y), MovementPriority::Combat, "combat:hold_building");
                in_range_count += 1;
                let timer = aq.timer_q.get(entity).map(|t| t.0).unwrap_or(0.0);
                if timer <= 0.0 {
                    timer_ready_count += 1;
                    if dist > 1.0 {
                        if let Some(proj_slot) = proj_alloc.alloc() {
                            let dir_x = dx / dist;
                            let dir_y = dy / dist;
                            proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Spawn {
                                idx: proj_slot, x, y,
                                vx: dir_x * cached_proj_speed, vy: dir_y * cached_proj_speed,
                                damage: cached_damage, faction: faction_id,
                                shooter: i as i32, lifetime: cached_proj_lifetime,
                            }));
                        }
                    } else {
                        damage_events.write(DamageMsg {
                            entity_idx: ti, amount: cached_damage,
                            attacker: i as i32, attacker_faction: faction_id,
                        });
                    }
                    attacks += 1;
                    if let Ok(mut t) = aq.timer_q.get_mut(entity) { t.0 = cached_cooldown; }
                }
            } else if dist <= close_chase_radius {
                intents.submit(entity, inst_pos, MovementPriority::Combat, "combat:chase_building");
                chases += 1;
            }
            continue;
        }

        // ── NPC target ──
        if entity_map.get_npc(ti).is_none() {
            if is_fighting { if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::None; } }
            continue;
        }
        let target_faction = gpu_state.factions.get(ti).copied().unwrap_or(-1);
        if target_faction < 0 || target_faction == faction_id {
            if is_fighting { if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::None; } }
            continue;
        }
        if gpu_state.health.get(ti).copied().unwrap_or(0.0) <= 0.0 {
            if is_fighting { if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::None; } }
            continue;
        }
        if ti * 2 + 1 >= positions.len() { bounds_failures += 1; continue; }

        if !is_fighting {
            if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::Fighting { origin: Vec2::new(x, y) }; }
            in_combat_added += 1;
        }
        let (tx, ty) = (positions[ti * 2], positions[ti * 2 + 1]);

        if tx < -9000.0 {
            if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) { *cs = CombatState::None; }
            continue;
        }

        let dx = tx - x;
        let dy = ty - y;
        let dist = (dx * dx + dy * dy).sqrt();

        if attackers == 1 { sample_dist = dist; }

        if dist <= cached_range {
            intents.submit(entity, Vec2::new(x, y), MovementPriority::Combat, "combat:hold_npc");
            in_range_count += 1;
            let timer = aq.timer_q.get(entity).map(|t| t.0).unwrap_or(0.0);
            if in_range_count == 1 { sample_timer = timer; }
            if timer <= 0.0 {
                timer_ready_count += 1;
                if dist > 1.0 {
                    if let Some(proj_slot) = proj_alloc.alloc() {
                        let dir_x = dx / dist;
                        let dir_y = dy / dist;
                        proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Spawn {
                            idx: proj_slot, x, y,
                            vx: dir_x * cached_proj_speed, vy: dir_y * cached_proj_speed,
                            damage: cached_damage, faction: faction_id,
                            shooter: i as i32, lifetime: cached_proj_lifetime,
                        }));
                    }
                } else {
                    damage_events.write(DamageMsg {
                        entity_idx: ti, amount: cached_damage,
                        attacker: i as i32, attacker_faction: faction_id,
                    });
                }
                attacks += 1;
                if let Ok(mut t) = aq.timer_q.get_mut(entity) { t.0 = cached_cooldown; }
            }
        } else {
            intents.submit(entity, Vec2::new(tx, ty), MovementPriority::Combat, "combat:chase_npc");
            chases += 1;
        }
    }

    debug.attackers_queried = attackers;
    debug.targets_found = targets_found;
    debug.attacks_made = attacks;
    debug.chases_started = chases;
    debug.in_combat_added = in_combat_added;
    debug.sample_target_idx = sample_target;
    debug.positions_len = positions.len();
    debug.combat_targets_len = combat_targets.len();
    debug.bounds_failures = bounds_failures;
    debug.sample_dist = sample_dist;
    debug.in_range_count = in_range_count;
    debug.timer_ready_count = timer_ready_count;
    debug.sample_timer = sample_timer;
    debug.sample_combat_target_0 = combat_targets.get(0).copied().unwrap_or(-99);
    debug.sample_combat_target_1 = combat_targets.get(1).copied().unwrap_or(-99);
    debug.sample_pos_0 = (
        positions.get(0).copied().unwrap_or(-999.0),
        positions.get(1).copied().unwrap_or(-999.0),
    );
    debug.sample_pos_1 = (
        positions.get(2).copied().unwrap_or(-999.0),
        positions.get(3).copied().unwrap_or(-999.0),
    );
}

/// Process GPU projectile hits: convert to unified DamageMsg events and recycle slots.
/// Entity buffer layout: unified slot namespace (NPCs and buildings share [0..entity_count]).
/// damage_system routes by entity_idx to NPC or building path.
pub fn process_proj_hits(
    mut damage_events: MessageWriter<DamageMsg>,
    mut proj_updates: MessageWriter<ProjGpuUpdateMsg>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    proj_writes: Res<ProjBufferWrites>,
    mut hit_state: ResMut<ProjHitState>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("process_proj_hits");
    let max_slot = proj_alloc.next.min(hit_state.0.len());
    for (slot, hit) in hit_state.0[..max_slot].iter().enumerate() {
        if slot < proj_writes.active.len() && proj_writes.active[slot] == 0 {
            continue;
        }

        let hit_idx = hit[0];
        let processed = hit[1];

        if hit_idx >= 0 && processed == 0 {
            let damage = if slot < proj_writes.damages.len() {
                proj_writes.damages[slot]
            } else {
                0.0
            };

            if damage > 0.0 {
                let shooter = if slot < proj_writes.shooters.len() {
                    proj_writes.shooters[slot]
                } else {
                    -1
                };
                let attacker_faction = proj_writes.factions.get(slot).copied().unwrap_or(-1);
                damage_events.write(DamageMsg {
                    entity_idx: hit_idx as usize,
                    amount: damage,
                    attacker: shooter,
                    attacker_faction,
                });
            }

            proj_alloc.free(slot);
            proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Deactivate { idx: slot }));
        } else if hit_idx == -2 {
            proj_alloc.free(slot);
            proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Deactivate { idx: slot }));
        }
    }
    hit_state.0.clear();
}

/// Building tower auto-attack: reads GPU combat_targets readback for tower building slots.
/// GPU spatial grid targeting finds nearest enemy — same path as NPC targeting.
pub fn building_tower_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    gpu_state: Res<GpuReadState>,
    world_data: Res<WorldData>,
    upgrades: Res<TownUpgrades>,
    entity_map: Res<EntityMap>,
    mut tower: ResMut<TowerState>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    mut proj_updates: MessageWriter<ProjGpuUpdateMsg>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("building_tower");
    let dt = game_time.delta(&time);
    // --- Towns: sync state, refresh enabled from sprite_type == 0 (fountain) every tick ---
    while tower.town.timers.len() < world_data.towns.len() {
        tower.town.timers.push(0.0);
        tower.town.attack_enabled.push(false);
    }
    for (i, town) in world_data.towns.iter().enumerate() {
        if i < tower.town.attack_enabled.len() {
            tower.town.attack_enabled[i] = is_alive(town.center);
        }
    }

    for (i, town) in world_data.towns.iter().enumerate() {
        if i >= tower.town.attack_enabled.len() || !tower.town.attack_enabled[i] { continue; }

        // Decrement cooldown
        if i < tower.town.timers.len() && tower.town.timers[i] > 0.0 {
            tower.town.timers[i] = (tower.town.timers[i] - dt).max(0.0);
            if tower.town.timers[i] > 0.0 { continue; }
        }

        let stats = resolve_town_tower_stats(&upgrades.town_levels(i));
        let pos = town.center;
        let faction = town.faction;

        // Look up tower building slot via EntityMap (one Fountain per town)
        debug_assert!(entity_map.iter_kind_for_town(BuildingKind::Fountain, i as u32).count() <= 1,
            "multiple fountains in town {i}");
        let bld_slot = entity_map.iter_kind_for_town(BuildingKind::Fountain, i as u32)
            .next().map(|inst| inst.slot);
        let Some(bld_slot) = bld_slot else { continue };

        // Read GPU combat_targets for this tower's entity index (unified slot)
        let target = gpu_state.combat_targets.get(bld_slot).copied().unwrap_or(-1);
        if target < 0 || entity_map.get_instance(target as usize).is_some() { continue; } // only target NPCs

        let ti = target as usize;
        let tx = gpu_state.positions.get(ti * 2).copied().unwrap_or(-9999.0);
        let ty = gpu_state.positions.get(ti * 2 + 1).copied().unwrap_or(-9999.0);
        if tx < -9000.0 { continue; }

        let dx = tx - pos.x;
        let dy = ty - pos.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist > stats.range { continue; } // out of range

        if dist > 1.0 {
            if let Some(proj_slot) = proj_alloc.alloc() {
                let dir_x = dx / dist;
                let dir_y = dy / dist;
                proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Spawn {
                    idx: proj_slot,
                    x: pos.x, y: pos.y,
                    vx: dir_x * stats.proj_speed,
                    vy: dir_y * stats.proj_speed,
                    damage: stats.damage,
                    faction,
                    shooter: -1,
                    lifetime: stats.proj_lifetime,
                }));
            }
        }
        if i < tower.town.timers.len() {
            tower.town.timers[i] = stats.cooldown;
        }
    }
}

/// Populate BuildingHpRender from building entity Health (only damaged buildings).
pub fn sync_building_hp_render(
    query: Query<(&Building, &EntitySlot, &Health), Without<Dead>>,
    gpu_state: Res<crate::gpu::EntityGpuState>,
    mut render: ResMut<crate::resources::BuildingHpRender>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("sync_hp_render");
    render.positions.clear();
    render.health_pcts.clear();
    let positions = &gpu_state.positions;
    for (building, npc_idx, health) in query.iter() {
        let max_hp = crate::constants::building_def(building.kind).hp;
        if health.0 <= 0.0 || health.0 >= max_hp { continue; }
        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }
        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        render.positions.push(Vec2::new(x, y));
        render.health_pcts.push(health.0 / max_hp);
    }
}
