//! Combat systems - Attack processing using GPU targeting results

use crate::components::*;
use crate::gpu::ProjBufferWrites;
use crate::messages::{DamageMsg, ProjGpuUpdate, ProjGpuUpdateMsg};
use crate::resources::{
    CombatDebug, EntityMap, GameTime, GpuReadState, MovementPriority, PathRequestQueue,
    ProjHitState, ProjSlotAllocator, TowerState,
};
use crate::systems::stats::{UPGRADES, resolve_town_tower_stats};
use crate::world::{Biome, BuildingKind, WorldData, WorldGrid, is_alive};
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
    homing_target: i32,
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
            homing_target,
        }));
        sfx_writer.write(crate::resources::PlaySfxMsg {
            kind: crate::resources::SfxKind::ArrowShoot,
            position: Some(src),
        });
        return true;
    }
    false
}

pub(crate) fn fire_loot_fly(
    src: Vec2,
    killer_pos: Vec2,
    target_slot: usize,
    proj_alloc: &mut ProjSlotAllocator,
    proj_updates: &mut MessageWriter<ProjGpuUpdateMsg>,
) {
    let delta = killer_pos - src;
    let dist = delta.length().max(1.0);
    let dir = delta / dist;
    let speed = 300.0;
    if let Some(proj_slot) = proj_alloc.alloc() {
        proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Spawn {
            idx: proj_slot,
            x: src.x,
            y: src.y,
            vx: dir.x * speed,
            vy: dir.y * speed,
            damage: 0.0,
            faction: -1,
            shooter: -1,
            lifetime: 1.5,
            homing_target: target_slot as i32,
        }));
    }
}

/// Snapshot of a living NPC used for CPU-side target selection.
/// Built once per `attack_system` run to avoid repeated ECS lookups.
#[derive(Clone)]
pub(crate) struct TargetCandidate {
    pub slot: usize,
    pub x: f32,
    pub y: f32,
    /// True when the NPC is retreating (ReturnLoot activity heading home).
    /// Attackers prefer non-retreating enemies when alternatives are in range.
    pub is_retreating: bool,
    /// Absolute current health. Lower = higher kill priority (secondary factor).
    pub health: f32,
    pub faction: i32,
}

/// Select the best NPC target for an attacker from a pre-built candidate list.
///
/// Priority (lower score wins):
///   1. Non-retreating enemy in range, lowest HP first
///   2. Retreating enemy in range (fallback when no non-retreating in range)
///
/// Returns `gpu_candidate` unchanged when the GPU candidate is already the best
/// or when no candidates vec entry covers it (fast path for non-retreating targets).
pub(crate) fn pick_npc_target(
    gpu_candidate: usize,
    attacker_pos: Vec2,
    range: f32,
    attacker_faction: i32,
    candidates: &[TargetCandidate],
) -> usize {
    // Score: (retreating: u8, health: f32); lower tuple = higher priority.
    let score_of = |c: &TargetCandidate| -> (u8, f32) { (c.is_retreating as u8, c.health) };

    let gpu_entry = candidates.iter().find(|c| c.slot == gpu_candidate);
    let gpu_score = gpu_entry.map(score_of).unwrap_or((0, f32::MAX));

    // Fast path: GPU candidate is non-retreating -- use it immediately.
    if gpu_score.0 == 0 {
        return gpu_candidate;
    }

    // GPU candidate is retreating; scan for any better in-range target.
    let range_sq = range * range;
    let mut best_slot = gpu_candidate;
    let mut best_score = gpu_score;

    for c in candidates {
        if c.faction == attacker_faction
            || c.faction == crate::constants::FACTION_NEUTRAL
            || c.x < -9000.0
        {
            continue;
        }
        let dx = c.x - attacker_pos.x;
        let dy = c.y - attacker_pos.y;
        if dx * dx + dy * dy > range_sq {
            continue;
        }
        let s = score_of(c);
        if s < best_score {
            best_score = s;
            best_slot = c.slot;
        }
    }

    best_slot
}

/// ECS queries for attack_system (bundled to stay under 16-param limit).
#[derive(bevy::ecs::system::SystemParam)]
pub struct AttackQueries<'w, 's> {
    pub combat_state_q: Query<'w, 's, &'static mut CombatState>,
    pub timer_q: Query<'w, 's, &'static mut AttackTimer>,
    /// Read-only snapshot query used to build the `TargetCandidate` list.
    target_q: Query<
        'w,
        's,
        (
            &'static GpuSlot,
            &'static Activity,
            &'static Health,
            &'static Faction,
        ),
        (Without<Building>, Without<Dead>),
    >,
    /// Read town upgrade levels for target switching gate.
    town_upgrades_q: Query<'w, 's, &'static TownUpgradeLevel>,
    town_index: Res<'w, crate::resources::TownIndex>,
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
    grid: Res<WorldGrid>,
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

