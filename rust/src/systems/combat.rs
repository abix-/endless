//! Combat systems - Attack processing using GPU targeting results

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::PhysicsDelta;

use crate::components::*;
use crate::messages::*;

/// Debug: track combat system activity
pub static COMBAT_DEBUG: std::sync::Mutex<CombatDebug> = std::sync::Mutex::new(CombatDebug::new());

pub struct CombatDebug {
    pub attackers_queried: usize,
    pub targets_found: usize,
    pub attacks_made: usize,
    pub chases_started: usize,
    pub in_combat_added: usize,
    pub sample_target_idx: i32,
    pub positions_len: usize,
    pub combat_targets_len: usize,
    pub bounds_failures: usize,
    pub sample_dist: f32,
    pub in_range_count: usize,
    pub timer_ready_count: usize,
    pub sample_timer: f32,
    pub cooldown_entities: usize,
    pub frame_delta: f32,
    pub sample_combat_target_0: i32,
    pub sample_combat_target_1: i32,
    pub sample_pos_0: (f32, f32),
    pub sample_pos_1: (f32, f32),
}

impl CombatDebug {
    pub const fn new() -> Self {
        Self {
            attackers_queried: 0,
            targets_found: 0,
            attacks_made: 0,
            chases_started: 0,
            in_combat_added: 0,
            sample_target_idx: -99,
            positions_len: 0,
            combat_targets_len: 0,
            bounds_failures: 0,
            sample_dist: -1.0,
            in_range_count: 0,
            timer_ready_count: 0,
            sample_timer: -1.0,
            cooldown_entities: 0,
            frame_delta: 0.0,
            sample_combat_target_0: -99,
            sample_combat_target_1: -99,
            sample_pos_0: (0.0, 0.0),
            sample_pos_1: (0.0, 0.0),
        }
    }
}

/// Decrement attack cooldown timers each frame.
pub fn cooldown_system(delta: Res<PhysicsDelta>, mut query: Query<&mut AttackTimer>) {
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

    if let Ok(mut debug) = COMBAT_DEBUG.lock() {
        debug.sample_timer = first_timer_before;
        debug.cooldown_entities = timer_count;
        debug.frame_delta = dt;
    }
}

/// Process attacks using GPU targeting results.
/// GPU finds nearest enemy, Bevy checks range and applies damage.
pub fn attack_system(
    mut commands: Commands,
    mut query: Query<(Entity, &NpcIndex, &AttackStats, &mut AttackTimer, &Faction, Option<&InCombat>), Without<Dead>>,
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
                commands.entity(entity).remove::<InCombat>();
            }
            continue;
        }

        targets_found += 1;

        if in_combat.is_none() {
            commands.entity(entity).insert(InCombat);
            in_combat_added += 1;
        }

        let ti = target_idx as usize;

        if i * 2 + 1 >= positions.len() || ti * 2 + 1 >= positions.len() {
            bounds_failures += 1;
            continue;
        }

        let (x, y) = (positions[i * 2], positions[i * 2 + 1]);
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
            // GPU-FIRST: Push to GPU_UPDATE_QUEUE instead of GPU_TARGET_QUEUE
            if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
                queue.push(GpuUpdate::SetTarget { idx: i, x: tx, y: ty });
            }
            chases += 1;
        }
    }

    if let Ok(mut debug) = COMBAT_DEBUG.lock() {
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
}
