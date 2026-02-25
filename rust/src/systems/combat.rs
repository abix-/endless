//! Combat systems - Attack processing using GPU targeting results

use bevy::prelude::*;
use crate::components::*;
use crate::constants::{TowerStats, ItemKind, building_def};
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg, BuildingDamageMsg, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE};
use crate::resources::{CombatDebug, GpuReadState, ProjSlotAllocator, ProjHitState, TowerState, TowerKindState, SystemTimings, CombatLog, CombatEventKind, GameTime, NpcEntityMap, NpcMetaCache};
use crate::systems::stats::{TownUpgrades, resolve_town_tower_stats};
use crate::systemparams::WorldState;
use crate::gpu::ProjBufferWrites;
use crate::resources::BuildingEntityMap;
use crate::world::{self, WorldData, BuildingKind, is_alive};

/// Bundled params for building destruction side effects (loot, endless respawn).
#[derive(bevy::ecs::system::SystemParam)]
pub struct BuildingDeathExtra<'w> {
    npc_meta: Res<'w, NpcMetaCache>,
    squad_state: Res<'w, crate::resources::SquadState>,
    ai_state: ResMut<'w, crate::systems::AiPlayerState>,
    endless: ResMut<'w, crate::resources::EndlessMode>,
    upgrades: Res<'w, crate::systems::stats::TownUpgrades>,
    food_storage: Res<'w, crate::resources::FoodStorage>,
    gold_storage: Res<'w, crate::resources::GoldStorage>,
}

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
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut damage_events: MessageWriter<DamageMsg>,
    mut building_damage_events: MessageWriter<BuildingDamageMsg>,
    mut debug: ResMut<CombatDebug>,
    gpu_state: Res<GpuReadState>,
    npc_map: Res<NpcEntityMap>,
    bmap: Res<BuildingEntityMap>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    timings: Res<SystemTimings>,
    squad_state: Res<crate::resources::SquadState>,
    mut commands: Commands,
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

    for (entity, npc_idx, cached, mut timer, faction, mut combat_state, activity, job, manual_target, squad_id) in query.iter_mut() {
        attackers += 1;
        let i = npc_idx.0;

        // Don't re-engage NPCs heading home (fled combat, delivering food, or resting)
        if matches!(activity, Activity::Returning { .. } | Activity::GoingToRest | Activity::Resting) {
            if combat_state.is_fighting() {
                *combat_state = CombatState::None;
            }
            continue;
        }

        // Manual target override: player-assigned focus-fire takes priority over GPU auto-target.
        // Clear ManualTarget::Npc if the target is dead.
        let target_idx = if let Some(mt) = manual_target {
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

        if attackers == 1 {
            sample_target = target_idx;
        }

        // No NPC combat target — try opportunistic building attack
        if target_idx < 0 {
            if combat_state.is_fighting() {
                *combat_state = CombatState::None;
            }
            // Only ranged NPCs (archers/raiders) attack buildings
            let job_id = match job {
                Job::Archer | Job::Crossbow => 1,
                Job::Raider => 2,
                _ => { continue; }
            };
            if i * 2 + 1 >= positions.len() { continue; }
            let (x, y) = (positions[i * 2], positions[i * 2 + 1]);
            if x < -9000.0 { continue; } // dead/hidden

            if let Some((bkind, bidx, bpos)) = world::find_nearest_enemy_building(
                Vec2::new(x, y), &bmap, faction.0, job_id, cached.range,
            ) {
                // In range and cooldown ready — fire at building
                if timer.0 <= 0.0 {
                    let dx = bpos.x - x;
                    let dy = bpos.y - y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > 1.0 {
                        // Stand ground while shooting building
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: i, x, y }));
                        // Spawn visual projectile (no GPU collision with buildings)
                        if let Some(proj_slot) = proj_alloc.alloc() {
                            let dir_x = dx / dist;
                            let dir_y = dy / dist;
                            if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                                queue.push(ProjGpuUpdate::Spawn {
                                    idx: proj_slot,
                                    x, y,
                                    vx: dir_x * cached.projectile_speed,
                                    vy: dir_y * cached.projectile_speed,
                                    damage: 0.0, // visual only — damage applied directly below
                                    faction: faction.0,
                                    shooter: i as i32,
                                    lifetime: cached.projectile_lifetime,
                                });
                            }
                        }
                        // Apply building damage directly (buildings not in NPC GPU buffer)
                        building_damage_events.write(BuildingDamageMsg {
                            kind: bkind,
                            index: bidx,
                            amount: cached.damage,
                            attacker_faction: faction.0,
                            attacker: i as i32,
                        });
                        timer.0 = cached.cooldown;
                        attacks += 1;
                    }
                }
            }
            continue;
        }

        targets_found += 1;

        let ti = target_idx as usize;

        // Validate GPU target: must be a real live enemy NPC (not building, self, or stale)
        if ti == i {
            if combat_state.is_fighting() { *combat_state = CombatState::None; }
            continue;
        }
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

        if i * 2 + 1 >= positions.len() || ti * 2 + 1 >= positions.len() {
            bounds_failures += 1;
            continue;
        }

        let (x, y) = (positions[i * 2], positions[i * 2 + 1]);

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
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: i, x, y }));

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
                        if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                            queue.push(ProjGpuUpdate::Spawn {
                                idx: proj_slot,
                                x, y,
                                vx: dir_x * cached.projectile_speed,
                                vy: dir_y * cached.projectile_speed,
                                damage: cached.damage,
                                faction: faction.0,
                                shooter: i as i32,
                                lifetime: cached.projectile_lifetime,
                            });
                        }
                    }
                } else {
                    // Point blank — apply damage directly (no projectile needed)
                    damage_events.write(DamageMsg {
                        npc_index: target_idx as usize,
                        amount: cached.damage,
                        attacker: i as i32,
                    });
                }

                attacks += 1;
                timer.0 = cached.cooldown;
            }
        } else {
            // Out of range - chase target
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: i, x: tx, y: ty }));
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