    // Build a per-frame candidate list (O(n) once) for CPU-side target switching.
    // Only used by already-fighting NPCs whose GPU candidate is retreating.
    let target_candidates: Vec<TargetCandidate> = aq
        .target_q
        .iter()
        .filter_map(|(slot, activity, health, faction)| {
            let s = slot.0;
            if s * 2 + 1 >= positions.len() {
                return None;
            }
            let px = positions[s * 2];
            if px < -9000.0 {
                return None;
            }
            Some(TargetCandidate {
                slot: s,
                x: px,
                y: positions[s * 2 + 1],
                is_retreating: activity.kind == ActivityKind::ReturnLoot,
                health: health.0,
                faction: faction.0,
            })
        })
        .collect();

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
        let cached_damage = if stats.berserk_bonus != 0.0
            && stats.max_health > 0.0
            && health.0 / stats.max_health < 0.5
        {
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
                    if curr_inst.faction != crate::constants::FACTION_NEUTRAL
                        && curr_inst.faction != faction_id
                    {
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

        // Rock high ground: +20% attack range for NPCs standing on Rock tiles
        let effective_range = if grid.width > 0 {
            let (gc, gr) = grid.world_to_grid(Vec2::new(x, y));
            if grid.cell(gc, gr).is_some_and(|c| c.terrain == Biome::Rock) {
                cached_range * 1.2
            } else {
                cached_range
            }
        } else {
            cached_range
        };

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

            let close_chase_radius = effective_range + 120.0;
            if dist <= effective_range {
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
                        Vec2::new(x, y),
                        inst_pos,
                        cached_damage,
                        cached_proj_speed,
                        cached_proj_lifetime,
                        faction_id,
                        i as i32,
                        -1,
                        &mut proj_alloc,
                        &mut proj_updates,
                        &mut sfx_writer,
                    ) {
                        if let Some(target_entity) = entity_map.entities.get(&ti).copied() {
                            damage_events.write(DamageMsg {
                                target: target_entity,
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

        // CPU-side target switching for already-fighting NPCs (gated by upgrade):
        // prefer non-retreating enemies, break ties by lower HP (finish wounded faster).
        // Scan range scales with TargetSwitching upgrade level (+20% weapon range per level).
        let ti = if is_fighting {
            let reg = &*UPGRADES;
            let cat = crate::constants::npc_def(job)
                .upgrade_category
                .unwrap_or("");
            let town_levels = aq
                .town_index
                .0
                .get(&faction_id)
                .and_then(|e| aq.town_upgrades_q.get(*e).ok())
                .map(|u| u.0.as_slice())
                .unwrap_or(&[]);
            let tgt_mult = reg.stat_mult(
                town_levels,
                cat,
                crate::constants::UpgradeStatKind::TargetSwitching,
            );
            if tgt_mult > 1.0 {
                let scan_range = cached_range * tgt_mult;
                pick_npc_target(
                    ti,
                    Vec2::new(x, y),
                    scan_range,
                    faction_id,
                    &target_candidates,
                )
            } else {
                ti
            }
        } else {
            ti
        };
        // Re-validate bounds after potential target switch.
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

        if dist <= effective_range {
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
                    Vec2::new(x, y),
                    Vec2::new(tx, ty),
                    cached_damage,
                    cached_proj_speed,
                    cached_proj_lifetime,
                    faction_id,
                    i as i32,
                    -1,
                    &mut proj_alloc,
                    &mut proj_updates,
                    &mut sfx_writer,
                ) {
                    if let Some(&target_entity) = entity_map.entities.get(&ti) {
                        damage_events.write(DamageMsg {
                            target: target_entity,
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
    gpu_state: Res<GpuReadState>,
    grid: Res<WorldGrid>,
    mut skills_q: Query<&mut NpcSkills>,
    town_access: crate::systemparams::TownAccess,
) {
    use rand::Rng;
    let mut rng = rand::rng();

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
                // Forest cover: 25% miss chance for projectile hits on targets in Forest cells
                let ti = hit_idx as usize;
                let tx = gpu_state.positions.get(ti * 2).copied().unwrap_or(0.0);
                let ty = gpu_state.positions.get(ti * 2 + 1).copied().unwrap_or(0.0);
                if grid.width > 0 {
                    let (gc, gr) = grid.world_to_grid(Vec2::new(tx, ty));
                    if grid
                        .cell(gc, gr)
                        .is_some_and(|c| c.terrain == Biome::Forest)
                        && rng.random_range(0.0..1.0_f32) < 0.25
                    {
                        // Miss -- projectile absorbed by forest cover
                        proj_alloc.free(slot);
                        proj_updates
                            .write(ProjGpuUpdateMsg(ProjGpuUpdate::Deactivate { idx: slot }));
                        continue;
                    }
                }

                // Dodge proficiency: personal miss chance based on target's dodge skill.
                // Uses same proficiency_mult as combat/farming: dodge_chance = 1 - 1/mult.
                // At prof 0: 0%, 100: 50%, 1000: 91%, SOFT_CAP: 99%.
                if let Some(npc) = entity_map.get_npc(ti) {
                    let levels = town_access.upgrade_levels(npc.town_idx);
                    if crate::systems::stats::dodge_unlocked(&levels) {
                        if let Ok(mut skills) = skills_q.get_mut(npc.entity) {
                            let mult = crate::systems::stats::proficiency_mult(skills.dodge);
                            let dodge_chance = 1.0 - 1.0 / mult;
                            if rng.random_range(0.0..1.0_f32) < dodge_chance {
                                // Dodged -- grant dodge proficiency
                                skills.dodge = (skills.dodge + crate::constants::DODGE_SKILL_RATE)
                                    .min(crate::constants::MAX_PROFICIENCY);
                                proj_alloc.free(slot);
                                proj_updates.write(ProjGpuUpdateMsg(ProjGpuUpdate::Deactivate {
                                    idx: slot,
                                }));
                                continue;
                            }
                        }
                    }
                }

                let shooter = if slot < proj_writes.shooters.len() {
                    proj_writes.shooters[slot]
                } else {
                    -1
                };
                let attacker_faction = proj_writes.factions.get(slot).copied().unwrap_or(-1);
                if let Some(&target_entity) = entity_map.entities.get(&(hit_idx as usize)) {
                    damage_events.write(DamageMsg {
                        target: target_entity,
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
    town_access: crate::systemparams::TownAccess,
    entity_map: Res<EntityMap>,
    mut tower: ResMut<TowerState>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    mut proj_updates: MessageWriter<ProjGpuUpdateMsg>,
    mut sfx_writer: MessageWriter<crate::resources::PlaySfxMsg>,
    mut building_health: Query<&mut Health, (With<Building>, Without<Dead>)>,
    tower_bld_q: Query<&crate::components::TowerBuildingState, With<Building>>,
) {
    let dt = game_time.delta(&time);
    // --- Towns: sync state, refresh enabled for non-raider towns (fountain) every tick ---
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

        let stats = resolve_town_tower_stats(&town_access.upgrade_levels(i as i32));
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
        if !entity_map
            .get_npc(ti)
            .is_some_and(|n| !n.dead && n.faction != faction)
        {
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
            pos,
            Vec2::new(tx, ty),
            stats.damage,
            stats.proj_speed,
            stats.proj_lifetime,
            faction,
            bld_slot as i32,
            -1,
            &mut proj_alloc,
            &mut proj_updates,
            &mut sfx_writer,
        ) {
            if i < tower.town.timers.len() {
                tower.town.timers[i] = stats.cooldown;
            }
        }
    }

    // --- Player/AI-built Towers ---
    tower.tower_cooldowns.retain(|slot, _| {
        entity_map.get_instance(*slot).is_some_and(|i| {
            let def = crate::constants::building_def(i.kind);
            def.is_tower && i.kind != BuildingKind::Fountain
        })
    });

    // Collect tower data with stack-allocated upgrade levels (no heap clone)
    const MAX_UPGRADES: usize = crate::constants::TOWER_UPGRADES.len();
    let towers: Vec<(
        usize,
        Vec2,
        i32,
        i32,
        [u8; MAX_UPGRADES],
        usize,
        Entity,
        BuildingKind,
        f32,
    )> = entity_map
        .iter_instances()
        .filter(|i| {
            let def = crate::constants::building_def(i.kind);
            def.is_tower && i.kind != BuildingKind::Fountain
        })
        .filter_map(|inst| {
            let entity = *entity_map.entities.get(&inst.slot)?;
            let tbs = tower_bld_q.get(entity).ok()?;
            let mut levels = [0u8; MAX_UPGRADES];
            let len = tbs.upgrade_levels.len().min(MAX_UPGRADES);
            levels[..len].copy_from_slice(&tbs.upgrade_levels[..len]);
            let equip_bonus = tbs
                .equipped_weapon
                .as_ref()
                .map(|w| w.stat_bonus)
                .unwrap_or(0.0);
            Some((
                inst.slot,
                inst.position,
                inst.faction,
                tbs.xp,
                levels,
                len,
                entity,
                inst.kind,
                equip_bonus,
            ))
        })
        .collect();

    for &(slot, src, faction, xp, ref levels, levels_len, entity, kind, equip_bonus) in &towers {
        // tower building kinds always have tower_stats in BUILDING_REGISTRY
        let Some(base) = crate::constants::building_def(kind).tower_stats else {
            continue;
        };
        let level = crate::systems::stats::level_from_xp(xp);
        let mut stats = crate::systems::stats::resolve_tower_instance_stats(
            &base,
            level,
            &levels[..levels_len],
        );
        if equip_bonus > 0.0 {
            stats.damage *= 1.0 + equip_bonus;
        }

        // HP regen (runs unconditionally, before cooldown/target checks)
        if stats.hp_regen > 0.0 {
            if let Ok(mut health) = building_health.get_mut(entity) {
                health.0 = (health.0 + stats.hp_regen * dt).min(stats.max_hp);
            }
        }

        // Combat cooldown
        let timer = tower.tower_cooldowns.entry(slot).or_insert(0.0);
        if *timer > 0.0 {
            *timer = (*timer - dt).max(0.0);
            if *timer > 0.0 {
                continue;
            }
        }

        let target = gpu_state.combat_targets.get(slot).copied().unwrap_or(-1);
        if target < 0 || entity_map.get_instance(target as usize).is_some() {
            continue;
        }
        let ti = target as usize;
        if !entity_map
            .get_npc(ti)
            .is_some_and(|n| !n.dead && n.faction != faction)
        {
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
            src,
            target_pos,
            stats.damage,
            stats.proj_speed,
            stats.proj_lifetime,
            faction,
            slot as i32,
            -1,
            &mut proj_alloc,
            &mut proj_updates,
            &mut sfx_writer,
        ) {
            *timer = stats.cooldown;
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
    use crate::gpu::populate_proj_buffer_writes;
    use crate::messages::{ProjGpuUpdate, ProjGpuUpdateMsg};
    use crate::resources::{
        CombatDebug, EntityMap, GameTime, PlaySfxMsg, ProjHitState, ProjSlotAllocator, TowerState,
    };
    use bevy::ecs::system::RunSystemOnce;
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
        app.update();
        app
    }

    fn spawn_attacker(app: &mut App, timer: f32) -> Entity {
        app.world_mut().spawn((GpuSlot(0), AttackTimer(timer))).id()
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
        assert!(
            timer.abs() < f32::EPSILON,
            "zero timer should stay zero: {timer}"
        );
    }

    #[test]
    fn cooldown_dead_excluded() {
        let mut app = setup_app();
        let npc = app
            .world_mut()
            .spawn((GpuSlot(0), AttackTimer(5.0), Dead))
            .id();

        app.update();
        let timer = app.world().get::<AttackTimer>(npc).unwrap().0;
        assert!(
            (timer - 5.0).abs() < f32::EPSILON,
            "dead entity timer should not change: {timer}"
        );
    }

    #[test]
    fn cooldown_buildings_excluded() {
        let mut app = setup_app();
        let building = app
            .world_mut()
            .spawn((
                GpuSlot(0),
                AttackTimer(5.0),
                Building {
                    kind: crate::world::BuildingKind::BowTower,
                },
            ))
            .id();

        app.update();
        let timer = app.world().get::<AttackTimer>(building).unwrap().0;
        assert!(
            (timer - 5.0).abs() < f32::EPSILON,
            "building timer should not change: {timer}"
        );
    }

    #[test]
    fn cooldown_updates_debug() {
        let mut app = setup_app();
        spawn_attacker(&mut app, 3.0);

        app.update();
        let debug = app.world().resource::<CombatDebug>();
        assert_eq!(debug.cooldown_entities, 1, "should count 1 entity");
        assert!(
            debug.frame_delta > 0.0,
            "frame_delta should be positive: {}",
            debug.frame_delta
        );
        // sample_timer captures pre-decrement value on first tick, but FixedUpdate
        // runs many sub-ticks per app.update(), so later ticks see partially decremented values
        assert!(
            debug.sample_timer > 0.0 && debug.sample_timer <= 3.0,
            "sample_timer should reflect timer value: {}",
            debug.sample_timer
        );
    }

    // -- sync_building_hp_render ---------------------------------------------

    fn setup_hp_render_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(crate::gpu::EntityGpuState::default());
        app.insert_resource(crate::resources::BuildingHpRender::default());
        app.insert_resource(crate::resources::BuildingHealState {
            needs_healing: true,
        });
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, sync_building_hp_render);
        app.update();
        app.update();
        app
    }

    fn setup_building_tower_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ProjGpuUpdateMsg>();
        app.add_message::<PlaySfxMsg>();
        app.insert_resource(GameTime::default());
        app.insert_resource(crate::resources::GpuReadState::default());
        app.insert_resource(crate::world::WorldData::default());
        app.insert_resource(EntityMap::default());
        app.insert_resource(ProjSlotAllocator::default());
        app.insert_resource(TowerState::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.init_resource::<crate::resources::TownIndex>();

        let town = app
            .world_mut()
            .spawn((
                crate::components::TownMarker,
                crate::components::FoodStore(0),
                crate::components::GoldStore(0),
                crate::components::WoodStore(0),
                crate::components::StoneStore(0),
                crate::components::TownPolicy::default(),
                crate::components::TownUpgradeLevel::default(),
                crate::components::TownEquipment::default(),
                crate::components::TownAreaLevel::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<crate::resources::TownIndex>()
            .0
            .insert(0, town);
        app.world_mut()
            .resource_mut::<crate::world::WorldData>()
            .towns
            .push(crate::world::Town {
                name: "BenchTown".into(),
                center: Vec2::new(128.0, 128.0),
                faction: 1,
                kind: crate::constants::TownKind::Player,
            });

        app
    }

    #[test]
    fn hp_render_shows_damaged_building() {
        let mut app = setup_hp_render_app();
        // Spawn a building at slot 0 with 50% HP
        // Fountain has hp=500 (from building_def)
        app.world_mut().spawn((
            Building {
                kind: crate::world::BuildingKind::Fountain,
            },
            GpuSlot(0),
            crate::components::Health(250.0),
        ));
        // Fill GPU positions
        app.world_mut()
            .resource_mut::<crate::gpu::EntityGpuState>()
            .positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert_eq!(render.positions.len(), 1, "should have 1 damaged building");
        assert!((render.positions[0].x - 100.0).abs() < 0.1);
        assert!(
            render.health_pcts[0] > 0.0 && render.health_pcts[0] < 1.0,
            "health pct should be between 0 and 1: {}",
            render.health_pcts[0]
        );
    }

    #[test]
    fn hp_render_skips_full_hp_building() {
        let mut app = setup_hp_render_app();
        let max_hp = crate::constants::building_def(crate::world::BuildingKind::Fountain).hp;
        app.world_mut().spawn((
            Building {
                kind: crate::world::BuildingKind::Fountain,
            },
            GpuSlot(0),
            crate::components::Health(max_hp),
        ));
        app.world_mut()
            .resource_mut::<crate::gpu::EntityGpuState>()
            .positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert!(
            render.positions.is_empty(),
            "full HP building should not appear in render"
        );
    }

    #[test]
    fn hp_render_skips_dead_building() {
        let mut app = setup_hp_render_app();
        app.world_mut().spawn((
            Building {
                kind: crate::world::BuildingKind::Fountain,
            },
            GpuSlot(0),
            crate::components::Health(0.0),
        ));
        app.world_mut()
            .resource_mut::<crate::gpu::EntityGpuState>()
            .positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert!(
            render.positions.is_empty(),
            "dead building (hp=0) should not appear in render"
        );
    }

    #[test]
    fn hp_render_skips_when_no_healing_needed() {
        let mut app = setup_hp_render_app();
        app.world_mut()
            .resource_mut::<crate::resources::BuildingHealState>()
            .needs_healing = false;
        app.world_mut().spawn((
            Building {
                kind: crate::world::BuildingKind::Fountain,
            },
            GpuSlot(0),
            crate::components::Health(250.0),
        ));
        app.world_mut()
            .resource_mut::<crate::gpu::EntityGpuState>()
            .positions = vec![100.0, 200.0];
        app.update();
        let render = app.world().resource::<crate::resources::BuildingHpRender>();
        assert!(
            render.positions.is_empty(),
            "should skip query when needs_healing is false"
        );
    }

    #[test]
    fn building_tower_system_uses_tower_state_to_heal_and_fire() {
        let mut app = setup_building_tower_app();
        let tower_upgrades = crate::constants::TOWER_UPGRADES.len();
        let mut upgrade_levels = vec![0; tower_upgrades];
        upgrade_levels[0] = 1;
        upgrade_levels[tower_upgrades - 1] = 1;

        let enemy = app
            .world_mut()
            .spawn((GpuSlot(0), crate::components::Faction(2)))
            .id();
        let tower = app
            .world_mut()
            .spawn((
                GpuSlot(1),
                crate::components::Health(900.0),
                Building {
                    kind: crate::world::BuildingKind::BowTower,
                },
                crate::components::TowerBuildingState {
                    kills: 0,
                    xp: 100,
                    upgrade_levels: upgrade_levels.clone(),
                    auto_upgrade_flags: vec![false; tower_upgrades],
                    equipped_weapon: None,
                },
            ))
            .id();

        {
            let mut entity_map = app.world_mut().resource_mut::<EntityMap>();
            entity_map.register_npc(0, enemy, crate::components::Job::Archer, 2, 0);
            entity_map.set_entity(1, tower);
            entity_map.add_instance(crate::resources::BuildingInstance {
                kind: crate::world::BuildingKind::BowTower,
                position: Vec2::new(96.0, 64.0),
                town_idx: 0,
                slot: 1,
                faction: 1,
            });
        }
        {
            let mut gpu = app
                .world_mut()
                .resource_mut::<crate::resources::GpuReadState>();
            gpu.positions = vec![64.0, 64.0, 96.0, 64.0];
            gpu.combat_targets = vec![-1, 0];
            gpu.factions = vec![2, 1];
            gpu.health = vec![1.0, 1.0];
            gpu.threat_counts = vec![0, 0];
            gpu.npc_count = 1;
        }

        app.world_mut()
            .resource_mut::<Time>()
            .advance_by(std::time::Duration::from_secs_f32(1.0));
        let dt = app
            .world_mut()
            .run_system_once(|time: Res<Time>| time.delta_secs())
            .unwrap();
        assert!(
            dt > 0.0,
            "tower test should advance Time before the system runs"
        );
        app.world_mut()
            .run_system_once(building_tower_system)
            .unwrap();

        let cooldown = app
            .world()
            .resource::<TowerState>()
            .tower_cooldowns
            .get(&1)
            .copied()
            .unwrap_or_default();
        assert!(cooldown > 0.0, "tower should enter cooldown after firing");

        let shots = app
            .world_mut()
            .run_system_once(|mut reader: MessageReader<ProjGpuUpdateMsg>| {
                reader
                    .read()
                    .filter_map(|msg| match &msg.0 {
                        ProjGpuUpdate::Spawn {
                            shooter,
                            damage,
                            homing_target,
                            ..
                        } => Some((*shooter, *damage, *homing_target)),
                        ProjGpuUpdate::Deactivate { .. } => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap();
        assert_eq!(shots.len(), 1, "tower should spawn one projectile");
        assert_eq!(
            shots[0].0, 1,
            "tower projectile should use the tower slot as shooter"
        );
        assert!(shots[0].1 > 0.0, "tower projectile should carry damage");
        assert_eq!(
            shots[0].2, -1,
            "tower shots should not be homing projectiles"
        );

        let sfx_count = app
            .world_mut()
            .run_system_once(|mut reader: MessageReader<PlaySfxMsg>| reader.read().count())
            .unwrap();
        assert_eq!(sfx_count, 1, "tower shot should emit one SFX event");

        let health = app
            .world()
            .get::<crate::components::Health>(tower)
            .expect("tower health");
        assert!(
            health.0 > 900.0,
            "tower should regen when HpRegen is upgraded (dt={dt}, health={})",
            health.0
        );
    }

    #[test]
    fn loot_fly_spawns_homing_zero_damage_projectile_without_sfx() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ProjGpuUpdateMsg>();
        app.add_message::<PlaySfxMsg>();
        app.insert_resource(ProjSlotAllocator::default());
        app.insert_resource(crate::gpu::ProjBufferWrites::default());

        app.world_mut()
            .run_system_once(
                |mut proj_alloc: ResMut<ProjSlotAllocator>,
                 mut proj_updates: MessageWriter<ProjGpuUpdateMsg>| {
                    fire_loot_fly(
                        Vec2::new(10.0, 20.0),
                        Vec2::new(110.0, 20.0),
                        7,
                        &mut proj_alloc,
                        &mut proj_updates,
                    );
                },
            )
            .unwrap();
        app.world_mut()
            .run_system_once(populate_proj_buffer_writes)
            .unwrap();

        let writes = app.world().resource::<crate::gpu::ProjBufferWrites>();
        assert_eq!(writes.active[0], 1);
        assert_eq!(writes.damages[0], 0.0);
        assert_eq!(writes.factions[0], -1);
        assert_eq!(writes.shooters[0], -1);
        assert_eq!(writes.homing_targets[0], 7);
        assert!(
            writes.velocities[0] > 0.0 && writes.velocities[1].abs() < 0.01,
            "loot fly should start moving toward the killer"
        );

        let sfx_count = app
            .world_mut()
            .run_system_once(|mut reader: MessageReader<PlaySfxMsg>| reader.read().count())
            .unwrap();
        assert_eq!(sfx_count, 0, "loot fly should not emit arrow SFX");
    }

    #[test]
    fn zero_damage_projectile_hit_does_not_emit_damage() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<crate::messages::DamageMsg>();
        app.add_message::<ProjGpuUpdateMsg>();
        app.insert_resource(ProjSlotAllocator::default());
        app.insert_resource(crate::gpu::ProjBufferWrites::default());
        app.insert_resource(ProjHitState(vec![[0, 0]]));
        app.insert_resource(crate::resources::EntityMap::default());
        app.insert_resource(crate::resources::GpuReadState::default());
        app.insert_resource(crate::world::WorldGrid::default());
        app.insert_resource(crate::resources::TownIndex::default());

        let slot = app
            .world_mut()
            .resource_mut::<ProjSlotAllocator>()
            .alloc()
            .expect("projectile slot");
        {
            let mut writes = app
                .world_mut()
                .resource_mut::<crate::gpu::ProjBufferWrites>();
            writes.active[slot] = 1;
            writes.damages[slot] = 0.0;
            writes.shooters[slot] = -1;
            writes.factions[slot] = -1;
        }

        app.world_mut().run_system_once(process_proj_hits).unwrap();

        let damage_count = app
            .world_mut()
            .run_system_once(|mut reader: MessageReader<crate::messages::DamageMsg>| {
                reader.read().count()
            })
            .unwrap();
        let proj_update_count = app
            .world_mut()
            .run_system_once(|mut reader: MessageReader<ProjGpuUpdateMsg>| reader.read().count())
            .unwrap();

        assert_eq!(damage_count, 0, "zero-damage projectile should not damage");
        assert_eq!(
            proj_update_count, 1,
            "hit should still recycle the projectile slot"
        );
    }

    // -- pick_npc_target: target switching -----------------------------------

    fn make_candidate(
        slot: usize,
        x: f32,
        y: f32,
        is_retreating: bool,
        health: f32,
        faction: i32,
    ) -> TargetCandidate {
        TargetCandidate {
            slot,
            x,
            y,
            is_retreating,
            health,
            faction,
        }
    }

    const ENEMY_FACTION: i32 = 2;
    const ATTACKER_FACTION: i32 = 1;
    const RANGE: f32 = 200.0;

    /// Regression: non-retreating GPU candidate is returned immediately (fast path).
    #[test]
    fn target_non_retreating_uses_gpu_candidate() {
        let candidates = vec![make_candidate(5, 0.0, 100.0, false, 80.0, ENEMY_FACTION)];
        let result = pick_npc_target(5, Vec2::ZERO, RANGE, ATTACKER_FACTION, &candidates);
        assert_eq!(result, 5, "non-retreating candidate should be used as-is");
    }

    /// Regression: when GPU candidate is retreating and a non-retreating enemy is in range,
    /// the non-retreating target is preferred over the retreating one.
    #[test]
    fn target_prefers_non_retreating_over_retreating() {
        // Slot 5 = retreating (GPU candidate), slot 7 = non-retreating, both in range.
        let candidates = vec![
            make_candidate(5, 0.0, 50.0, true, 40.0, ENEMY_FACTION), // retreating
            make_candidate(7, 0.0, 80.0, false, 90.0, ENEMY_FACTION), // non-retreating
        ];
        let result = pick_npc_target(5, Vec2::ZERO, RANGE, ATTACKER_FACTION, &candidates);
        assert_eq!(
            result, 7,
            "non-retreating in-range enemy should be preferred over retreating"
        );
    }

    /// Regression: when GPU candidate is retreating and no non-retreating enemy is in range,
    /// the original retreating candidate is used as fallback.
    #[test]
    fn target_falls_back_to_retreating_when_no_alternatives() {
        // Slot 5 = retreating in range, slot 7 = non-retreating but out of range.
        let candidates = vec![
            make_candidate(5, 0.0, 50.0, true, 40.0, ENEMY_FACTION), // retreating, in range
            make_candidate(7, 0.0, 500.0, false, 20.0, ENEMY_FACTION), // non-retreating, far
        ];
        let result = pick_npc_target(5, Vec2::ZERO, RANGE, ATTACKER_FACTION, &candidates);
        assert_eq!(
            result, 5,
            "retreating target should be fallback when no non-retreating in range"
        );
    }

    /// Regression: among multiple non-retreating targets in range, the lowest-HP one is chosen.
    #[test]
    fn target_prefers_lower_hp_among_non_retreating() {
        // GPU candidate = slot 5 (retreating). Two non-retreating options: slot 7 (high HP) and slot 9 (low HP).
        let candidates = vec![
            make_candidate(5, 0.0, 30.0, true, 50.0, ENEMY_FACTION), // retreating (GPU)
            make_candidate(7, 0.0, 80.0, false, 90.0, ENEMY_FACTION), // non-retreating, high HP
            make_candidate(9, 0.0, 100.0, false, 10.0, ENEMY_FACTION), // non-retreating, low HP
        ];
        let result = pick_npc_target(5, Vec2::ZERO, RANGE, ATTACKER_FACTION, &candidates);
        assert_eq!(result, 9, "lowest-HP non-retreating target should win");
    }

    /// Regression: same-faction candidates are never selected.
    #[test]
    fn target_skips_same_faction() {
        // GPU candidate is retreating (slot 5). The only "better" candidate is same faction.
        let candidates = vec![
            make_candidate(5, 0.0, 50.0, true, 80.0, ENEMY_FACTION), // retreating (GPU)
            make_candidate(8, 0.0, 60.0, false, 10.0, ATTACKER_FACTION), // same faction -- skip
        ];
        let result = pick_npc_target(5, Vec2::ZERO, RANGE, ATTACKER_FACTION, &candidates);
        assert_eq!(
            result, 5,
            "same-faction candidate must never be selected as target"
        );
    }

    /// Regression: neutral-faction candidates are skipped.
    #[test]
    fn target_skips_neutral_faction() {
        let candidates = vec![
            make_candidate(5, 0.0, 50.0, true, 80.0, ENEMY_FACTION), // retreating (GPU)
            make_candidate(6, 0.0, 60.0, false, 10.0, crate::constants::FACTION_NEUTRAL), // neutral -- skip
        ];
        let result = pick_npc_target(5, Vec2::ZERO, RANGE, ATTACKER_FACTION, &candidates);
        assert_eq!(
            result, 5,
            "neutral-faction candidate must never be selected"
        );
    }

    // -- tower collection: registry-based filter ----------------------------

    /// Regression: tower combat collection uses registry is_tower flag, not a hardcoded kind list.
    /// Verifies that iter_instances().filter(is_tower) returns exactly the tower-flagged buildings
    /// registered in BUILDING_REGISTRY (excluding Fountain), and excludes non-tower buildings.
    #[test]
    fn tower_collection_uses_registry_is_tower_filter() {
        use crate::constants::{BUILDING_REGISTRY, building_def};
        use crate::resources::BuildingInstance;

        let mut em = EntityMap::default();

        // Register one instance per is_tower kind (excluding Fountain).
        let tower_kinds: Vec<_> = BUILDING_REGISTRY
            .iter()
            .filter(|d| d.is_tower && d.kind != BuildingKind::Fountain)
            .map(|d| d.kind)
            .collect();

        for (slot, &kind) in tower_kinds.iter().enumerate() {
            em.add_instance(BuildingInstance {
                kind,
                position: Vec2::ZERO,
                town_idx: 0,
                slot,
                faction: 1,
            });
        }

        // Also register a non-tower building to verify it is excluded.
        let non_tower_kind = crate::world::BuildingKind::Farm;
        assert!(
            !building_def(non_tower_kind).is_tower,
            "Farm must not be a tower for this test to be valid"
        );
        em.add_instance(BuildingInstance {
            kind: non_tower_kind,
            position: Vec2::ZERO,
            town_idx: 0,
            slot: tower_kinds.len(),
            faction: 1,
        });

        let filtered: Vec<_> = em
            .iter_instances()
            .filter(|i| {
                let def = building_def(i.kind);
                def.is_tower && i.kind != BuildingKind::Fountain
            })
            .collect();

        assert_eq!(
            filtered.len(),
            tower_kinds.len(),
            "registry-based filter must return exactly all is_tower buildings (got {}, expected {})",
            filtered.len(),
            tower_kinds.len()
        );

        for kind in &tower_kinds {
            assert!(
                filtered.iter().any(|i| i.kind == *kind),
                "tower kind {:?} must be included by registry-based filter",
                kind
            );
        }
    }
    // -- Terrain combat modifiers -------------------------------------------

    /// Forest cover: 25% of projectile hits on Forest-tile targets should miss.
    /// Runs 200 simultaneous hits and checks the damage count is within 3 sigma of 75%.
    #[test]
    fn forest_cover_misses_25_percent() {
        use crate::world::{Biome, WorldCell, WorldGrid};
        const N: usize = 200;

        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<crate::messages::DamageMsg>();
        app.add_message::<ProjGpuUpdateMsg>();
        app.insert_resource(crate::resources::TownIndex::default());

        // Build a tiny 1x1 grid with Forest at (0,0)
        let mut grid = WorldGrid::default();
        grid.width = 1;
        grid.height = 1;
        grid.cell_size = 64.0;
        grid.cells = vec![WorldCell {
            terrain: Biome::Forest,
            original_terrain: Biome::Forest,
        }];
        app.insert_resource(grid);

        // Target NPC lives at (32, 32) -- center of forest cell (0,0)
        let target_entity = app.world_mut().spawn(()).id();
        let mut entity_map = crate::resources::EntityMap::default();
        entity_map.register_npc(1, target_entity, crate::components::Job::Archer, 2, 0);
        app.insert_resource(entity_map);

        let mut gpu_state = crate::resources::GpuReadState::default();
        gpu_state.positions = vec![0.0, 0.0, 32.0, 32.0]; // slot 0 unused, slot 1 = target
        app.insert_resource(gpu_state);

        // Allocate N projectile slots and set them all to hit target slot 1 with damage 10.0
        let mut proj_alloc = ProjSlotAllocator::default();
        let mut proj_writes = crate::gpu::ProjBufferWrites::default();
        let mut hit_slots = Vec::new();
        for _ in 0..N {
            let slot = proj_alloc.alloc().expect("slot");
            proj_writes.active[slot] = 1;
            proj_writes.damages[slot] = 10.0;
            proj_writes.shooters[slot] = -1;
            proj_writes.factions[slot] = -1;
            hit_slots.push(slot);
        }
        app.insert_resource(proj_alloc);
        app.insert_resource(proj_writes);

        // ProjHitState: each slot reports hit_idx=1 (target slot), processed=0
        let max_slot = *hit_slots.iter().max().unwrap() + 1;
        let mut hit_state_vec = vec![[0i32, 0i32]; max_slot];
        for &s in &hit_slots {
            hit_state_vec[s] = [1, 0];
        }
        app.insert_resource(ProjHitState(hit_state_vec));

        app.world_mut().run_system_once(process_proj_hits).unwrap();

        let damage_count = app
            .world_mut()
            .run_system_once(|mut reader: MessageReader<crate::messages::DamageMsg>| {
                reader.read().count()
            })
            .unwrap();

        // Expect ~75% hits (150/200). 3-sigma tolerance: sqrt(200*0.75*0.25)*3 = ~18.4
        let hits = damage_count as f32;
        let expected = N as f32 * 0.75;
        let sigma3 = (N as f32 * 0.75 * 0.25_f32).sqrt() * 3.0;
        assert!(
            (hits - expected).abs() < sigma3,
            "forest cover: expected ~{expected} hits, got {hits} (3-sigma tolerance {sigma3:.1})"
        );
        // Verify at least some misses occurred
        assert!(
            damage_count < N,
            "forest cover should cause some misses, got {damage_count}/{N} hits"
        );
    }

    fn setup_attack_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<ProjGpuUpdateMsg>();
        app.add_message::<DamageMsg>();
        app.add_message::<crate::resources::PlaySfxMsg>();
        app.insert_resource(crate::resources::PathRequestQueue::default());
        app.insert_resource(CombatDebug::default());
        app.insert_resource(crate::resources::GpuReadState::default());
        app.insert_resource(crate::gpu::EntityGpuState::default());
        app.insert_resource(crate::resources::EntityMap::default());
        app.insert_resource(ProjSlotAllocator::default());
        app.insert_resource(crate::resources::SquadState::default());
        app.insert_resource(GameTime::default());
        app.insert_resource(crate::world::WorldGrid::default());
        app.insert_resource(crate::resources::TownIndex::default());
        app
    }

    /// Rock high ground: NPC on Rock tile can attack at 1.1x range (within +20% bonus).
    /// Reverted (Grass tile), same NPC cannot reach the same target.
    #[test]
    fn rock_high_ground_extends_attack_range() {
        use crate::components::{Activity, CachedStats, CombatState, Faction, Health, Job};
        use crate::world::{Biome, WorldCell, WorldGrid};

        // Attacker at (32,32), target at (252, 32): distance = 220.0
        // Base range = 200.0: out of range normally, in range with +20% rock bonus (240.0)
        const BASE_RANGE: f32 = 200.0;
        const TARGET_X: f32 = 252.0;
        const ATTACKER_X: f32 = 32.0;
        const ATTACKER_Y: f32 = 32.0;

        // Build a grid: 5 wide x 1 tall, cell 0 = Rock, rest = Grass
        let make_grid = |cell0_biome: Biome| {
            let mut grid = WorldGrid::default();
            grid.width = 5;
            grid.height = 1;
            grid.cell_size = 64.0;
            grid.cells = std::iter::once(WorldCell {
                terrain: cell0_biome,
                original_terrain: cell0_biome,
            })
            .chain((1..5).map(|_| WorldCell {
                terrain: Biome::Grass,
                original_terrain: Biome::Grass,
            }))
            .collect();
            grid
        };

        let run_and_count_projs = |biome: Biome| -> usize {
            let mut app = setup_attack_app();
            app.insert_resource(make_grid(biome));

            // Spawn target NPC at slot 1
            let target = app.world_mut().spawn(()).id();
            {
                let mut em = app
                    .world_mut()
                    .resource_mut::<crate::resources::EntityMap>();
                em.register_npc(1, target, Job::Archer, 2, 0);
            }

            // Spawn attacker NPC at slot 0: on cell (0,0)
            let attacker = app
                .world_mut()
                .spawn((
                    GpuSlot(0),
                    Job::Archer,
                    Faction(1),
                    CachedStats {
                        damage: 10.0,
                        range: BASE_RANGE,
                        cooldown: 1.5,
                        projectile_speed: 300.0,
                        projectile_lifetime: 2.0,
                        max_health: 100.0,
                        speed: 100.0,
                        stamina: 1.0,
                        hp_regen: 0.0,
                        berserk_bonus: 0.0,
                    },
                    Activity::default(),
                    Health(100.0),
                    CombatState::default(),
                    AttackTimer(0.0),
                ))
                .id();
            let _ = attacker;

            // GPU state: attacker at (32,32), target at (TARGET_X, 32)
            {
                let mut gpu = app
                    .world_mut()
                    .resource_mut::<crate::resources::GpuReadState>();
                gpu.positions = vec![ATTACKER_X, ATTACKER_Y, TARGET_X, ATTACKER_Y];
                gpu.combat_targets = vec![1, -1]; // attacker targets slot 1
            }

            app.world_mut().run_system_once(attack_system).unwrap();

            // Count projectile spawn messages
            app.world_mut()
                .run_system_once(|mut reader: MessageReader<ProjGpuUpdateMsg>| {
                    reader
                        .read()
                        .filter(|msg| matches!(msg.0, ProjGpuUpdate::Spawn { .. }))
                        .count()
                })
                .unwrap()
        };

        let rock_projs = run_and_count_projs(Biome::Rock);
        let grass_projs = run_and_count_projs(Biome::Grass);

        assert!(
            rock_projs > 0,
            "attacker on Rock should fire at target 220 units away (range={BASE_RANGE} * 1.2 = 240)"
        );
        assert_eq!(
            grass_projs, 0,
            "attacker on Grass should NOT fire at target 220 units away (range={BASE_RANGE})"
        );
    }
}
