//! Combat systems - Attack processing using GPU targeting results

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::PhysicsDelta;

use crate::components::*;
use crate::messages::{GPU_READ_STATE, GpuUpdate, GpuUpdateMsg, PROJECTILE_FIRE_QUEUE, FireProjectileMsg};
use crate::resources::CombatDebug;

/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(
    delta: Res<PhysicsDelta>,
    mut query: Query<&mut AttackTimer>,
    mut debug: ResMut<CombatDebug>,
) {
    let dt = delta.delta_seconds;

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
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &AttackStats, &mut AttackTimer, &Faction, Option<&InCombat>), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut debug: ResMut<CombatDebug>,
) {
    // GPU-FIRST: Read from single GpuReadState instead of scattered statics
    let (positions, combat_targets, _npc_count) = {
        match GPU_READ_STATE.lock() {
            Ok(state) => (state.positions.clone(), state.combat_targets.clone(), state.npc_count),
            Err(_) => return,
        }
    };

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

    for (entity, npc_idx, stats, mut timer, _faction, in_combat) in query.iter_mut() {
        attackers += 1;
        let i = npc_idx.0;

        let target_idx = combat_targets.get(i).copied().unwrap_or(-1);

        if attackers == 1 {
            sample_target = target_idx;
        }

        if target_idx < 0 {
            if in_combat.is_some() {
                commands.entity(entity)
                    .remove::<InCombat>()
                    .remove::<CombatOrigin>();
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

        if in_combat.is_none() {
            commands.entity(entity)
                .insert(InCombat)
                .insert(CombatOrigin { x, y });  // Store where combat started
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
                // Attack! Fire projectile (melee = fast projectile, ranged = slow projectile)
                if let Ok(mut queue) = PROJECTILE_FIRE_QUEUE.lock() {
                    queue.push(FireProjectileMsg {
                        from_x: x,
                        from_y: y,
                        to_x: tx,
                        to_y: ty,
                        damage: stats.damage,
                        faction: _faction.to_i32(),
                        shooter: i,
                        speed: stats.projectile_speed,
                        lifetime: stats.projectile_lifetime,
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
