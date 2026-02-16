//! Combat systems - Attack processing using GPU targeting results

use bevy::prelude::*;
use crate::components::*;
use crate::constants::{GUARD_POST_RANGE, GUARD_POST_DAMAGE, GUARD_POST_COOLDOWN, GUARD_POST_PROJ_SPEED, GUARD_POST_PROJ_LIFETIME};
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg, BuildingDamageMsg, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE};
use crate::resources::{CombatDebug, GpuReadState, ProjSlotAllocator, ProjHitState, GuardPostState, BuildingHpState, SystemTimings, CombatLog, GameTime, SlotAllocator};
use crate::systemparams::WorldState;
use crate::gpu::ProjBufferWrites;
use crate::world::{self, WorldData, BuildingKind, BuildingSpatialGrid};

/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(
    time: Res<Time>,
    mut query: Query<&mut AttackTimer>,
    mut debug: ResMut<CombatDebug>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("cooldown");
    let dt = time.delta_secs();

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
    mut query: Query<(Entity, &NpcIndex, &CachedStats, &mut AttackTimer, &Faction, &mut CombatState, &Activity, &Job), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut damage_events: MessageWriter<DamageMsg>,
    mut debug: ResMut<CombatDebug>,
    gpu_state: Res<GpuReadState>,
    bgrid: Res<BuildingSpatialGrid>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    timings: Res<SystemTimings>,
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

    for (_entity, npc_idx, cached, mut timer, faction, mut combat_state, activity, job) in query.iter_mut() {
        attackers += 1;
        let i = npc_idx.0;

        // Don't re-engage NPCs heading home (fled combat, delivering food, or resting)
        if matches!(activity, Activity::Returning { .. } | Activity::GoingToRest) {
            if combat_state.is_fighting() {
                *combat_state = CombatState::None;
            }
            continue;
        }

        let target_idx = combat_targets.get(i).copied().unwrap_or(-1);

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
                Job::Archer => 1,
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

/// Process GPU projectile hits: convert to DamageMsg events and recycle slots.
/// Also checks active projectiles against BuildingSpatialGrid for building collisions.
/// Runs before attack_system so freed slots can be reused for new projectiles.
pub fn process_proj_hits(
    mut damage_events: MessageWriter<DamageMsg>,
    mut building_damage_events: MessageWriter<BuildingDamageMsg>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    proj_writes: Res<ProjBufferWrites>,
    proj_pos: Res<crate::resources::ProjPositionState>,
    mut hit_state: ResMut<ProjHitState>,
    bgrid: Res<BuildingSpatialGrid>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("process_proj_hits");
    // Only iterate up to high-water mark — readback returns full MAX buffer but
    // slots beyond proj_alloc.next were never allocated (stale/zero data)
    let max_slot = proj_alloc.next.min(hit_state.0.len());
    for (slot, hit) in hit_state.0[..max_slot].iter().enumerate() {
        // Skip inactive projectiles (deactivated but stale in readback)
        if slot < proj_writes.active.len() && proj_writes.active[slot] == 0 {
            continue;
        }

        let npc_idx = hit[0];
        let processed = hit[1];

        if npc_idx >= 0 && processed == 0 {
            // Collision detected — apply damage and recycle slot
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
            // Expired projectile (lifetime ran out) — recycle slot
            proj_alloc.free(slot);
            if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                queue.push(ProjGpuUpdate::Deactivate { idx: slot });
            }
        }
    }
    hit_state.0.clear();

    // Building collision: check active projectiles against BuildingSpatialGrid
    let hit_radius = 20.0; // building tile is 32px, check within ~half tile
    for slot in 0..proj_alloc.next {
        if slot >= proj_writes.active.len() || proj_writes.active[slot] == 0 { continue; }

        let i2 = slot * 2;
        if i2 + 1 >= proj_pos.0.len() { continue; }
        let px = proj_pos.0[i2];
        let py = proj_pos.0[i2 + 1];
        if px < -9000.0 { continue; }

        let proj_faction = if slot < proj_writes.factions.len() { proj_writes.factions[slot] } else { continue };
        let damage = if slot < proj_writes.damages.len() { proj_writes.damages[slot] } else { 0.0 };
        if damage <= 0.0 { continue; }

        // Collect hit (can't borrow building_damage_events inside for_each_nearby closure)
        let pos = Vec2::new(px, py);
        let mut hit: Option<(BuildingKind, usize)> = None;
        bgrid.for_each_nearby(pos, hit_radius, |bref| {
            if hit.is_some() { return; }
            if bref.faction == proj_faction { return; }
            if bref.position.distance(pos) > hit_radius { return; }
            hit = Some((bref.kind, bref.index));
        });

        if let Some((kind, index)) = hit {
            building_damage_events.write(BuildingDamageMsg { kind, index, amount: damage });
            proj_alloc.free(slot);
            if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                queue.push(ProjGpuUpdate::Deactivate { idx: slot });
            }
        }
    }
}

