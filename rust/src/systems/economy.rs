//! Economy systems - Population tracking, food production, respawning

use godot_bevy::prelude::godot_prelude::godot_print;
use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::PhysicsDelta;

use crate::components::*;
use crate::messages::{FOOD_STORAGE, SPAWN_QUEUE, SpawnNpcMsg, FREE_SLOTS, NPC_SLOT_COUNTER};
use crate::resources::*;
use crate::world::{WORLD_DATA, BED_OCCUPANCY, FARM_OCCUPANCY};

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
// SLOT AND POSITION HELPERS
// ============================================================================

/// Allocate a slot for a new NPC (reuse free slots first).
fn allocate_slot() -> Option<usize> {
    if let Ok(mut free) = FREE_SLOTS.lock() {
        if let Some(recycled) = free.pop() {
            return Some(recycled);
        }
    }
    if let Ok(mut counter) = NPC_SLOT_COUNTER.lock() {
        if *counter < MAX_NPC_COUNT {
            let idx = *counter;
            *counter += 1;
            return Some(idx);
        }
    }
    None
}

/// Find a free bed for a given town, returns (bed_index, position).
fn find_free_bed(town_idx: u32) -> Option<(usize, (f32, f32))> {
    let world = WORLD_DATA.lock().ok()?;
    let occupancy = BED_OCCUPANCY.lock().ok()?;

    for (i, bed) in world.beds.iter().enumerate() {
        if bed.town_idx == town_idx {
            if i < occupancy.occupant_npc.len() && occupancy.occupant_npc[i] < 0 {
                return Some((i, (bed.position.x, bed.position.y)));
            }
        }
    }
    None
}

/// Find a farm with lowest occupancy for a given town.
fn find_available_farm(town_idx: u32) -> Option<(f32, f32)> {
    let world = WORLD_DATA.lock().ok()?;
    let occupancy = FARM_OCCUPANCY.lock().ok()?;

    let mut best: Option<(usize, i32, (f32, f32))> = None;
    for (i, farm) in world.farms.iter().enumerate() {
        if farm.town_idx == town_idx {
            let count = occupancy.occupant_count.get(i).copied().unwrap_or(0);
            match &best {
                None => best = Some((i, count, (farm.position.x, farm.position.y))),
                Some((_, best_count, _)) if count < *best_count => {
                    best = Some((i, count, (farm.position.x, farm.position.y)));
                }
                _ => {}
            }
        }
    }
    best.map(|(_, _, pos)| pos)
}

// ============================================================================
// ECONOMY TICK SYSTEM - All hourly logic in one place
// ============================================================================

/// Unified economy system: tracks time, produces food, respawns NPCs.
/// Uses PhysicsDelta (synced with Godot's physics frame by godot-bevy).
pub fn economy_tick_system(
    delta: Res<PhysicsDelta>,
    mut game_time: ResMut<GameTime>,
    working_farmers: Query<&TownId, (With<Farmer>, With<Working>)>,
    pop_stats: Res<PopulationStats>,
    config: Res<GameConfig>,
    mut timers: ResMut<RespawnTimers>,
) {
    // Accumulate elapsed time
    game_time.elapsed_seconds += delta.delta_seconds;

    // Debug: print every ~60 frames
    static mut FRAME_COUNT: u32 = 0;
    unsafe {
        FRAME_COUNT += 1;
        if FRAME_COUNT % 60 == 0 {
            let working_count = working_farmers.iter().count();
            godot_print!("Economy: delta={:.4}, elapsed={:.1}/{:.0}s, working_farmers={}",
                delta.delta_seconds, game_time.elapsed_seconds, game_time.seconds_per_hour, working_count);
        }
    }

    // Check for hour boundary
    if game_time.elapsed_seconds < game_time.seconds_per_hour {
        return;
    }

    // Hour boundary crossed - do hourly tasks
    game_time.elapsed_seconds -= game_time.seconds_per_hour;
    game_time.current_hour = (game_time.current_hour + 1) % 24;

    let working_count = working_farmers.iter().count();
    godot_print!("=== HOUR {} === Working farmers: {}", game_time.current_hour, working_count);

    // --- FOOD PRODUCTION ---
    produce_food(&working_farmers, &config);

    // --- RESPAWN CHECK ---
    check_respawns(&pop_stats, &config, &mut timers);
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
        godot_print!("produce_food: storage len={}, farmers_per_clan={:?}, food_per_hour={}",
            food.food.len(), farmers_per_clan, config.food_per_work_hour);
        for (clan_id, farmer_count) in farmers_per_clan {
            let food_produced = farmer_count * config.food_per_work_hour;
            if clan_id >= 0 && (clan_id as usize) < food.food.len() {
                food.food[clan_id as usize] += food_produced;
                godot_print!("  clan {} += {} food (now {})", clan_id, food_produced, food.food[clan_id as usize]);
            } else {
                godot_print!("  clan {} OUT OF RANGE (len={})", clan_id, food.food.len());
            }
        }
    }
}

