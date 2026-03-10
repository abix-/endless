//! Combat systems - Attack processing using GPU targeting results

use crate::components::*;
use crate::gpu::ProjBufferWrites;
use crate::messages::{DamageMsg, ProjGpuUpdate, ProjGpuUpdateMsg};
use crate::resources::{
    CombatDebug, EntityMap, GameTime, GpuReadState, MovementPriority, PathRequestQueue,
    ProjHitState, ProjSlotAllocator, TowerState,
};
use crate::systems::stats::{TownUpgrades, resolve_town_tower_stats};
use crate::world::{BuildingKind, WorldData, is_alive};
use bevy::prelude::*;

/// Fire a projectile from source toward target. Returns true if fired.
fn fire_projectile(
    src: Vec2,
    target_pos: Vec2,
    damage: f32,
    proj_speed: f32,
    lifetime: f32,
    faction: i32,
    shooter: i32,
    proj_alloc: &mut ProjSlotAllocator,
    proj_updates: &mut MessageWriter<ProjGpuUpdateMsg>,
    sfx_writer: &mut MessageWriter<crate::resources::PlaySfxMsg>,
) -> bool {
    let delta = target_pos - src;
    let dist = delta.length();
    if dist <= 1.0 {
        return false;
    }
    if let Some(proj_slot) = proj_alloc.alloc() {
        let dir = delta / dist;
        proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Spawn {
            idx: proj_slot,
            x: src.x,
            y: src.y,
            vx: dir.x * proj_speed,
            vy: dir.y * proj_speed,
            damage,
            faction,
            shooter,
            lifetime,
        }));
        sfx_writer.write(crate::resources::PlaySfxMsg {
            kind: crate::resources::SfxKind::ArrowShoot,
            position: Some(src),
        });
        return true;
    }
    false
}

/// ECS queries for attack_system (bundled to stay under 16-param limit).
#[derive(bevy::ecs::system::SystemParam)]
pub struct AttackQueries<'w, 's> {
    pub combat_state_q: Query<'w, 's, &'static mut CombatState>,
    pub timer_q: Query<'w, 's, &'static mut AttackTimer>,
}