/// Sync guard post NPC slots: allocate for new posts, free for tombstoned posts.
/// Gated by DirtyFlags::guard_post_slots — only runs when guard posts are built/destroyed/loaded.
pub fn sync_guard_post_slots(
    mut slots: ResMut<SlotAllocator>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut world: WorldState,
) {
    if !world.dirty.guard_post_slots { return; }
    world.dirty.guard_post_slots = false;

    // Collect new allocations needing faction set (can't borrow towns during guard_posts iter_mut)
    let mut new_slots: Vec<(usize, u32)> = Vec::new(); // (slot, town_idx)

    for gp in world.world_data.guard_posts.iter_mut() {
        let alive = gp.position.x > -9000.0;
        match (alive, gp.npc_slot) {
            (true, None) => {
                let Some(slot) = slots.alloc() else { continue };
                gp.npc_slot = Some(slot);
                // Match spawn.rs order: Position, Target, Speed, Health, SpriteFrame
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx: slot, x: gp.position.x, y: gp.position.y }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx: slot, x: gp.position.x, y: gp.position.y }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx: slot, speed: 0.0 }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: slot, health: 999.0 }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx: slot, col: -1.0, row: 0.0, atlas: 0.0 }));
                new_slots.push((slot, gp.town_idx));
            }
            (false, Some(slot)) => {
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::HideNpc { idx: slot }));
                slots.free(slot);
                gp.npc_slot = None;
            }
            _ => {}
        }
    }

    // Set factions for newly allocated slots (needs immutable towns access, separate from iter_mut)
    for (slot, town_idx) in new_slots {
        let faction = world.world_data.towns.get(town_idx as usize).map(|t| t.faction).unwrap_or(0);
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx: slot, faction }));
    }
}

/// Guard post turret auto-attack: reads GPU combat_targets for nearest enemy, fires projectile.
/// State length auto-syncs with WorldData.guard_posts (handles runtime building).
pub fn guard_post_attack_system(
    time: Res<Time>,
    gpu_state: Res<GpuReadState>,
    world_data: Res<WorldData>,
    mut gp_state: ResMut<GuardPostState>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("guard_post_attack");
    let dt = time.delta_secs();
    let positions = &gpu_state.positions;

    // Sync state length with guard post count (handles new builds)
    while gp_state.timers.len() < world_data.guard_posts.len() {
        gp_state.timers.push(0.0);
        gp_state.attack_enabled.push(true);
    }

    let range_sq = GUARD_POST_RANGE * GUARD_POST_RANGE;

    for (i, post) in world_data.guard_posts.iter().enumerate() {
        if i >= gp_state.timers.len() { break; }
        if !gp_state.attack_enabled[i] { continue; }
        let Some(slot) = post.npc_slot else { continue }; // No GPU slot yet

        // Decrement cooldown
        if gp_state.timers[i] > 0.0 {
            gp_state.timers[i] = (gp_state.timers[i] - dt).max(0.0);
            if gp_state.timers[i] > 0.0 { continue; }
        }

        // Read GPU combat_targets — O(1) instead of scanning all NPCs
        let target_idx = gpu_state.combat_targets.get(slot).copied().unwrap_or(-1);
        if target_idx < 0 { continue; }
        let target = target_idx as usize;

        let px = post.position.x;
        let py = post.position.y;
        let tx = positions.get(target * 2).copied().unwrap_or(-9999.0);
        let ty = positions.get(target * 2 + 1).copied().unwrap_or(-9999.0);
        if tx < -9000.0 { continue; }

        let dx = tx - px;
        let dy = ty - py;
        let dist_sq = dx * dx + dy * dy;
        if dist_sq > range_sq { continue; } // GPU found target but it's out of weapon range

        let dist = dist_sq.sqrt();
        if dist > 1.0 {
            if let Some(proj_slot) = proj_alloc.alloc() {
                let dir_x = dx / dist;
                let dir_y = dy / dist;
                if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                    queue.push(ProjGpuUpdate::Spawn {
                        idx: proj_slot,
                        x: px, y: py,
                        vx: dir_x * GUARD_POST_PROJ_SPEED,
                        vy: dir_y * GUARD_POST_PROJ_SPEED,
                        damage: GUARD_POST_DAMAGE,
                        faction: world_data.towns.get(post.town_idx as usize)
                            .map(|t| t.faction).unwrap_or(0),
                        shooter: -1, // Building, not NPC
                        lifetime: GUARD_POST_PROJ_LIFETIME,
                    });
                }
            }
        }
        gp_state.timers[i] = GUARD_POST_COOLDOWN;
    }
}

