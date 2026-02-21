//! Combat systems - Attack processing using GPU targeting results

use bevy::prelude::*;
use crate::components::*;
use crate::constants::{TowerStats, ItemKind, building_def};
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg, BuildingDamageMsg, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE};
use crate::resources::{CombatDebug, GpuReadState, ProjSlotAllocator, ProjHitState, TowerState, TowerKindState, BuildingHpState, SystemTimings, CombatLog, CombatEventKind, GameTime, NpcEntityMap, NpcMetaCache};
use crate::systems::stats::{TownUpgrades, resolve_town_tower_stats};
use crate::systemparams::WorldState;
use crate::gpu::ProjBufferWrites;
use crate::resources::BuildingSlotMap;
use crate::world::{self, WorldData, BuildingKind, BuildingSpatialGrid};

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
    mut debug: ResMut<CombatDebug>,
    gpu_state: Res<GpuReadState>,
    npc_map: Res<NpcEntityMap>,
    bgrid: Res<BuildingSpatialGrid>,
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
        // Clear ManualTarget if the target is dead.
        let target_idx = if let Some(mt) = manual_target {
            let t = mt.0;
            let dead = gpu_state.health.get(t).map_or(true, |&h| h <= 0.0);
            if dead {
                commands.entity(entity).remove::<ManualTarget>();
                combat_targets.get(i).copied().unwrap_or(-1)
            } else {
                t as i32
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

            if let Some((_bkind, _bidx, bpos)) = world::find_nearest_enemy_building(
                Vec2::new(x, y), &bgrid, faction.0, job_id, cached.range,
            ) {
                // In range and cooldown ready — fire at building
                if timer.0 <= 0.0 {
                    let dx = bpos.x - x;
                    let dy = bpos.y - y;
                    let dist = (dx * dx + dy * dy).sqrt();
                    if dist > 1.0 {
                        // Stand ground while shooting building
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: i, x, y }));
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
                        timer.0 = cached.cooldown;
                        attacks += 1;
                    }
                }
            }
            continue;
        }

        targets_found += 1;

        let ti = target_idx as usize;

        // Validate GPU target: must be a real live enemy NPC (not building slot, self, or stale)
        if ti == i || !npc_map.0.contains_key(&ti) {
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
    mut building_damage_events: MessageWriter<BuildingDamageMsg>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    proj_writes: Res<ProjBufferWrites>,
    mut hit_state: ResMut<ProjHitState>,
    building_slots: Res<BuildingSlotMap>,
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
                // Check if the hit NPC slot is actually a building
                if let Some((kind, index)) = building_slots.get_building(npc_idx as usize) {
                    let attacker_faction = if slot < proj_writes.factions.len() {
                        proj_writes.factions[slot]
                    } else { 0 };
                    let attacker = if slot < proj_writes.shooters.len() {
                        proj_writes.shooters[slot]
                    } else { -1 };
                    building_damage_events.write(BuildingDamageMsg {
                        kind, index, amount: damage, attacker_faction, attacker,
                    });
                } else {
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

/// Shared tower loop: for each building with attack enabled, check GPU combat target, fire projectile.
fn fire_towers(
    dt: f32,
    positions: &[f32],
    combat_targets: &[i32],
    building_slots: &BuildingSlotMap,
    proj_alloc: &mut ProjSlotAllocator,
    state: &mut TowerKindState,
    buildings: &[(Vec2, i32, TowerStats, BuildingKind)],
) {
    for (i, &(pos, faction, stats, kind)) in buildings.iter().enumerate() {
        if i >= state.attack_enabled.len() || !state.attack_enabled[i] { continue; }
        let Some(slot) = building_slots.get_slot(kind, i) else { continue };

        // Decrement cooldown
        if i < state.timers.len() && state.timers[i] > 0.0 {
            state.timers[i] = (state.timers[i] - dt).max(0.0);
            if state.timers[i] > 0.0 { continue; }
        }

        // Read GPU combat_targets for nearest enemy
        let target_idx = combat_targets.get(slot).copied().unwrap_or(-1);
        if target_idx < 0 { continue; }
        let target = target_idx as usize;

        let tx = positions.get(target * 2).copied().unwrap_or(-9999.0);
        let ty = positions.get(target * 2 + 1).copied().unwrap_or(-9999.0);
        if tx < -9000.0 { continue; }

        let dx = tx - pos.x;
        let dy = ty - pos.y;
        let dist_sq = dx * dx + dy * dy;
        if dist_sq > stats.range * stats.range { continue; }

        let dist = dist_sq.sqrt();
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
    building_slots: Res<BuildingSlotMap>,
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
            tower.town.attack_enabled[i] = town.sprite_type == 0;
        }
    }
    let town_buildings: Vec<_> = world_data.towns.iter().enumerate()
        .map(|(i, t)| {
            let levels = upgrades.town_levels(i);
            let kind = if t.faction == 0 { BuildingKind::Fountain } else { BuildingKind::Camp };
            (t.center, t.faction, resolve_town_tower_stats(&levels), kind)
        })
        .collect();

    fire_towers(dt, &gpu_state.positions, &gpu_state.combat_targets,
        &building_slots, &mut proj_alloc,
        &mut tower.town, &town_buildings);
}

/// Process building damage messages: decrement HP, destroy when HP reaches 0.
/// On destruction, grants loot (half building cost as food) to the attacker NPC.
pub fn building_damage_system(
    mut damage_reader: MessageReader<BuildingDamageMsg>,
    mut world: WorldState,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    timings: Res<SystemTimings>,
    npc_map: Res<NpcEntityMap>,
    mut loot_query: Query<(&NpcIndex, &mut Activity, &Home, &Faction), Without<Dead>>,
    npc_meta: Res<NpcMetaCache>,
    mut ai_state: ResMut<crate::systems::AiPlayerState>,
    mut endless: ResMut<crate::resources::EndlessMode>,
    upgrades: Res<crate::systems::stats::TownUpgrades>,
    food_storage: Res<crate::resources::FoodStorage>,
    gold_storage: Res<crate::resources::GoldStorage>,
) {
    let _t = timings.scope("building_damage");
    for msg in damage_reader.read() {
        if msg.kind == BuildingKind::GoldMine { continue; } // mines are indestructible
        let Some(hp) = world.building_hp.get_mut(msg.kind, msg.index) else { continue };
        if *hp <= 0.0 { continue; } // already dead

        *hp -= msg.amount;
        let new_hp = (*hp).max(0.0);
        let max_hp = BuildingHpState::max_hp(msg.kind);

        // Look up position and town for logging
        let Some((pos, town_idx_u32)) = world.world_data.building_pos_town(msg.kind, msg.index) else { continue };
        let town_idx = town_idx_u32 as usize;

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

        *hp = new_hp;

        // Sync HP to GPU building slot
        if let Some(slot) = world.building_slots.get_slot(msg.kind, msg.index) {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: slot, health: new_hp }));
        }

        if new_hp > 0.0 { continue; } // still alive

        // Building destroyed
        let center = world.world_data.towns.get(town_idx)
            .map(|t| t.center).unwrap_or_default();
        let (trow, tcol) = world::world_to_town_grid(center, pos);

        // Capture linked NPC slot BEFORE destroy_building tombstones the spawner
        let npc_slot = world.spawner_state.0.iter()
            .find(|s| (s.position - pos).length() < 1.0)
            .map(|s| s.npc_slot)
            .unwrap_or(-1);

        let _ = world::destroy_building(
            &mut world.grid, &mut world.world_data, &mut world.farm_states,
            &mut world.spawner_state, &mut world.building_hp,
            &mut world.slot_alloc, &mut world.building_slots,
            &mut combat_log, &game_time,
            trow, tcol, center,
            &format!("{:?} destroyed in {}", msg.kind, town_name),
        );
        world.dirty.mark_building_changed(msg.kind);

        // Town center destroyed — deactivate AI player (town becomes leaderless)
        if matches!(msg.kind, BuildingKind::Fountain | BuildingKind::Camp) {
            if let Some(player) = ai_state.players.iter_mut().find(|p| p.town_data_idx == town_idx) {
                player.active = false;
            }
            combat_log.push(CombatEventKind::Raid, defender_faction,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} has been defeated!", town_name));
            info!("{} (town_idx={}) defeated — AI deactivated", town_name, town_idx);

            // Endless mode: queue replacement AI scaled to player strength
            if endless.enabled {
                let is_camp = world.world_data.towns.get(town_idx)
                    .map(|t| t.sprite_type == 1).unwrap_or(true);
                let player_town = world.world_data.towns.iter().position(|t| t.faction == 0).unwrap_or(0);
                let player_levels = upgrades.town_levels(player_town);
                let frac = endless.strength_fraction;
                let scaled_levels: Vec<u8> = player_levels.iter()
                    .map(|&lv| (lv as f32 * frac).round() as u8)
                    .collect();
                let starting_food = (food_storage.food.get(player_town).copied().unwrap_or(0) as f32 * frac) as i32;
                let starting_gold = (gold_storage.gold.get(player_town).copied().unwrap_or(0) as f32 * frac) as i32;
                endless.pending_spawns.push(crate::resources::PendingAiSpawn {
                    delay_remaining: crate::constants::ENDLESS_RESPAWN_DELAY_HOURS,
                    is_camp,
                    upgrade_levels: scaled_levels,
                    starting_food,
                    starting_gold,
                });
                info!("Endless mode: queued replacement AI (is_camp={}, delay={}h, strength={:.0}%)",
                    is_camp, crate::constants::ENDLESS_RESPAWN_DELAY_HOURS, frac * 100.0);
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
                    if let Ok((npc_idx, mut activity, home, faction)) = loot_query.get_mut(attacker_entity) {
                        if matches!(&*activity, Activity::Returning { .. }) {
                            activity.add_loot(drop.item, amount);
                        } else {
                            *activity = Activity::Returning { loot: vec![(drop.item, amount)] };
                        }
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: npc_idx.0, x: home.0.x, y: home.0.y }));

                        let item_name = match drop.item { ItemKind::Food => "food", ItemKind::Gold => "gold" };
                        let killer_name = &npc_meta.0[npc_idx.0].name;
                        let killer_job = crate::job_name(npc_meta.0[npc_idx.0].job);
                        combat_log.push(CombatEventKind::Loot, faction.0,
                            game_time.day(), game_time.hour(), game_time.minute(),
                            format!("{} '{}' looted {} {} from {:?}", killer_job, killer_name, amount, item_name, msg.kind));
                    }
                }
            }
        }
    }
}

/// Populate BuildingHpRender from BuildingHpState + WorldData (only damaged buildings).
pub fn sync_building_hp_render(
    building_hp: Res<BuildingHpState>,
    world_data: Res<WorldData>,
    mut render: ResMut<crate::resources::BuildingHpRender>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("sync_hp_render");
    render.positions.clear();
    render.health_pcts.clear();
    for (pos, pct) in building_hp.iter_damaged(&world_data) {
        render.positions.push(pos);
        render.health_pcts.push(pct);
    }
}