/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut debug: ResMut<CombatDebug>,
    mut timer_q: Query<(&GpuSlot, &mut AttackTimer), (Without<Building>, Without<Dead>)>,
) {
    let dt = game_time.delta(&time);

    let mut first_timer_before = -99.0f32;
    let mut timer_count = 0usize;

    for (_es, mut timer) in timer_q.iter_mut() {
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
    mut intents: ResMut<PathRequestQueue>,
    mut proj_updates: MessageWriter<ProjGpuUpdateMsg>,
    mut damage_events: MessageWriter<DamageMsg>,
    mut sfx_writer: MessageWriter<crate::resources::PlaySfxMsg>,
    mut debug: ResMut<CombatDebug>,
    gpu_state: Res<GpuReadState>,
    npc_gpu: Res<crate::gpu::EntityGpuState>,
    entity_map: Res<EntityMap>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    squad_state: Res<crate::resources::SquadState>,
    mut commands: Commands,
    game_time: Res<GameTime>,
    mut aq: AttackQueries,
    npc_q: Query<
        (
            Entity,
            &GpuSlot,
            &Job,
            &Faction,
            &CachedStats,
            &Activity,
            &Health,
            Option<&SquadId>,
            Option<&ManualTarget>,
        ),
        (Without<Building>, Without<Dead>),
    >,
) {
    if game_time.is_paused() {
        return;
    }
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

    for (entity, slot, job, faction, stats, activity, health, squad_id_opt, manual_target_opt) in
        npc_q.iter()
    {
        let i = slot.0;
        let faction_id = faction.0;
        let job = *job;
        let cached_range = stats.range;
        // Berserker/Timid: damage modifier when below 50% HP
        let cached_damage = if stats.berserk_bonus != 0.0 && stats.max_health > 0.0 && health.0 / stats.max_health < 0.5 {
            (stats.damage * (1.0 + stats.berserk_bonus)).max(0.0)
        } else {
            stats.damage
        };
        let cached_cooldown = stats.cooldown;
        let cached_proj_speed = stats.projectile_speed;
        let cached_proj_lifetime = stats.projectile_lifetime;
        let activity_skip = activity.kind.distraction() == Distraction::None;
        let squad_id_val = squad_id_opt.map(|s| s.0);
        let is_fighting = aq
            .combat_state_q
            .get(entity)
            .is_ok_and(|cs| cs.is_fighting());

        attackers += 1;

        // Don't auto-engage while NPC is in survival/transit states.
        if activity_skip {
            if is_fighting {
                if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) {
                    *cs = CombatState::None;
                }
            }
            continue;
        }

        // Manual target override
        let mut target_idx = if let Some(mt) = manual_target_opt {
            match mt {
                ManualTarget::Npc(t) => {
                    let dead = entity_map.get_npc(*t).is_none_or(|n| n.dead);
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
            let hold = squad_id_val
                .and_then(|sid| squad_state.squads.get(sid as usize))
                .is_some_and(|sq| sq.hold_fire);
            if hold {
                -1
            } else {
                combat_targets.get(i).copied().unwrap_or(-1)
            }
        };

        // Sticky building target
        if target_idx >= 0 {
            let ti = target_idx as usize;
            if entity_map.get_instance(ti).is_some() && i * 2 + 1 < npc_gpu.targets.len() {
                let current_target = Vec2::new(npc_gpu.targets[i * 2], npc_gpu.targets[i * 2 + 1]);
                if let Some(curr_inst) = entity_map.find_by_position(current_target) {
                    if curr_inst.faction != crate::constants::FACTION_NEUTRAL && curr_inst.faction != faction_id {
                        target_idx = curr_inst.slot as i32;
                    }
                }
            }
        }

        if attackers == 1 {
            sample_target = target_idx;
        }

        if target_idx < 0 {
            if is_fighting {
                if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) {
                    *cs = CombatState::None;
                }
            }
            continue;
        }

        let ti = target_idx as usize;
        targets_found += 1;

        if ti == i {
            if is_fighting {
                if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) {
                    *cs = CombatState::None;
                }
            }
            continue;
        }

        if i * 2 + 1 >= positions.len() {
            bounds_failures += 1;
            continue;
        }
        let (x, y) = (positions[i * 2], positions[i * 2 + 1]);
        if x < -9000.0 {
            continue;
        }

        // ── Building target ──
        if let Some(inst) = entity_map.get_instance(ti) {
            if inst.kind.is_road() {
                continue;
            }
            if !matches!(job, Job::Archer | Job::Crossbow | Job::Raider) {
                continue;
            }
            if inst.faction == faction_id {
                continue;
            }

            let dx = inst.position.x - x;
            let dy = inst.position.y - y;
            let dist = (dx * dx + dy * dy).sqrt();
            let inst_pos = inst.position;

            let close_chase_radius = cached_range + 120.0;
            if dist <= cached_range {
                intents.submit(
                    entity,
                    Vec2::new(x, y),
                    MovementPriority::Combat,
                    "combat:hold_building",
                );
                in_range_count += 1;
                let timer = aq.timer_q.get(entity).map(|t| t.0).unwrap_or(0.0);
                if timer <= 0.0 {
                    timer_ready_count += 1;
                    if !fire_projectile(
                        Vec2::new(x, y), inst_pos,
                        cached_damage, cached_proj_speed, cached_proj_lifetime,
                        faction_id, i as i32, &mut proj_alloc, &mut proj_updates, &mut sfx_writer,
                    ) {
                        if let Some(target_uid) = entity_map.uid_for_slot(ti) {
                            damage_events.write(DamageMsg {
                                target: target_uid,
                                amount: cached_damage,
                                attacker: i as i32,
                                attacker_faction: faction_id,
                            });
                        }
                    }
                    attacks += 1;
                    if let Ok(mut t) = aq.timer_q.get_mut(entity) {
                        t.0 = cached_cooldown;
                    }
                }
            } else if dist <= close_chase_radius {
                intents.submit(
                    entity,
                    inst_pos,
                    MovementPriority::Combat,
                    "combat:chase_building",
                );
                chases += 1;
            }
            continue;
        }

        // ── NPC target ──
        let target_npc = entity_map.get_npc(ti);
        if target_npc.is_none_or(|n| n.dead) {
            if is_fighting {
                if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) {
                    *cs = CombatState::None;
                }
            }
            continue;
        }
        // Use ECS faction as source-of-truth for NPC targets.
        // GPU faction readback is throttled and can be temporarily stale (-1),
        // which would incorrectly suppress valid combat in tests/gameplay.
        let target_faction = target_npc
            .map(|n| n.faction)
            .unwrap_or_else(|| gpu_state.factions.get(ti).copied().unwrap_or(-1));
        if target_faction == crate::constants::FACTION_NEUTRAL || target_faction == faction_id {
            if is_fighting {
                if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) {
                    *cs = CombatState::None;
                }
            }
            continue;
        }
        if ti * 2 + 1 >= positions.len() {
            bounds_failures += 1;
            continue;
        }

        if !is_fighting {
            if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) {
                *cs = CombatState::Fighting {
                    origin: Vec2::new(x, y),
                };
            }
            in_combat_added += 1;
        }
        let (tx, ty) = (positions[ti * 2], positions[ti * 2 + 1]);

        if tx < -9000.0 {
            if let Ok(mut cs) = aq.combat_state_q.get_mut(entity) {
                *cs = CombatState::None;
            }
            continue;
        }

        let dx = tx - x;
        let dy = ty - y;
        let dist = (dx * dx + dy * dy).sqrt();

        if attackers == 1 {
            sample_dist = dist;
        }

        if dist <= cached_range {
            intents.submit(
                entity,
                Vec2::new(x, y),
                MovementPriority::Combat,
                "combat:hold_npc",
            );
            in_range_count += 1;
            let timer = aq.timer_q.get(entity).map(|t| t.0).unwrap_or(0.0);
            if in_range_count == 1 {
                sample_timer = timer;
            }
            if timer <= 0.0 {
                timer_ready_count += 1;
                if !fire_projectile(
                    Vec2::new(x, y), Vec2::new(tx, ty),
                    cached_damage, cached_proj_speed, cached_proj_lifetime,
                    faction_id, i as i32, &mut proj_alloc, &mut proj_updates, &mut sfx_writer,
                ) {
                    if let Some(target_uid) = entity_map.uid_for_slot(ti) {
                        damage_events.write(DamageMsg {
                            target: target_uid,
                            amount: cached_damage,
                            attacker: i as i32,
                            attacker_faction: faction_id,
                        });
                    }
                }
                attacks += 1;
                if let Ok(mut t) = aq.timer_q.get_mut(entity) {
                    t.0 = cached_cooldown;
                }
            }
        } else {
            intents.submit(
                entity,
                Vec2::new(tx, ty),
                MovementPriority::Combat,
                "combat:chase_npc",
            );
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
    debug.sample_combat_target_0 = combat_targets.first().copied().unwrap_or(-99);
    debug.sample_combat_target_1 = combat_targets.get(1).copied().unwrap_or(-99);
    debug.sample_pos_0 = (
        positions.first().copied().unwrap_or(-999.0),
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
    entity_map: Res<crate::resources::EntityMap>,
) {
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
                if let Some(target_uid) = entity_map.uid_for_slot(hit_idx as usize) {
                    damage_events.write(DamageMsg {
                        target: target_uid,
                        amount: damage,
                        attacker: shooter,
                        attacker_faction,
                    });
                }
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
    mut sfx_writer: MessageWriter<crate::resources::PlaySfxMsg>,
    mut building_health: Query<&mut Health, (With<Building>, Without<Dead>)>,
    tower_bld_q: Query<&crate::components::TowerBuildingState, With<Building>>,
) {
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
        if i >= tower.town.attack_enabled.len() || !tower.town.attack_enabled[i] {
            continue;
        }

        // Decrement cooldown
        if i < tower.town.timers.len() && tower.town.timers[i] > 0.0 {
            tower.town.timers[i] = (tower.town.timers[i] - dt).max(0.0);
            if tower.town.timers[i] > 0.0 {
                continue;
            }
        }

        let stats = resolve_town_tower_stats(&upgrades.town_levels(i));
        let pos = town.center;
        let faction = town.faction;

        // Look up tower building slot via EntityMap (one Fountain per town)
        debug_assert!(
            entity_map
                .iter_kind_for_town(BuildingKind::Fountain, i as u32)
                .count()
                <= 1,
            "multiple fountains in town {i}"
        );
        let bld_slot = entity_map
            .iter_kind_for_town(BuildingKind::Fountain, i as u32)
            .next()
            .map(|inst| inst.slot);
        let Some(bld_slot) = bld_slot else { continue };

        // Read GPU combat_targets for this tower's entity index (unified slot)
        let target = gpu_state
            .combat_targets
            .get(bld_slot)
            .copied()
            .unwrap_or(-1);
        if target < 0 || entity_map.get_instance(target as usize).is_some() {
            continue;
        } // only target NPCs
        let ti = target as usize;
        if !entity_map.get_npc(ti).is_some_and(|n| !n.dead && n.faction != faction) {
            continue;
        }

        let tx = gpu_state.positions.get(ti * 2).copied().unwrap_or(-9999.0);
        let ty = gpu_state
            .positions
            .get(ti * 2 + 1)
            .copied()
            .unwrap_or(-9999.0);
        if tx < -9000.0 {
            continue;
        }

        let dx = tx - pos.x;
        let dy = ty - pos.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist > stats.range {
            continue;
        }

        if fire_projectile(
            pos, Vec2::new(tx, ty),
            stats.damage, stats.proj_speed, stats.proj_lifetime,
            faction, bld_slot as i32, &mut proj_alloc, &mut proj_updates, &mut sfx_writer,
        ) {
            if i < tower.town.timers.len() {
                tower.town.timers[i] = stats.cooldown;
            }
        }
    }

    // --- Player/AI-built Towers ---
    tower.tower_cooldowns.retain(|slot, _| {
        entity_map
            .get_instance(*slot)
            .is_some_and(|i| i.kind == BuildingKind::Tower)
    });

    // Collect tower data (slot, position, faction, xp, upgrade_levels) from ECS
    let towers: Vec<_> = entity_map.iter_kind(BuildingKind::Tower)
        .filter_map(|inst| {
            let entity = entity_map.entities.get(&inst.slot)?;
            let tbs = tower_bld_q.get(*entity).ok()?;
            Some((inst.slot, inst.position, inst.faction, tbs.xp, tbs.upgrade_levels.clone()))
        })
        .collect();

    for (slot, src, faction, xp, upgrade_levels) in &towers {
        let timer = tower.tower_cooldowns.entry(*slot).or_insert(0.0);
        if *timer > 0.0 {
            *timer = (*timer - dt).max(0.0);
            if *timer > 0.0 {
                continue;
            }
        }
        let level = crate::systems::stats::level_from_xp(*xp);
        let stats = crate::systems::stats::resolve_tower_instance_stats(level, upgrade_levels);

        let target = gpu_state
            .combat_targets
            .get(*slot)
            .copied()
            .unwrap_or(-1);
        if target < 0 || entity_map.get_instance(target as usize).is_some() {
            continue;
        }
        let ti = target as usize;
        if !entity_map.get_npc(ti).is_some_and(|n| !n.dead && n.faction != *faction) {
            continue;
        }
        let tx = gpu_state.positions.get(ti * 2).copied().unwrap_or(-9999.0);
        let ty = gpu_state
            .positions
            .get(ti * 2 + 1)
            .copied()
            .unwrap_or(-9999.0);
        if tx < -9000.0 {
            continue;
        }
        let target_pos = Vec2::new(tx, ty);
        if src.distance(target_pos) > stats.range {
            continue;
        }
        if fire_projectile(
            *src, target_pos,
            stats.damage, stats.proj_speed, stats.proj_lifetime,
            *faction, *slot as i32, &mut proj_alloc, &mut proj_updates, &mut sfx_writer,
        ) {
            *timer = stats.cooldown;
        }
    }

    // Tower HP regen: heal towers with hp_regen upgrade
    for (slot, _, _, xp, upgrade_levels) in &towers {
        let level = crate::systems::stats::level_from_xp(*xp);
        let stats = crate::systems::stats::resolve_tower_instance_stats(level, upgrade_levels);
        if stats.hp_regen > 0.0 {
            if let Some(&entity) = entity_map.entities.get(slot) {
                if let Ok(mut health) = building_health.get_mut(entity) {
                    health.0 = (health.0 + stats.hp_regen * dt).min(stats.max_hp);
                }
            }
        }
    }
}

