//! Combat systems - Attack processing using GPU targeting results

use bevy::prelude::*;
use crate::components::*;
use crate::messages::{GpuUpdate, GpuUpdateMsg, DamageMsg, ProjGpuUpdate, PROJ_GPU_UPDATE_QUEUE, PROJ_HIT_STATE};
use crate::resources::{CombatDebug, GpuReadState, ProjSlotAllocator};
use crate::gpu::ProjBufferWrites;

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
    mut query: Query<(Entity, &NpcIndex, &AttackStats, &mut AttackTimer, &Faction, &mut CombatState), Without<Dead>>,
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

    for (_entity, npc_idx, stats, mut timer, faction, mut combat_state) in query.iter_mut() {
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

        if dist <= stats.range {
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
                                vx: dir_x * stats.projectile_speed,
                                vy: dir_y * stats.projectile_speed,
                                damage: stats.damage,
                                faction: faction.0,
                                shooter: i as i32,
                                lifetime: stats.projectile_lifetime,
                            });
                        }
                    }
                } else {
                    // Point blank — apply damage directly (no projectile needed)
                    damage_events.write(DamageMsg {
                        npc_index: target_idx as usize,
                        amount: stats.damage,
                    });
                }

                attacks += 1;
                timer.0 = stats.cooldown;
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
) {
    if let Ok(mut hits) = PROJ_HIT_STATE.lock() {
        for (slot, hit) in hits.iter().enumerate() {
            let npc_idx = hit[0];
            let processed = hit[1];

            // hit[0] >= 0 means a collision was detected, hit[1] == 0 means not yet processed
            if npc_idx >= 0 && processed == 0 {
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

                // Recycle projectile slot and tell GPU to deactivate
                proj_alloc.free(slot);
                if let Ok(mut queue) = PROJ_GPU_UPDATE_QUEUE.lock() {
                    queue.push(ProjGpuUpdate::Deactivate { idx: slot });
                }
            }
        }
        hits.clear();
    }
}
