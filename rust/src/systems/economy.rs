//! Economy systems - Population tracking, food production, respawning

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::PhysicsDelta;

use crate::components::*;
#[allow(unused_imports)]
use crate::messages::{FOOD_STORAGE, SPAWN_QUEUE, SpawnNpcMsg, FREE_SLOTS, NPC_SLOT_COUNTER};
use crate::resources::*;
// WorldData, BedOccupancy, FarmOccupancy available via Resources when needed

// Respawn system constants (disabled but kept for later)
#[allow(dead_code)]
const MAX_NPC_COUNT: usize = 10_000;

// ============================================================================
// POPULATION TRACKING HELPERS
// ============================================================================

/// Increment alive count for a (job, clan) pair.
pub fn pop_inc_alive(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    stats.0.entry(key).or_default().alive += 1;
}

/// Decrement alive count for a (job, clan) pair.
pub fn pop_dec_alive(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    if let Some(entry) = stats.0.get_mut(&key) {
        entry.alive = (entry.alive - 1).max(0);
    }
}

/// Increment working count for a (job, clan) pair.
pub fn pop_inc_working(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    stats.0.entry(key).or_default().working += 1;
}

/// Decrement working count for a (job, clan) pair.
pub fn pop_dec_working(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    if let Some(entry) = stats.0.get_mut(&key) {
        entry.working = (entry.working - 1).max(0);
    }
}

// ============================================================================
// ECONOMY TICK SYSTEM - All hourly logic in one place
// ============================================================================

/// Unified economy system: tracks time, produces food, respawns NPCs.
/// Uses PhysicsDelta (synced with Godot's physics frame by godot-bevy).
pub fn economy_tick_system(
    delta: Res<PhysicsDelta>,
    mut game_time: ResMut<GameTime>,
    mut prev_hour: Local<i32>,
    working_farmers: Query<&TownId, (With<Farmer>, With<Working>)>,
    _pop_stats: Res<PopulationStats>,
    config: Res<GameConfig>,
    _timers: ResMut<RespawnTimers>,
) {
    // Respect pause
    if game_time.paused {
        return;
    }

    // Accumulate time (scaled)
    game_time.total_seconds += delta.delta_seconds * game_time.time_scale;

    // Check for hour boundary
    let current_hour = game_time.hour();
    if current_hour == *prev_hour {
        return;
    }
    *prev_hour = current_hour;

    // --- HOURLY TASKS ---
    produce_food(&working_farmers, &config);

    // --- RESPAWN CHECK --- (disabled, keeping code for later)
    // check_respawns(&_pop_stats, &config, &mut _timers);
}

/// Produce food based on working farmers.
fn produce_food(
    working_farmers: &Query<&TownId, (With<Farmer>, With<Working>)>,
    config: &GameConfig,
) {
    // Count working farmers per clan
    let mut farmers_per_clan: std::collections::HashMap<i32, i32> = std::collections::HashMap::new();
    for clan in working_farmers.iter() {
        *farmers_per_clan.entry(clan.0).or_insert(0) += 1;
    }

    // Add food to each clan's storage
    if let Ok(mut food) = FOOD_STORAGE.lock() {
        for (clan_id, farmer_count) in farmers_per_clan {
            let food_produced = farmer_count * config.food_per_work_hour;
            if clan_id >= 0 && (clan_id as usize) < food.food.len() {
                food.food[clan_id as usize] += food_produced;
            }
        }
    }
}