/// Check each town for respawns.
fn check_respawns(
    pop_stats: &PopulationStats,
    config: &GameConfig,
    timers: &mut RespawnTimers,
) {
    // Get town count from world data
    let town_count = match WORLD_DATA.lock() {
        Ok(world) => world.towns.len(),
        Err(_) => return,
    };

    for town_idx in 0..town_count {
        let clan_id = town_idx as i32;

        // Check/update timer for this clan
        let timer = timers.0.entry(clan_id).or_insert(0);
        if *timer > 0 {
            *timer -= 1;
            continue;
        }

        // Timer expired, check if we need to spawn
        let farmer_key = (Job::Farmer as i32, clan_id);
        let guard_key = (Job::Guard as i32, clan_id);

        let farmers_alive = pop_stats.0.get(&farmer_key).map(|s| s.alive).unwrap_or(0);
        let guards_alive = pop_stats.0.get(&guard_key).map(|s| s.alive).unwrap_or(0);

        let mut spawned_any = false;

        // Spawn farmer if below cap
        if farmers_alive < config.farmers_per_town {
            if let Some(slot) = allocate_slot() {
                if let Some((_bed_idx, (home_x, home_y))) = find_free_bed(town_idx as u32) {
                    let (work_x, work_y) = find_available_farm(town_idx as u32).unwrap_or((home_x, home_y));
                    let spawn_pos = match WORLD_DATA.lock() {
                        Ok(world) => world.towns.get(town_idx).map(|t| (t.center.x, t.center.y)),
                        Err(_) => None,
                    };
                    if let Some((x, y)) = spawn_pos {
                        if let Ok(mut queue) = SPAWN_QUEUE.lock() {
                            queue.push(SpawnNpcMsg {
                                slot_idx: slot,
                                x, y,
                                job: 0, // Farmer
                                faction: 0, // Villager
                                town_idx: clan_id,
                                home_x, home_y,
                                work_x, work_y,
                                starting_post: -1,
                                attack_type: 0,
                            });
                            spawned_any = true;
                        }
                    }
                }
            }
        }

        // Spawn guard if below cap
        if guards_alive < config.guards_per_town {
            if let Some(slot) = allocate_slot() {
                if let Some((_bed_idx, (home_x, home_y))) = find_free_bed(town_idx as u32) {
                    let spawn_pos = match WORLD_DATA.lock() {
                        Ok(world) => world.towns.get(town_idx).map(|t| (t.center.x, t.center.y)),
                        Err(_) => None,
                    };
                    if let Some((x, y)) = spawn_pos {
                        if let Ok(mut queue) = SPAWN_QUEUE.lock() {
                            queue.push(SpawnNpcMsg {
                                slot_idx: slot,
                                x, y,
                                job: 1, // Guard
                                faction: 0, // Villager
                                town_idx: clan_id,
                                home_x, home_y,
                                work_x: -1.0, work_y: -1.0,
                                starting_post: 0, // First patrol post
                                attack_type: 0,
                            });
                            spawned_any = true;
                        }
                    }
                }
            }
        }

        // Reset timer if we spawned anything
        if spawned_any {
            *timers.0.get_mut(&clan_id).unwrap() = config.spawn_interval_hours;
        }
    }
}