/// Process GPU projectile hits: convert to DamageMsg or BuildingDamageMsg events and recycle slots.
/// Building hits are detected by the GPU collision pipeline (buildings occupy NPC slots with speed=0).
/// Runs before attack_system so freed slots can be reused for new projectiles.
pub fn process_proj_hits(
    mut damage_events: MessageWriter<DamageMsg>,
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

        let npc_idx = hit[0];
        let processed = hit[1];

        if npc_idx >= 0 && processed == 0 {
            let damage = if slot < proj_writes.damages.len() {
                proj_writes.damages[slot]
            } else {
                0.0
            };

            if damage > 0.0 {
                // Buildings are on separate GPU buffers now — all NPC-buffer hits are NPC hits
                let shooter = if slot < proj_writes.shooters.len() {
                    proj_writes.shooters[slot]
                } else {
                    -1
                };
                damage_events.write(DamageMsg {
                    npc_index: npc_idx as usize,
                    amount: damage,
                    attacker: shooter,
                });
            }

            proj_alloc.free(slot);
            if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                queue.push(ProjGpuUpdate::Deactivate { idx: slot });
            }
        } else if npc_idx == -2 {
            proj_alloc.free(slot);
            if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                queue.push(ProjGpuUpdate::Deactivate { idx: slot });
            }
        }
    }
    hit_state.0.clear();
}

/// Shared tower loop: CPU-side nearest-enemy targeting, fire projectiles.
/// Buildings are on separate GPU buffers, so tower targeting is done CPU-side
/// by scanning NPC positions/factions (towers are few, so CPU iteration is fine).
fn fire_towers(
    dt: f32,
    npc_positions: &[f32],
    npc_factions: &[i32],
    npc_health: &[f32],
    npc_count: usize,
    proj_alloc: &mut ProjSlotAllocator,
    state: &mut TowerKindState,
    buildings: &[(Vec2, i32, TowerStats, BuildingKind)],
) {
    for (i, &(pos, faction, stats, _kind)) in buildings.iter().enumerate() {
        if i >= state.attack_enabled.len() || !state.attack_enabled[i] { continue; }

        // Decrement cooldown
        if i < state.timers.len() && state.timers[i] > 0.0 {
            state.timers[i] = (state.timers[i] - dt).max(0.0);
            if state.timers[i] > 0.0 { continue; }
        }

        // CPU-side: find nearest enemy NPC within range
        let range_sq = stats.range * stats.range;
        let mut best_dist_sq = f32::MAX;
        let mut best_target: Option<usize> = None;

        for j in 0..npc_count {
            let jf = npc_factions.get(j).copied().unwrap_or(-1);
            if jf < 0 || jf == faction { continue; }
            let jh = npc_health.get(j).copied().unwrap_or(0.0);
            if jh <= 0.0 { continue; }
            let jx = npc_positions.get(j * 2).copied().unwrap_or(-9999.0);
            let jy = npc_positions.get(j * 2 + 1).copied().unwrap_or(-9999.0);
            if jx < -9000.0 { continue; }
            let dx = jx - pos.x;
            let dy = jy - pos.y;
            let d2 = dx * dx + dy * dy;
            if d2 < best_dist_sq && d2 <= range_sq {
                best_dist_sq = d2;
                best_target = Some(j);
            }
        }

        let Some(target) = best_target else { continue };
        let tx = npc_positions[target * 2];
        let ty = npc_positions[target * 2 + 1];
        let dx = tx - pos.x;
        let dy = ty - pos.y;
        let dist = best_dist_sq.sqrt();

        if dist > 1.0 {
            if let Some(proj_slot) = proj_alloc.alloc() {
                let dir_x = dx / dist;
                let dir_y = dy / dist;
                if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                    queue.push(ProjGpuUpdate::Spawn {
                        idx: proj_slot,
                        x: pos.x, y: pos.y,
                        vx: dir_x * stats.proj_speed,
                        vy: dir_y * stats.proj_speed,
                        damage: stats.damage,
                        faction,
                        shooter: -1,
                        lifetime: stats.proj_lifetime,
                    });
                }
            }
        }
        if i < state.timers.len() {
            state.timers[i] = stats.cooldown;
        }
    }
}

