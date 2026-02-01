//! Economy systems - Population tracking, food production, respawning

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::PhysicsDelta;

use crate::components::*;
use crate::resources::*;
use crate::constants::{FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE};
use crate::world::{WorldData, FarmOccupancy};

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

/// Increment dead count for a (job, clan) pair.
pub fn pop_inc_dead(stats: &mut PopulationStats, job: Job, clan: i32) {
    let key = (job as i32, clan);
    stats.0.entry(key).or_default().dead += 1;
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
    mut food_storage: ResMut<FoodStorage>,
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
    // Count working farmers per clan
    let mut farmers_per_clan: std::collections::HashMap<i32, i32> = std::collections::HashMap::new();
    for clan in working_farmers.iter() {
        *farmers_per_clan.entry(clan.0).or_insert(0) += 1;
    }

    // Add food to each clan's storage
    for (clan_id, farmer_count) in farmers_per_clan {
        let food_produced = farmer_count * config.food_per_work_hour;
        if clan_id >= 0 && (clan_id as usize) < food_storage.food.len() {
            food_storage.food[clan_id as usize] += food_produced;
        }
    }
}

// ============================================================================
// FARM GROWTH SYSTEM
// ============================================================================

/// Farm growth system: advances crop progress based on time and farmer presence.
/// - Passive growth: FARM_BASE_GROWTH_RATE per game hour (~12 hours to full)
/// - Tended growth: FARM_TENDED_GROWTH_RATE per game hour (~4 hours to full)
/// When progress >= 1.0, farm transitions to Ready state.
pub fn farm_growth_system(
    delta: Res<PhysicsDelta>,
    game_time: Res<GameTime>,
    mut farm_states: ResMut<FarmStates>,
    world_data: Res<WorldData>,
    farm_occupancy: Res<FarmOccupancy>,
) {
    if game_time.paused {
        return;
    }

    // Calculate hours elapsed this frame
    let hours_elapsed = (delta.delta_seconds * game_time.time_scale) / game_time.seconds_per_hour;

    for (farm_idx, farm) in world_data.farms.iter().enumerate() {
        // Skip if farm_states not initialized for this farm
        if farm_idx >= farm_states.states.len() {
            continue;
        }

        // Only grow farms that are in Growing state
        if farm_states.states[farm_idx] != FarmGrowthState::Growing {
            continue;
        }

        // Determine growth rate based on whether a farmer is working this farm
        let is_tended = farm_idx < farm_occupancy.occupant_count.len()
            && farm_occupancy.occupant_count[farm_idx] > 0;

        let growth_rate = if is_tended {
            FARM_TENDED_GROWTH_RATE
        } else {
            FARM_BASE_GROWTH_RATE
        };

        // Advance growth progress
        farm_states.progress[farm_idx] += growth_rate * hours_elapsed;

        // Transition to Ready when fully grown
        if farm_states.progress[farm_idx] >= 1.0 {
            farm_states.states[farm_idx] = FarmGrowthState::Ready;
            farm_states.progress[farm_idx] = 1.0; // Clamp at max
        }

        let _ = farm; // Silence unused warning
    }
}

