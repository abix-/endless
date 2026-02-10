//! Combat systems - Attack processing using GPU targeting results

use bevy::prelude::*;
use crate::components::*;
use crate::constants::{GUARD_POST_RANGE, GUARD_POST_DAMAGE, GUARD_POST_COOLDOWN, GUARD_POST_PROJ_SPEED, GUARD_POST_PROJ_LIFETIME};
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE};
use crate::resources::{CombatDebug, GpuReadState, ProjSlotAllocator, ProjHitState, GuardPostState};
use crate::gpu::ProjBufferWrites;
use crate::world::WorldData;

/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(
    time: Res<Time>,
    mut query: Query<&mut AttackTimer>,
    mut debug: ResMut<CombatDebug>,
) {
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
    mut query: Query<(Entity, &NpcIndex, &CachedStats, &mut AttackTimer, &Faction, &mut CombatState), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut damage_events: MessageWriter<DamageMsg>,
    mut debug: ResMut<CombatDebug>,
    gpu_state: Res<GpuReadState>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
) {
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

    for (_entity, npc_idx, cached, mut timer, faction, mut combat_state) in query.iter_mut() {
        attackers += 1;
        let i = npc_idx.0;

        let target_idx = combat_targets.get(i).copied().unwrap_or(-1);

        if attackers == 1 {
            sample_target = target_idx;
        }

        // No combat target - clear combat state, activity preserved so NPC resumes
        if target_idx < 0 {
            if combat_state.is_fighting() {
                *combat_state = CombatState::None;
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

        let dx = tx - x;
        let dy = ty - y;
        let dist = (dx * dx + dy * dy).sqrt();

        if attackers == 1 {
            sample_dist = dist;
        }

        if dist <= cached.range {
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
/// Runs before attack_system so freed slots can be reused for new projectiles.
pub fn process_proj_hits(
    mut damage_events: MessageWriter<DamageMsg>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
    proj_writes: Res<ProjBufferWrites>,
    mut hit_state: ResMut<ProjHitState>,
) {
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
                damage_events.write(DamageMsg {
                    npc_index: npc_idx as usize,
                    amount: damage,
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
}

/// Guard post turret auto-attack: scans for nearest enemy within range, fires projectile.
/// State length auto-syncs with WorldData.guard_posts (handles runtime building).
pub fn guard_post_attack_system(
    time: Res<Time>,
    gpu_state: Res<GpuReadState>,
    world_data: Res<WorldData>,
    mut gp_state: ResMut<GuardPostState>,
    mut proj_alloc: ResMut<ProjSlotAllocator>,
) {
    let dt = time.delta_secs();
    let positions = &gpu_state.positions;
    let factions = &gpu_state.factions;
    let npc_count = positions.len() / 2;

    // Sync state length with guard post count (handles new builds)
    while gp_state.timers.len() < world_data.guard_posts.len() {
        gp_state.timers.push(0.0);
        gp_state.attack_enabled.push(true);
    }

    let range_sq = GUARD_POST_RANGE * GUARD_POST_RANGE;

    for (i, post) in world_data.guard_posts.iter().enumerate() {
        if i >= gp_state.timers.len() { break; }
        if !gp_state.attack_enabled[i] { continue; }

        // Decrement cooldown
        if gp_state.timers[i] > 0.0 {
            gp_state.timers[i] = (gp_state.timers[i] - dt).max(0.0);
            if gp_state.timers[i] > 0.0 { continue; }
        }

        // Find nearest enemy within range
        let px = post.position.x;
        let py = post.position.y;
        let mut best_dist_sq = range_sq;
        let mut best_idx: Option<usize> = None;

        for n in 0..npc_count {
            if n >= factions.len() { continue; }
            if factions[n] == 0 { continue; } // Don't shoot friendlies
            let nx = positions[n * 2];
            let ny = positions[n * 2 + 1];
            if nx < -9000.0 { continue; } // Hidden/dead

            let dx = nx - px;
            let dy = ny - py;
            let dist_sq = dx * dx + dy * dy;
            if dist_sq < best_dist_sq {
                best_dist_sq = dist_sq;
                best_idx = Some(n);
            }
        }

        // Fire projectile at target
        if let Some(target) = best_idx {
            let tx = positions[target * 2];
            let ty = positions[target * 2 + 1];
            let dx = tx - px;
            let dy = ty - py;
            let dist = best_dist_sq.sqrt();

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
                            faction: 0,
                            shooter: -1, // Building, not NPC
                            lifetime: GUARD_POST_PROJ_LIFETIME,
                        });
                    }
                }
            }
            gp_state.timers[i] = GUARD_POST_COOLDOWN;
        }
    }
}