/// Building tower auto-attack: tower buildings fire at nearby enemies using GPU combat targets.
pub fn building_tower_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    gpu_state: Res<GpuReadState>,
    world_data: Res<WorldData>,
    upgrades: Res<TownUpgrades>,
    slots: Res<crate::resources::SlotAllocator>,
    mut tower: ResMut<TowerState>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
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
    let town_buildings: Vec<_> = world_data.towns.iter().enumerate()
        .map(|(i, t)| {
            let levels = upgrades.town_levels(i);
            let kind = BuildingKind::Fountain;
            (t.center, t.faction, resolve_town_tower_stats(&levels), kind)
        })
        .collect();

    let npc_count = slots.count();
    fire_towers(dt, &gpu_state.positions, &gpu_state.factions, &gpu_state.health,
        npc_count, &mut proj_alloc,
        &mut tower.town, &town_buildings);
}

/// Process building damage messages: decrement HP, destroy when HP reaches 0.
/// On destruction, grants loot (half building cost as food) to the attacker NPC.
pub fn building_damage_system(
    mut commands: Commands,
    mut damage_reader: MessageReader<BuildingDamageMsg>,
    mut world: WorldState,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    timings: Res<SystemTimings>,
    npc_map: Res<NpcEntityMap>,
    mut building_health: Query<&mut Health, With<Building>>,
    mut loot_query: Query<(&NpcIndex, &mut Activity, &Home, &Faction, Option<&DirectControl>), Without<Dead>>,
    mut extra: BuildingDeathExtra,
) {
    let _t = timings.scope("building_damage");
    for msg in damage_reader.read() {
        if matches!(msg.kind, BuildingKind::GoldMine | BuildingKind::Road) { continue; } // mines + roads are indestructible

        // Look up building entity via BuildingEntityMap
        let Some(entity) = world.building_slots.get_entity_by_building(msg.kind, msg.index) else { continue };
        let Ok(mut health) = building_health.get_mut(entity) else { continue };
        if health.0 <= 0.0 { continue; } // already dead

        health.0 = (health.0 - msg.amount).max(0.0);
        let new_hp = health.0;
        let max_hp = crate::constants::building_def(msg.kind).hp;

        // Look up position and town for logging (via BuildingEntityMap instance)
        let Some(slot) = world.building_slots.get_slot(msg.kind, msg.index) else { continue };
        let Some(inst) = world.building_slots.get_instance(slot) else { continue };
        let pos = inst.position;
        let town_idx = inst.town_idx as usize;

        // Mark dirty so healing_system knows to run
        if new_hp > 0.0 { world.dirty.buildings_need_healing = true; }

        let town_name = world.world_data.towns.get(town_idx)
            .map(|t| t.name.clone()).unwrap_or_default();
        let attacker_name = world.world_data.towns.get(msg.attacker_faction as usize)
            .map(|t| t.name.as_str()).unwrap_or("?");

        // Log every damage hit to combat log
        let defender_faction = world.world_data.towns.get(town_idx).map(|t| t.faction).unwrap_or(0);
        combat_log.push(CombatEventKind::BuildingDamage, defender_faction,
            game_time.day(), game_time.hour(), game_time.minute(),
            format!("{:?} in {} hit by {} for {:.0} ({:.0}/{:.0} HP)", msg.kind, town_name, attacker_name, msg.amount, new_hp, max_hp));

        // GPU sync (damage flash + health bar) — building buffer
        if let Some(slot) = world.building_slots.get_slot(msg.kind, msg.index) {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::BldSetHealth { idx: slot, health: new_hp }));
        }

        if new_hp > 0.0 { continue; } // still alive

        // Building destroyed
        let center = world.world_data.towns.get(town_idx)
            .map(|t| t.center).unwrap_or_default();
        let (trow, tcol) = world::world_to_town_grid(center, pos);

        // Capture linked NPC slot from BuildingInstance (O(1) lookup)
        let npc_slot = world.building_slots.find_by_position(pos)
            .map(|i| i.npc_slot)
            .unwrap_or(-1);

        let _ = world.destroy_building(
            &mut combat_log, &game_time,
            trow, tcol, center,
            &format!("{:?} destroyed in {}", msg.kind, town_name),
            &mut commands,
        );
        world.dirty.mark_building_changed(msg.kind);

        // Town center destroyed — deactivate AI player (town becomes leaderless)
        if matches!(msg.kind, BuildingKind::Fountain) {
            if let Some(player) = extra.ai_state.players.iter_mut().find(|p| p.town_data_idx == town_idx) {
                player.active = false;
            }
            combat_log.push(CombatEventKind::Raid, defender_faction,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} has been defeated!", town_name));
            info!("{} (town_idx={}) defeated — AI deactivated", town_name, town_idx);

            // Endless mode: queue replacement AI scaled to player strength
            if extra.endless.enabled {
                let is_raider = world.world_data.towns.get(town_idx)
                    .map(|t| t.sprite_type == 1).unwrap_or(true);
                let player_town = world.world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
                let player_levels = extra.upgrades.town_levels(player_town);
                let frac = extra.endless.strength_fraction;
                let scaled_levels: Vec<u8> = player_levels.iter()
                    .map(|&lv| (lv as f32 * frac).round() as u8)
                    .collect();
                let starting_food = (extra.food_storage.food.get(player_town).copied().unwrap_or(0) as f32 * frac) as i32;
                let starting_gold = (extra.gold_storage.gold.get(player_town).copied().unwrap_or(0) as f32 * frac) as i32;
                extra.endless.pending_spawns.push(crate::resources::PendingAiSpawn {
                    delay_remaining: crate::constants::ENDLESS_RESPAWN_DELAY_HOURS,
                    is_raider,
                    upgrade_levels: scaled_levels,
                    starting_food,
                    starting_gold,
                });
                info!("Endless mode: queued replacement AI (is_raider={}, delay={}h, strength={:.0}%)",
                    is_raider, crate::constants::ENDLESS_RESPAWN_DELAY_HOURS, frac * 100.0);
            }
        }

        // Kill the linked NPC if alive
        if npc_slot >= 0 {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::HideNpc { idx: npc_slot as usize }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: npc_slot as usize, health: 0.0 }));
        }

        // Loot: attacker picks up building loot and returns home
        if let Some(drop) = building_def(msg.kind).loot_drop() {
            let amount = if drop.min == drop.max { drop.min } else {
                drop.min + ((msg.index as i32) % (drop.max - drop.min + 1))
            };
            if amount > 0 && msg.attacker >= 0 {
                let attacker_slot = msg.attacker as usize;
                if let Some(&attacker_entity) = npc_map.0.get(&attacker_slot) {
                    if let Ok((npc_idx, mut activity, home, faction, dc)) = loot_query.get_mut(attacker_entity) {
                        let dc_keep_fighting = dc.is_some() && extra.squad_state.dc_no_return;

                        if matches!(&*activity, Activity::Returning { .. }) {
                            activity.add_loot(drop.item, amount);
                        } else {
                            *activity = Activity::Returning { loot: vec![(drop.item, amount)] };
                        }
                        if !dc_keep_fighting {
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: npc_idx.0, x: home.0.x, y: home.0.y }));
                        }

                        let item_name = match drop.item { ItemKind::Food => "food", ItemKind::Gold => "gold" };
                        let killer_name = &extra.npc_meta.0[npc_idx.0].name;
                        let killer_job = crate::job_name(extra.npc_meta.0[npc_idx.0].job);
                        combat_log.push(CombatEventKind::Loot, faction.0,
                            game_time.day(), game_time.hour(), game_time.minute(),
                            format!("{} '{}' looted {} {} from {:?}", killer_job, killer_name, amount, item_name, msg.kind));
                    }
                }
            }
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
