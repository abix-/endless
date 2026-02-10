//! Economy systems - Game time, population tracking, farm growth, camp foraging, respawning

use bevy::prelude::*;

use crate::components::*;
use crate::resources::*;
use crate::constants::{FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE, CAMP_FORAGE_RATE, RAIDER_SPAWN_COST, CAMP_MAX_POP, STARVING_SPEED_MULT};
use crate::world::{WorldData, FarmOccupancy, pos_to_key};
use crate::messages::{SpawnNpcMsg, GpuUpdate, GpuUpdateMsg};
use crate::systems::stats::{TownUpgrades, UpgradeType, UPGRADE_PCT};

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
// GAME TIME SYSTEM
// ============================================================================

/// Advances game time based on delta and time_scale.
/// Sets hour_ticked = true when the hour changes (for hourly systems).
pub fn game_time_system(
    time: Res<Time>,
    mut game_time: ResMut<GameTime>,
) {
    // Reset tick flag each frame
    game_time.hour_ticked = false;

    if game_time.paused {
        return;
    }

    game_time.total_seconds += time.delta_secs() * game_time.time_scale;

    // Check if hour changed
    let current_hour = game_time.total_hours();
    if current_hour > game_time.last_hour {
        game_time.last_hour = current_hour;
        game_time.hour_ticked = true;
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
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut farm_states: ResMut<FarmStates>,
    world_data: Res<WorldData>,
    farm_occupancy: Res<FarmOccupancy>,
    upgrades: Res<TownUpgrades>,
) {
    if game_time.paused {
        return;
    }

    // Calculate hours elapsed this frame
    let hours_elapsed = (time.delta_secs() * game_time.time_scale) / game_time.seconds_per_hour;

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
        let farm_key = pos_to_key(farm.position);
        let is_tended = farm_occupancy.occupants.get(&farm_key).copied().unwrap_or(0) > 0;

        let base_rate = if is_tended {
            FARM_TENDED_GROWTH_RATE
        } else {
            FARM_BASE_GROWTH_RATE
        };

        // Apply FarmYield upgrade multiplier
        let town = farm.town_idx as usize;
        let yield_level = upgrades.levels.get(town).map(|l| l[UpgradeType::FarmYield as usize]).unwrap_or(0);
        let growth_rate = base_rate * (1.0 + yield_level as f32 * UPGRADE_PCT[UpgradeType::FarmYield as usize]);

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

// ============================================================================
// CAMP FORAGING SYSTEM
// ============================================================================

/// Camp foraging: each raider camp gains CAMP_FORAGE_RATE food per hour.
/// Only runs when game_time.hour_ticked is true.
pub fn camp_forage_system(
    game_time: Res<GameTime>,
    mut food_storage: ResMut<FoodStorage>,
    world_data: Res<WorldData>,
) {
    if !game_time.hour_ticked {
        return;
    }

    // Add foraging food to each raider camp (faction > 0)
    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction > 0 && town_idx < food_storage.food.len() {
            food_storage.food[town_idx] += CAMP_FORAGE_RATE;
        }
    }
}

// ============================================================================
// RAIDER RESPAWN SYSTEM
// ============================================================================

/// Raider respawning: camps spend food to spawn new raiders.
/// Only runs when game_time.hour_ticked is true.
/// Checks: food >= RAIDER_SPAWN_COST, population < CAMP_MAX_POP.
pub fn raider_respawn_system(
    game_time: Res<GameTime>,
    mut food_storage: ResMut<FoodStorage>,
    faction_stats: Res<FactionStats>,
    world_data: Res<WorldData>,
    mut slots: ResMut<SlotAllocator>,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
) {
    if !game_time.hour_ticked {
        return;
    }

    // Check each raider camp (faction > 0)
    for (town_idx, town) in world_data.towns.iter().enumerate() {
        if town.faction <= 0 {
            continue; // Skip villager towns
        }

        // Check food
        let food = food_storage.food.get(town_idx).copied().unwrap_or(0);
        if food < RAIDER_SPAWN_COST {
            continue; // Not enough food
        }

        // Check population cap
        let alive = faction_stats.stats.get(town.faction as usize)
            .map(|s| s.alive)
            .unwrap_or(0);
        if alive >= CAMP_MAX_POP {
            continue; // At capacity
        }

        // Allocate slot
        let slot_idx = match slots.alloc() {
            Some(idx) => idx,
            None => continue, // No slots available
        };

        // Spawn raider at camp center
        spawn_writer.write(SpawnNpcMsg {
            slot_idx,
            x: town.center.x,
            y: town.center.y,
            job: 2, // Raider
            faction: town.faction,
            town_idx: town_idx as i32,
            home_x: town.center.x,
            home_y: town.center.y,
            work_x: -1.0,
            work_y: -1.0,
            starting_post: -1,
            attack_type: 0, // Melee
        });

        // Subtract food cost
        if let Some(f) = food_storage.food.get_mut(town_idx) {
            *f -= RAIDER_SPAWN_COST;
        }
    }
}

// ============================================================================
// STARVATION SYSTEM
// ============================================================================

/// Starvation check: NPCs with zero energy become Starving.
/// Only runs when game_time.hour_ticked is true.
/// Starving NPCs have 50% speed.
pub fn starvation_system(
    mut commands: Commands,
    game_time: Res<GameTime>,
    query: Query<(Entity, &NpcIndex, &Energy, &CachedStats, Option<&Starving>), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    if !game_time.hour_ticked {
        return;
    }

    for (entity, npc_idx, energy, cached, starving) in query.iter() {
        let idx = npc_idx.0;

        if energy.0 <= 0.0 {
            if starving.is_none() {
                commands.entity(entity).insert(Starving);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed * STARVING_SPEED_MULT }));
            }
        } else if starving.is_some() {
            commands.entity(entity).remove::<Starving>();
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed }));
        }
    }
}

// ============================================================================
// FARM VISUAL SYSTEM
// ============================================================================

/// Spawns/despawns FarmReadyMarker entities when farm state transitions.
/// Growing→Ready: spawn marker. Ready→Growing (harvest): despawn marker.
pub fn farm_visual_system(
    mut commands: Commands,
    farm_states: Res<FarmStates>,
    world_data: Res<crate::world::WorldData>,
    markers: Query<(Entity, &FarmReadyMarker)>,
    mut prev_states: Local<Vec<FarmGrowthState>>,
) {
    prev_states.resize(farm_states.states.len(), FarmGrowthState::Growing);
    for (farm_idx, state) in farm_states.states.iter().enumerate() {
        let prev = prev_states[farm_idx];
        if *state == FarmGrowthState::Ready && prev == FarmGrowthState::Growing {
            if world_data.farms.get(farm_idx).is_some() {
                commands.spawn(FarmReadyMarker { farm_idx });
            }
        } else if *state == FarmGrowthState::Growing && prev == FarmGrowthState::Ready {
            for (entity, marker) in markers.iter() {
                if marker.farm_idx == farm_idx {
                    commands.entity(entity).despawn();
                }
            }
        }
        prev_states[farm_idx] = *state;
    }
}
