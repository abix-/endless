//! Combat systems - Attack processing using GPU targeting results

use bevy::prelude::*;
use crate::components::*;
use crate::messages::{DamageMsg, ProjGpuUpdate, ProjGpuUpdateMsg};
use crate::resources::{CombatDebug, GpuReadState, ProjSlotAllocator, ProjHitState, TowerState, SystemTimings, GameTime, NpcEntityMap, MovementIntents, MovementPriority};
use crate::systems::stats::{TownUpgrades, resolve_town_tower_stats};
use crate::gpu::ProjBufferWrites;
use crate::resources::BuildingEntityMap;
use crate::world::{WorldData, BuildingKind, is_alive};


/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut query: Query<&mut AttackTimer>,
    mut debug: ResMut<CombatDebug>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("cooldown");
    let dt = game_time.delta(&time);

    let mut first_timer_before = -99.0f32;
    let mut timer_count = 0usize;

    for mut timer in query.iter_mut() {
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
    mut query: Query<(Entity, &NpcIndex, &CachedStats, &mut AttackTimer, &Faction, &mut CombatState, &Activity, &Job, Option<&ManualTarget>, Option<&SquadId>), Without<Dead>>,
    mut intents: ResMut<MovementIntents>,
    mut proj_updates: MessageWriter<ProjGpuUpdateMsg>,
    mut damage_events: MessageWriter<DamageMsg>,
    mut debug: ResMut<CombatDebug>,
    gpu_state: Res<GpuReadState>,
    npc_gpu: Res<crate::gpu::NpcGpuState>,
    npc_map: Res<NpcEntityMap>,
    bmap: Res<BuildingEntityMap>,
    slots: Res<crate::resources::SlotAllocator>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    timings: Res<SystemTimings>,
    squad_state: Res<crate::resources::SquadState>,
    mut commands: Commands,
) {
    let _t = timings.scope("attack");
    let positions = &gpu_state.positions;
    let combat_targets = &gpu_state.combat_targets;
    let npc_count = slots.count();

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

    for (entity, npc_idx, cached, mut timer, faction, mut combat_state, activity, job, manual_target, squad_id) in query.iter_mut() {
        attackers += 1;
        let i = npc_idx.0;

        // Don't auto-engage while NPC is in survival/transit states.
        // Healing states are intentionally excluded from combat retargeting to prevent
        // fountain <-> enemy target oscillation.
        if matches!(
            activity,
            Activity::Returning { .. }
                | Activity::GoingToRest
                | Activity::Resting
                | Activity::GoingToHeal
                | Activity::HealingAtFountain { .. }
        ) {
            if combat_state.is_fighting() {
                *combat_state = CombatState::None;
            }
            continue;
        }

        // Manual target override: player-assigned focus-fire takes priority over GPU auto-target.
        // Clear ManualTarget::Npc if the target is dead.
        let mut target_idx = if let Some(mt) = manual_target {
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
            // Hold fire: squad members with hold_fire skip auto-targeting
            let hold = squad_id.and_then(|sid| squad_state.squads.get(sid.0 as usize))
                .map_or(false, |sq| sq.hold_fire);
            if hold {
                -1 // don't auto-engage
            } else {
                combat_targets.get(i).copied().unwrap_or(-1)
            }
        };

        // Sticky building target: avoid cross-town building flops when GPU alternates nearest building.
        // If current movement target is an enemy building, keep it until invalid.
        if target_idx >= 0 {
            let ti = target_idx as usize;
            if ti >= npc_count && i * 2 + 1 < npc_gpu.targets.len() {
                let current_target = Vec2::new(npc_gpu.targets[i * 2], npc_gpu.targets[i * 2 + 1]);
                if let Some(curr_inst) = bmap.find_by_position(current_target) {
                    if curr_inst.faction >= 0 && curr_inst.faction != faction.0 {
                        target_idx = (npc_count + curr_inst.slot) as i32;
                    }
                }
            }
        }

        if attackers == 1 {
            sample_target = target_idx;
        }

        // No target from GPU
        if target_idx < 0 {
            if combat_state.is_fighting() {
                *combat_state = CombatState::None;
            }
            continue;
        }

        let ti = target_idx as usize;
        targets_found += 1;

        // Self-targeting guard
        if ti == i {
            if combat_state.is_fighting() { *combat_state = CombatState::None; }
            continue;
        }

        if i * 2 + 1 >= positions.len() {
            bounds_failures += 1;
            continue;
        }
        let (x, y) = (positions[i * 2], positions[i * 2 + 1]);
        if x < -9000.0 { continue; } // dead/hidden

        // ── Building target (entity_idx >= npc_count) ──
        if ti >= npc_count {
            let bld_slot = ti - npc_count;
            // Only combat jobs attack buildings
            if !matches!(job, Job::Archer | Job::Crossbow | Job::Raider) { continue; }
            let Some(inst) = bmap.get_instance(bld_slot) else { continue };
            // Don't attack same-faction buildings
            if inst.faction == faction.0 { continue; }

            let dx = inst.position.x - x;
            let dy = inst.position.y - y;
            let dist = (dx * dx + dy * dy).sqrt();

            // Building engagement gate:
            // - In-range => attack as normal.
            // - Out-of-range but very close => allow short chase to close distance.
            // - Far away => ignore to prevent cross-map building pursuit (archer "wander to enemy farm" bug).
            let close_chase_radius = cached.range + 120.0;
            if dist <= cached.range {
                // Stand ground while shooting
                intents.submit(entity, Vec2::new(x, y), MovementPriority::Combat, "combat:hold_building");
                in_range_count += 1;
                if timer.0 <= 0.0 {
                    timer_ready_count += 1;
                    if dist > 1.0 {
                        if let Some(proj_slot) = proj_alloc.alloc() {
                            let dir_x = dx / dist;
                            let dir_y = dy / dist;
                            proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Spawn {
                                idx: proj_slot,
                                x, y,
                                vx: dir_x * cached.projectile_speed,
                                vy: dir_y * cached.projectile_speed,
                                damage: cached.damage,
                                faction: faction.0,
                                shooter: i as i32,
                                lifetime: cached.projectile_lifetime,
                            }));
                        }
                    } else {
                        // Point blank — direct damage
                        damage_events.write(DamageMsg {
                            entity_idx: ti,
                            amount: cached.damage,
                            attacker: i as i32,
                            attacker_faction: faction.0,
                        });
                    }
                    attacks += 1;
                    timer.0 = cached.cooldown;
                }
            } else if dist <= close_chase_radius {
                // Short chase only; prevents unbounded cross-map pulls on distant enemy buildings.
                intents.submit(entity, inst.position, MovementPriority::Combat, "combat:chase_building");
                chases += 1;
            }
            continue;
        }

        // ── NPC target (entity_idx < npc_count) ──
        let target_entity = npc_map.0.get(&ti);
        if target_entity.is_none() {
            if combat_state.is_fighting() { *combat_state = CombatState::None; }
            continue;
        }
        let target_faction = gpu_state.factions.get(ti).copied().unwrap_or(-1);
        if target_faction < 0 || target_faction == faction.0 {
            if combat_state.is_fighting() { *combat_state = CombatState::None; }
            continue;
        }
        if gpu_state.health.get(ti).copied().unwrap_or(0.0) <= 0.0 {
            if combat_state.is_fighting() { *combat_state = CombatState::None; }
            continue;
        }

        if ti * 2 + 1 >= positions.len() {
            bounds_failures += 1;
            continue;
        }

        if !combat_state.is_fighting() {
            *combat_state = CombatState::Fighting { origin: Vec2::new(x, y) };
            in_combat_added += 1;
        }
        let (tx, ty) = (positions[ti * 2], positions[ti * 2 + 1]);

        // Skip dead/hidden targets (graveyard at ~-9999)
        if tx < -9000.0 {
            if combat_state.is_fighting() {
                *combat_state = CombatState::None;
            }
            continue;
        }

        let dx = tx - x;
        let dy = ty - y;
        let dist = (dx * dx + dy * dy).sqrt();

        if attackers == 1 {
            sample_dist = dist;
        }

        if dist <= cached.range {
            // Stand ground while fighting — set target to own position so NPC stops moving
            intents.submit(entity, Vec2::new(x, y), MovementPriority::Combat, "combat:hold_npc");

            in_range_count += 1;
            if in_range_count == 1 {
                sample_timer = timer.0;
            }
            if timer.0 <= 0.0 {
                timer_ready_count += 1;

                // Fire projectile toward target (avoid NaN when overlapping)
                if dist > 1.0 {
                    if let Some(proj_slot) = proj_alloc.alloc() {
                        let dir_x = dx / dist;
                        let dir_y = dy / dist;
                        proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Spawn {
                            idx: proj_slot,
                            x, y,
                            vx: dir_x * cached.projectile_speed,
                            vy: dir_y * cached.projectile_speed,
                            damage: cached.damage,
                            faction: faction.0,
                            shooter: i as i32,
                            lifetime: cached.projectile_lifetime,
                        }));
                    }
                } else {
                    // Point blank — apply damage directly (no projectile needed)
                    damage_events.write(DamageMsg {
                        entity_idx: ti,
                        amount: cached.damage,
                        attacker: i as i32,
                        attacker_faction: faction.0,
                    });
                }

                attacks += 1;
                timer.0 = cached.cooldown;
            }
        } else {
            // Out of range - chase target
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
/// Entity buffer layout: [0..npc_count] = NPCs, [npc_count..entity_count] = buildings.
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
/// Entity index for tower = npc_count + building_slot.
pub fn building_tower_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    gpu_state: Res<GpuReadState>,
    world_data: Res<WorldData>,
    upgrades: Res<TownUpgrades>,
    slots: Res<crate::resources::SlotAllocator>,
    bmap: Res<BuildingEntityMap>,
    mut tower: ResMut<TowerState>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    mut proj_updates: MessageWriter<ProjGpuUpdateMsg>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("building_tower");
    let dt = game_time.delta(&time);
    let npc_count = slots.count();

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

        // Look up tower building slot via BuildingEntityMap (one Fountain per town)
        debug_assert!(bmap.iter_kind_for_town(BuildingKind::Fountain, i as u32).count() <= 1,
            "multiple fountains in town {i}");
        let bld_slot = bmap.iter_kind_for_town(BuildingKind::Fountain, i as u32)
            .next().map(|inst| inst.slot);
        let Some(bld_slot) = bld_slot else { continue };

        // Read GPU combat_targets for this tower's entity index
        let entity_idx = npc_count + bld_slot;
        let target = gpu_state.combat_targets.get(entity_idx).copied().unwrap_or(-1);
        if target < 0 || target as usize >= npc_count { continue; } // only target NPCs

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
    query: Query<(&Building, &NpcIndex, &Health), Without<Dead>>,
    bld_gpu_state: Res<crate::gpu::BuildingGpuState>,
    mut render: ResMut<crate::resources::BuildingHpRender>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("sync_hp_render");
    render.positions.clear();
    render.health_pcts.clear();
    let positions = &bld_gpu_state.positions;
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