/// Populate BuildingHpRender from building entity Health (only damaged buildings).
/// Gated behind `BuildingHealState.needs_healing` — skips full query when no buildings are damaged.
pub fn sync_building_hp_render(
    query: Query<(&Building, &GpuSlot, &Health), Without<Dead>>,
    gpu_state: Res<crate::gpu::EntityGpuState>,
    mut render: ResMut<crate::resources::BuildingHpRender>,
    heal_state: Res<crate::resources::BuildingHealState>,
) {
    render.positions.clear();
    render.health_pcts.clear();
    if !heal_state.needs_healing {
        return;
    }
    let positions = &gpu_state.positions;
    for (building, npc_idx, health) in query.iter() {
        let max_hp = crate::constants::building_def(building.kind).hp;
        if health.0 <= 0.0 || health.0 >= max_hp {
            continue;
        }
        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() {
            continue;
        }
        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        render.positions.push(Vec2::new(x, y));
        render.health_pcts.push(health.0 / max_hp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{AttackTimer, Building, Dead, GpuSlot};
    use crate::resources::{CombatDebug, GameTime};
    use bevy::time::TimeUpdateStrategy;

    fn setup_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(CombatDebug::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, cooldown_system);
        app.update();
        app.update();
        app
    }

    fn spawn_attacker(app: &mut App, timer: f32) -> Entity {
        app.world_mut().spawn((
            GpuSlot(0),
            AttackTimer(timer),
        )).id()
    }

    #[test]
    fn cooldown_decrements_timer() {
        let mut app = setup_app();
        let npc = spawn_attacker(&mut app, 2.0);

        app.update();
        let timer = app.world().get::<AttackTimer>(npc).unwrap().0;
        assert!(timer < 2.0, "timer should decrement: {timer}");
    }

    #[test]
    fn cooldown_floors_at_zero() {
        let mut app = setup_app();
        let npc = spawn_attacker(&mut app, 0.01);

        for _ in 0..100 {
            app.update();
        }
        let timer = app.world().get::<AttackTimer>(npc).unwrap().0;
        assert!(timer >= 0.0, "timer should never go negative: {timer}");
        assert!(timer < f32::EPSILON, "timer should be at zero: {timer}");
    }

    #[test]
    fn cooldown_zero_timer_unchanged() {
        let mut app = setup_app();
        let npc = spawn_attacker(&mut app, 0.0);

        app.update();
        let timer = app.world().get::<AttackTimer>(npc).unwrap().0;
        assert!(timer.abs() < f32::EPSILON, "zero timer should stay zero: {timer}");
    }

    #[test]
    fn cooldown_dead_excluded() {
        let mut app = setup_app();
        let npc = app.world_mut().spawn((
            GpuSlot(0),
            AttackTimer(5.0),
            Dead,
        )).id();

        app.update();
        let timer = app.world().get::<AttackTimer>(npc).unwrap().0;
        assert!((timer - 5.0).abs() < f32::EPSILON, "dead entity timer should not change: {timer}");
    }

    #[test]
    fn cooldown_buildings_excluded() {
        let mut app = setup_app();
        let building = app.world_mut().spawn((
            GpuSlot(0),
            AttackTimer(5.0),
            Building { kind: crate::world::BuildingKind::Tower },
        )).id();

        app.update();
        let timer = app.world().get::<AttackTimer>(building).unwrap().0;
        assert!((timer - 5.0).abs() < f32::EPSILON, "building timer should not change: {timer}");
    }

    #[test]
    fn cooldown_updates_debug() {
        let mut app = setup_app();
        spawn_attacker(&mut app, 3.0);

        app.update();
        let debug = app.world().resource::<CombatDebug>();
        assert_eq!(debug.cooldown_entities, 1, "should count 1 entity");
        assert!(debug.frame_delta > 0.0, "frame_delta should be positive: {}", debug.frame_delta);
        // sample_timer captures pre-decrement value on first tick, but FixedUpdate
        // runs many sub-ticks per app.update(), so later ticks see partially decremented values
        assert!(debug.sample_timer > 0.0 && debug.sample_timer <= 3.0,
                "sample_timer should reflect timer value: {}", debug.sample_timer);
    }

    // -- sync_building_hp_render ---------------------------------------------

    fn setup_hp_render_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::gpu::EntityGpuState::default());
        app.insert_resource(crate::resources::BuildingHpRender::default());
        app.insert_resource(crate::resources::BuildingHealState { needs_healing: true });
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, sync_building_hp_render);
        app.update();
        app.update();
        app
    }

    #[test]
    fn hp_render_shows_damaged_building() {
        let mut app = setup_hp_render_app();
        // Spawn a building at slot 0 with 50% HP
        // Fountain has hp=500 (from building_def)
        app.world_mut().spawn((
            Building { kind: crate::world::BuildingKind::Fountain },
            GpuSlot(0),
            crate::components::Health(250.0),
        ));
        // Fill GPU positions
        app.world_mut().resource_mut::<crate::gpu::EntityGpuState>().positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert_eq!(render.positions.len(), 1, "should have 1 damaged building");
        assert!((render.positions[0].x - 100.0).abs() < 0.1);
        assert!(render.health_pcts[0] > 0.0 && render.health_pcts[0] < 1.0,
                "health pct should be between 0 and 1: {}", render.health_pcts[0]);
    }

    #[test]
    fn hp_render_skips_full_hp_building() {
        let mut app = setup_hp_render_app();
        let max_hp = crate::constants::building_def(crate::world::BuildingKind::Fountain).hp;
        app.world_mut().spawn((
            Building { kind: crate::world::BuildingKind::Fountain },
            GpuSlot(0),
            crate::components::Health(max_hp),
        ));
        app.world_mut().resource_mut::<crate::gpu::EntityGpuState>().positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert!(render.positions.is_empty(), "full HP building should not appear in render");
    }

    #[test]
    fn hp_render_skips_dead_building() {
        let mut app = setup_hp_render_app();
        app.world_mut().spawn((
            Building { kind: crate::world::BuildingKind::Fountain },
            GpuSlot(0),
            crate::components::Health(0.0),
        ));
        app.world_mut().resource_mut::<crate::gpu::EntityGpuState>().positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert!(render.positions.is_empty(), "dead building (hp=0) should not appear in render");
    }

    #[test]
    fn hp_render_skips_when_no_healing_needed() {
        let mut app = setup_hp_render_app();
        app.world_mut().resource_mut::<crate::resources::BuildingHealState>().needs_healing = false;
        app.world_mut().spawn((
            Building { kind: crate::world::BuildingKind::Fountain },
            GpuSlot(0),
            crate::components::Health(250.0),
        ));
        app.world_mut().resource_mut::<crate::gpu::EntityGpuState>().positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert!(render.positions.is_empty(), "should skip query when needs_healing is false");
    }
}
