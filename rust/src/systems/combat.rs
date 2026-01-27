//! Combat systems - Attack processing using GPU targeting results

use godot_bevy::prelude::bevy_ecs_prelude::*;

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
    pub sample_timer: f32,       // Timer value in cooldown_system (before decrement)
    pub cooldown_entities: usize, // Entities with AttackTimer (in cooldown_system)
    pub frame_delta: f32,         // dt used for cooldown
    // Enhanced debug for diagnosing targeting issues
    pub sample_combat_target_0: i32,  // combat_targets[0]
    pub sample_combat_target_5: i32,  // combat_targets[5] (first raider in test 10)
    pub sample_pos_0: (f32, f32),     // position of NPC 0
    pub sample_pos_5: (f32, f32),     // position of NPC 5
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
            sample_combat_target_5: -99,
            sample_pos_0: (0.0, 0.0),
            sample_pos_5: (0.0, 0.0),
        }
    }
}

/// Decrement attack cooldown timers each frame.
/// Also updates debug with pre-cooldown timer state.
pub fn cooldown_system(mut query: Query<&mut AttackTimer>) {
    let dt = FRAME_DELTA.lock().map(|d| *d).unwrap_or(0.016);

    // Debug: capture first timer BEFORE decrement
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

    // Store in debug
    if let Ok(mut debug) = COMBAT_DEBUG.lock() {
        debug.sample_timer = first_timer_before;
        debug.cooldown_entities = timer_count;
        debug.frame_delta = dt;
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

    // Debug tracking
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

        // Get target from GPU (already faction-filtered)
        let target_idx = combat_targets.get(i).copied().unwrap_or(-1);

        if attackers == 1 {
            sample_target = target_idx;
        }

        if target_idx < 0 {
            // No enemy in range - remove InCombat marker
            if in_combat.is_some() {
                commands.entity(entity).remove::<InCombat>();
            }
            continue;
        }

        targets_found += 1;

        // Has target - add InCombat marker
        if in_combat.is_none() {
            commands.entity(entity).insert(InCombat);
            in_combat_added += 1;
        }

        let ti = target_idx as usize;

        // Bounds check
        if i * 2 + 1 >= positions.len() || ti * 2 + 1 >= positions.len() {
            bounds_failures += 1;
            continue;
        }

        // Get positions
        let (x, y) = (positions[i * 2], positions[i * 2 + 1]);
        let (tx, ty) = (positions[ti * 2], positions[ti * 2 + 1]);

        // Calculate distance
        let dx = tx - x;
        let dy = ty - y;
        let dist = (dx * dx + dy * dy).sqrt();

        if attackers == 1 {
            sample_dist = dist;
        }

        if dist <= stats.range {
            // In attack range
            in_range_count += 1;
            if in_range_count == 1 {
                sample_timer = timer.0;
            }
            // Check cooldown timer
            if timer.0 <= 0.0 {
                timer_ready_count += 1;
                // Attack! Queue damage
                if let Ok(mut queue) = DAMAGE_QUEUE.lock() {
                    queue.push(DamageMsg {
                        npc_index: ti,
                        amount: stats.damage,
                    });
                }
                attacks += 1;
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
            chases += 1;
        }
    }

    // Update debug
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
        // Sample combat targets and positions for debugging
        debug.sample_combat_target_0 = combat_targets.get(0).copied().unwrap_or(-99);
        debug.sample_combat_target_5 = combat_targets.get(5).copied().unwrap_or(-99);
        debug.sample_pos_0 = (
            positions.get(0).copied().unwrap_or(-999.0),
            positions.get(1).copied().unwrap_or(-999.0),
        );
        debug.sample_pos_5 = (
            positions.get(10).copied().unwrap_or(-999.0),
            positions.get(11).copied().unwrap_or(-999.0),
        );
    }
}
