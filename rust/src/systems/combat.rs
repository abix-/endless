//! Combat systems - Attack processing using GPU targeting results

use godot_bevy::prelude::bevy_ecs_prelude::*;

use crate::components::*;
use crate::messages::*;

/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(mut query: Query<&mut AttackTimer>) {
    let dt = FRAME_DELTA.lock().map(|d| *d).unwrap_or(0.016);
    for mut timer in query.iter_mut() {
        if timer.0 > 0.0 {
            timer.0 = (timer.0 - dt).max(0.0);
        }
    }
}

/// Process attacks using GPU targeting results.
/// GPU finds nearest enemy, Bevy checks range and applies damage.
/// Adds InCombat marker to NPCs with valid targets.
pub fn attack_system(
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &AttackStats, &mut AttackTimer, &Faction, Option<&InCombat>), Without<Dead>>,
) {
    // Read GPU targeting results from statics (updated by process())
    let combat_targets = match GPU_COMBAT_TARGETS.lock() {
        Ok(t) => t.clone(),
        Err(_) => return,
    };
    let positions = match GPU_POSITIONS.lock() {
        Ok(p) => p.clone(),
        Err(_) => return,
    };

    for (entity, npc_idx, stats, mut timer, _faction, in_combat) in query.iter_mut() {
        let i = npc_idx.0;

        // Get target from GPU (already faction-filtered)
        let target_idx = combat_targets.get(i).copied().unwrap_or(-1);

        if target_idx < 0 {
            // No enemy in range - remove InCombat marker
            if in_combat.is_some() {
                commands.entity(entity).remove::<InCombat>();
            }
            continue;
        }

        // Has target - add InCombat marker
        if in_combat.is_none() {
            commands.entity(entity).insert(InCombat);
        }

        let ti = target_idx as usize;

        // Bounds check
        if i * 2 + 1 >= positions.len() || ti * 2 + 1 >= positions.len() {
            continue;
        }

        // Get positions
        let (x, y) = (positions[i * 2], positions[i * 2 + 1]);
        let (tx, ty) = (positions[ti * 2], positions[ti * 2 + 1]);

        // Calculate distance
        let dx = tx - x;
        let dy = ty - y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= stats.range {
            // In attack range
            if timer.0 <= 0.0 {
                // Attack! Queue damage
                if let Ok(mut queue) = DAMAGE_QUEUE.lock() {
                    queue.push(DamageMsg {
                        npc_index: ti,
                        amount: stats.damage,
                    });
                }
                timer.0 = stats.cooldown;
            }
        } else {
            // Out of range - chase target
            if let Ok(mut queue) = GPU_TARGET_QUEUE.lock() {
                queue.push(SetTargetMsg {
                    npc_index: i,
                    x: tx,
                    y: ty,
                });
            }
        }
    }
}
