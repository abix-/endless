//! Economy systems - Game time, population tracking, farm growth, camp foraging, respawning

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::PhysicsDelta;

use crate::components::*;
use crate::resources::*;
use crate::constants::{FARM_BASE_GROWTH_RATE, FARM_TENDED_GROWTH_RATE, CAMP_FORAGE_RATE, RAIDER_SPAWN_COST, CAMP_MAX_POP, STARVATION_HOURS, STARVING_SPEED_MULT, RAID_GROUP_SIZE};
use crate::world::{WorldData, FarmOccupancy, pos_to_key, find_nearest_location, LocationKind};
use crate::messages::{SpawnNpcMsg, GpuUpdate, GpuUpdateMsg};

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
    delta: Res<PhysicsDelta>,
    mut game_time: ResMut<GameTime>,
) {
    // Reset tick flag each frame
    game_time.hour_ticked = false;

    if game_time.paused {
        return;
    }

    game_time.total_seconds += delta.delta_seconds * game_time.time_scale;

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
        let farm_key = pos_to_key(farm.position);
        let is_tended = farm_occupancy.occupants.get(&farm_key).copied().unwrap_or(0) > 0;

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
// RAID COORDINATOR SYSTEM
// ============================================================================

/// Coordinates group raids: counts available raiders per camp, triggers raids when 5+.
/// Only runs when game_time.hour_ticked is true.
///
/// A raider is "available" if they're at camp (not raiding, returning, or in combat).
/// When 5+ are available and no raid is in progress, picks a target farm.
pub fn raid_coordinator_system(
    game_time: Res<GameTime>,
    mut raid_coordinator: ResMut<RaidCoordinator>,
    world_data: Res<WorldData>,
    // Query for raiders that are idle at camp (not in transit or combat)
    query: Query<
        (&Faction, &Home, &NpcIndex),
        (With<Job>, Without<Dead>, Without<Raiding>, Without<Returning>, Without<InCombat>)
    >,
    gpu_state: Res<GpuReadState>,
) {
    if !game_time.hour_ticked {
        return;
    }

    let positions = &gpu_state.positions;
    const CAMP_RADIUS: f32 = 150.0; // Must be near camp to count as available

    // Count available raiders per faction
    let num_factions = world_data.towns.iter().filter(|t| t.faction > 0).count();
    let mut available_counts: Vec<i32> = vec![0; num_factions];
    let mut sample_positions: Vec<Option<Vector2>> = vec![None; num_factions];

    for (faction, home, npc_idx) in query.iter() {
        if faction.0 <= 0 {
            continue; // Only count raiders (faction > 0)
        }

        let camp_idx = (faction.0 - 1) as usize;
        if camp_idx >= available_counts.len() {
            continue;
        }

        // Check if raider is near their home (camp)
        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() {
            continue;
        }

        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= CAMP_RADIUS {
            available_counts[camp_idx] += 1;
            if sample_positions[camp_idx].is_none() {
                sample_positions[camp_idx] = Some(Vector2::new(x, y));
            }
        }
    }

    // Trigger raids for camps with 5+ available and no current raid
    for camp_idx in 0..num_factions {
        // Skip if raid already in progress
        if camp_idx < raid_coordinator.targets.len()
            && raid_coordinator.targets[camp_idx].is_some()
        {
            continue;
        }

        // Check if enough raiders available
        if available_counts[camp_idx] >= RAID_GROUP_SIZE {
            // Find target farm
            if let Some(raider_pos) = sample_positions[camp_idx] {
                if let Some(farm_pos) = find_nearest_location(raider_pos, &world_data, LocationKind::Farm) {
                    // Initialize coordinator if needed
                    if raid_coordinator.targets.len() <= camp_idx {
                        raid_coordinator.targets.resize(camp_idx + 1, None);
                        raid_coordinator.joined.resize(camp_idx + 1, 0);
                    }

                    // Set raid target
                    raid_coordinator.targets[camp_idx] = Some(farm_pos);
                    raid_coordinator.joined[camp_idx] = 0;
                }
            }
        }
    }
}

// ============================================================================
// STARVATION SYSTEM
// ============================================================================

/// Starvation check: NPCs who haven't eaten in 24+ hours become Starving.
/// Only runs when game_time.hour_ticked is true.
/// Starving NPCs have 75% speed.
pub fn starvation_system(
    mut commands: Commands,
    game_time: Res<GameTime>,
    query: Query<(Entity, &NpcIndex, &LastAteHour, Option<&Starving>), Without<Dead>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    if !game_time.hour_ticked {
        return;
    }

    let current_hour = game_time.total_hours();
    const BASE_SPEED: f32 = 60.0; // Default NPC speed

    for (entity, npc_idx, last_ate, starving) in query.iter() {
        let idx = npc_idx.0;
        let hours_since_ate = current_hour - last_ate.0;

        if hours_since_ate >= STARVATION_HOURS as i32 {
            // Should be starving
            if starving.is_none() {
                commands.entity(entity).insert(Starving);
                // Reduce speed
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: BASE_SPEED * STARVING_SPEED_MULT }));
            }
        } else {
            // Not starving - remove marker if present
            if starving.is_some() {
                commands.entity(entity).remove::<Starving>();
                // Restore speed
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: BASE_SPEED }));
            }
        }
    }
}