/// Process building damage messages: decrement HP, destroy when HP reaches 0.
pub fn building_damage_system(
    mut damage_reader: MessageReader<BuildingDamageMsg>,
    mut world: WorldState,
    mut combat_log: ResMut<CombatLog>,
    game_time: Res<GameTime>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("building_damage");
    for msg in damage_reader.read() {
        let Some(hp) = world.building_hp.get_mut(msg.kind, msg.index) else { continue };
        if *hp <= 0.0 { continue; } // already dead

        *hp -= msg.amount;
        if *hp > 0.0 { continue; } // still alive
        *hp = 0.0;

        // Building destroyed — find its position and town to call destroy_building
        let (pos, town_idx) = match msg.kind {
            BuildingKind::GuardPost => world.world_data.guard_posts.get(msg.index)
                .map(|g| (g.position, g.town_idx as usize)),
            BuildingKind::FarmerHome => world.world_data.farmer_homes.get(msg.index)
                .map(|h| (h.position, h.town_idx as usize)),
            BuildingKind::ArcherHome => world.world_data.archer_homes.get(msg.index)
                .map(|a| (a.position, a.town_idx as usize)),
            BuildingKind::Tent => world.world_data.tents.get(msg.index)
                .map(|t| (t.position, t.town_idx as usize)),
            BuildingKind::MinerHome => world.world_data.miner_homes.get(msg.index)
                .map(|m| (m.position, m.town_idx as usize)),
            BuildingKind::Farm => world.world_data.farms.get(msg.index)
                .map(|f| (f.position, f.town_idx as usize)),
            BuildingKind::Town => world.world_data.towns.get(msg.index)
                .map(|t| (t.center, msg.index)),
            BuildingKind::Bed => world.world_data.beds.get(msg.index)
                .map(|b| (b.position, b.town_idx as usize)),
            BuildingKind::GoldMine => world.world_data.gold_mines.get(msg.index)
                .map(|m| (m.position, 0)),
        }.unwrap_or((Vec2::ZERO, 0));

        if pos.x < -9000.0 { continue; } // already tombstoned

        let center = world.world_data.towns.get(town_idx)
            .map(|t| t.center).unwrap_or_default();
        let town_name = world.world_data.towns.get(town_idx)
            .map(|t| t.name.clone()).unwrap_or_default();
        let (trow, tcol) = world::world_to_town_grid(center, pos);

        // Capture linked NPC slot BEFORE destroy_building tombstones the spawner
        let npc_slot = world.spawner_state.0.iter()
            .find(|s| (s.position - pos).length() < 1.0)
            .map(|s| s.npc_slot)
            .unwrap_or(-1);

        let _ = world::destroy_building(
            &mut world.grid, &mut world.world_data, &mut world.farm_states,
            &mut world.spawner_state, &mut world.building_hp,
            &mut combat_log, &game_time,
            trow, tcol, center,
            &format!("{:?} destroyed in {}", msg.kind, town_name),
        );
        if msg.kind == BuildingKind::GuardPost {
            world.dirty.patrols = true;
            world.dirty.guard_post_slots = true;
        }
        world.dirty.building_grid = true;

        // Kill the linked NPC if alive
        if npc_slot >= 0 {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::HideNpc { idx: npc_slot as usize }));
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx: npc_slot as usize, health: 0.0 }));
        }
    }
}

/// Populate BuildingHpRender from BuildingHpState + WorldData (only damaged buildings).
pub fn sync_building_hp_render(
    building_hp: Res<BuildingHpState>,
    world_data: Res<WorldData>,
    mut render: ResMut<crate::resources::BuildingHpRender>,
) {
    render.positions.clear();
    render.health_pcts.clear();
    for (pos, pct) in building_hp.iter_damaged(&world_data) {
        render.positions.push(pos);
        render.health_pcts.push(pct);
    }
}
