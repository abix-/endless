//! Behavior systems - Unified decision-making and state transitions
//!
//! Key systems:
//! - `arrival_system`: Minimal - marks NPCs as AtDestination, handles proximity delivery
//! - `on_duty_tick_system`: Increments guard wait counters
//! - `decision_system`: Central priority-based decision making for ALL NPCs
//!
//! The decision system is the NPC's "brain" - all decisions flow through it:
//! Priority 0: AtDestination? → Handle arrival transition
//! Priority 1-3: Combat (flee/leash/skip)
//! Priority 4: Recovering + healed? → Resume
//! Priority 5: Working + tired? → Stop work
//! Priority 6: OnDuty + time_to_patrol? → Patrol
//! Priority 7: Resting + rested? → Wake up
//! Priority 8: Idle → Score Eat/Rest/Work/Wander

use bevy::ecs::system::SystemParam;
use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

use crate::components::*;
use crate::messages::{ArrivalMsg, GpuUpdate, GpuUpdateMsg};
use crate::constants::*;
use crate::resources::{FoodEvents, FoodDelivered, FoodConsumed, PopulationStats, GpuReadState, FoodStorage, GameTime, NpcLogCache, FarmStates, FarmGrowthState};
use crate::systems::economy::*;
use crate::world::{WorldData, LocationKind, find_nearest_location, find_location_within_radius, FarmOccupancy, find_farm_index_by_pos, pos_to_key};

// ============================================================================
// SYSTEM PARAM BUNDLES - Logical groupings for scalability
// ============================================================================

/// Farm-related resources
#[derive(SystemParam)]
pub struct FarmParams<'w> {
    pub states: ResMut<'w, FarmStates>,
    pub occupancy: ResMut<'w, FarmOccupancy>,
    pub world: Res<'w, WorldData>,
}

/// Economy resources (food, population tracking)
#[derive(SystemParam)]
pub struct EconomyParams<'w> {
    pub food_storage: ResMut<'w, FoodStorage>,
    pub food_events: ResMut<'w, FoodEvents>,
    pub pop_stats: ResMut<'w, PopulationStats>,
}

/// Combat-related queries
#[derive(SystemParam)]
pub struct CombatParams<'w, 's> {
    pub flee_data: Query<'w, 's, (&'static FleeThreshold, Option<&'static CarryingFood>)>,
    pub leash_data: Query<'w, 's, (&'static LeashRange, &'static CombatOrigin)>,
}

/// NPC state queries (split from main query due to Bevy tuple limit)
#[derive(SystemParam)]
pub struct NpcStateParams<'w, 's> {
    pub transit: Query<'w, 's, (
        Option<&'static Patrolling>,
        Option<&'static GoingToWork>,
        Option<&'static GoingToRest>,
        Option<&'static Wandering>,
        Option<&'static AtDestination>,
        Option<&'static WoundedThreshold>,
    )>,
    pub work: Query<'w, 's, &'static WorkPosition>,
    pub assigned: Query<'w, 's, &'static AssignedFarm>,
    pub patrols: Query<'w, 's, &'static mut PatrolRoute>,
}

// Distinct colors for raider factions (must match spawn.rs)
const RAIDER_COLORS: [(f32, f32, f32); 10] = [
    (0.9, 0.2, 0.2),   // Red
    (0.9, 0.5, 0.1),   // Orange
    (0.8, 0.2, 0.6),   // Magenta
    (0.6, 0.2, 0.8),   // Purple
    (0.9, 0.8, 0.1),   // Yellow
    (0.7, 0.3, 0.2),   // Brown
    (0.9, 0.3, 0.5),   // Pink
    (0.5, 0.1, 0.1),   // Dark red
    (0.8, 0.6, 0.2),   // Gold
    (0.6, 0.1, 0.4),   // Dark magenta
];

fn raider_faction_color(faction: &Faction) -> (f32, f32, f32, f32) {
    let idx = ((faction.0 - 1).max(0) as usize) % RAIDER_COLORS.len();
    let (r, g, b) = RAIDER_COLORS[idx];
    (r, g, b, 1.0)
}

/// Arrival system: Minimal event handler + proximity checks.
///
/// Responsibilities:
/// 1. Read ArrivalMsg events → add AtDestination marker (decision_system handles transition)
/// 2. Proximity-based delivery for Returning raiders (no ArrivalMsg for this)
/// 3. Working farmer drift check + harvest (continuous, not event-based)
///
/// All actual state transitions are handled by decision_system.
pub fn arrival_system(
    mut commands: Commands,
    mut events: MessageReader<ArrivalMsg>,
    // Query for NPCs that can receive arrival events
    arrival_query: Query<(Entity, &NpcIndex), Without<AtDestination>>,
    // Query for Returning NPCs (proximity-based delivery)
    returning_query: Query<(Entity, &NpcIndex, &TownId, &Home, &Faction, Option<&CarryingFood>), With<Returning>>,
    // Query for Working farmers with AssignedFarm (for drift check + harvest)
    working_farmers: Query<(&NpcIndex, &AssignedFarm, &TownId), With<Working>>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut food_storage: ResMut<FoodStorage>,
    mut food_events: ResMut<FoodEvents>,
    world_data: Res<WorldData>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut farm_states: ResMut<FarmStates>,
    mut frame_counter: Local<u32>,
) {
    let positions = &gpu_state.positions;
    const DELIVERY_RADIUS: f32 = 150.0;     // Same as healing radius - deliver when near camp
    const MAX_DRIFT: f32 = 20.0;            // Keep farmers visually on the farm

    // ========================================================================
    // 1. Read ArrivalMsg events → add AtDestination marker
    // ========================================================================
    for event in events.read() {
        for (entity, npc_idx) in arrival_query.iter() {
            if npc_idx.0 == event.npc_index {
                commands.entity(entity).insert(AtDestination);
                break;
            }
        }
    }

    // ========================================================================
    // 2. Proximity-based delivery for Returning (no ArrivalMsg for this)
    // ========================================================================
    for (entity, npc_idx, town, home, faction, carrying) in returning_query.iter() {
        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }

        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();

        if dist <= DELIVERY_RADIUS {
            let mut cmds = commands.entity(entity);
            cmds.remove::<Returning>();

            if carrying.is_some() {
                cmds.remove::<CarryingFood>();
                cmds.remove::<CarriedItem>();
                let (r, g, b, a) = raider_faction_color(faction);
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor { idx, r, g, b, a }));
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetCarriedItem { idx, item_id: CarriedItem::NONE }));
                let camp_idx = town.0 as usize;
                if camp_idx < food_storage.food.len() {
                    food_storage.food[camp_idx] += 1;
                }
                food_events.delivered.push(FoodDelivered { camp_idx: town.0 as u32 });
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Delivered food".into());
            }
        }
    }

    // ========================================================================
    // 3. Working farmer drift check + harvest (throttled: each farmer once per 30 frames)
    // ========================================================================
    *frame_counter = frame_counter.wrapping_add(1);
    let frame_slot = *frame_counter % 30;

    for (npc_idx, assigned, town) in working_farmers.iter() {
        if (npc_idx.0 as u32) % 30 != frame_slot { continue; }

        let farm_pos = assigned.0;  // AssignedFarm now stores position
        let idx = npc_idx.0;
        if idx * 2 + 1 >= positions.len() { continue; }
        let current = Vector2::new(positions[idx * 2], positions[idx * 2 + 1]);

        // If drifted too far, re-target to farm
        if current.distance_to(farm_pos) > MAX_DRIFT {
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx, x: farm_pos.x, y: farm_pos.y
            }));
        }

        // Harvest check: if farm became Ready while working, harvest it
        if let Some(farm_idx) = find_farm_index_by_pos(&world_data.farms, farm_pos) {
            if farm_idx < farm_states.states.len()
                && farm_states.states[farm_idx] == FarmGrowthState::Ready
            {
                let town_idx = town.0 as usize;
                if town_idx < food_storage.food.len() {
                    food_storage.food[town_idx] += 1;
                    food_events.consumed.push(FoodConsumed {
                        location_idx: farm_idx as u32,
                        is_camp: false,
                    });
                }
                farm_states.states[farm_idx] = FarmGrowthState::Growing;
                farm_states.progress[farm_idx] = 0.0;
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested (tending)".into());
            }
        }
    }
}

// ============================================================================
// FLEE / LEASH / RECOVERY SYSTEMS (generic)
// ============================================================================

/// Count nearby enemies and allies for threat assessment.
/// Returns (enemy_count, ally_count) within radius.
fn count_nearby_factions(
    positions: &[f32],
    factions: &[i32],
    health: &[f32],
    my_idx: usize,
    my_faction: i32,
    radius: f32,
) -> (u32, u32) {
    let radius_sq = radius * radius;
    let my_x = positions.get(my_idx * 2).copied().unwrap_or(0.0);
    let my_y = positions.get(my_idx * 2 + 1).copied().unwrap_or(0.0);

    let mut enemies = 0u32;
    let mut allies = 0u32;

    let npc_count = factions.len().min(positions.len() / 2).min(health.len());
    for i in 0..npc_count {
        if i == my_idx { continue; }

        // Skip dead NPCs
        let hp = health.get(i).copied().unwrap_or(0.0);
        if hp <= 0.0 { continue; }

        let x = positions[i * 2];
        let y = positions[i * 2 + 1];
        let dx = x - my_x;
        let dy = y - my_y;
        let dist_sq = dx * dx + dy * dy;

        if dist_sq <= radius_sq {
            let their_faction = factions.get(i).copied().unwrap_or(-1);
            if their_faction == my_faction {
                allies += 1;
            } else if their_faction >= 0 {
                enemies += 1;
            }
        }
    }

    (enemies, allies)
}

// ============================================================================
// DECISION SYSTEM
// ============================================================================

/// Actions an NPC can take.
#[derive(Clone, Copy, Debug, PartialEq)]
#[allow(dead_code)]
enum Action {
    Fight,
    Flee,
    Eat,
    Rest,
    Work,
    Wander,
}

/// Simple deterministic "random" for weighted selection.
fn pseudo_random(seed: usize, frame: usize) -> f32 {
    let x = ((seed.wrapping_mul(1103515245).wrapping_add(frame)) >> 16) & 0x7fff;
    (x as f32) / 32767.0
}

/// Weighted random selection from scored actions.
fn weighted_random(scores: &[(Action, f32)], seed: usize, frame: usize) -> Action {
    let total: f32 = scores.iter().map(|(_, s)| *s).sum();
    if total <= 0.0 {
        return Action::Wander;
    }

    let roll = pseudo_random(seed, frame) * total;
    let mut acc = 0.0;
    for (action, score) in scores {
        acc += score;
        if roll < acc {
            return *action;
        }
    }
    scores.last().map(|(a, _)| *a).unwrap_or(Action::Wander)
}

/// Frame counter for pseudo-random seeding.
static DECISION_FRAME: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Unified decision system: ALL NPC decisions in one place with priority cascade.
/// This is the NPC's "brain" - all decisions and state transitions flow through here.
///
/// Uses SystemParam bundles for scalability - add new features to bundles, not here.
///
/// Priority order (first match wins):
/// 0. AtDestination → Handle arrival transition (Patrolling→OnDuty, GoingToWork→Working, etc.)
/// 1-3. Combat (flee/leash/skip)
/// 4. Recovering + healed? → Resume
/// 5. Working + tired? → Stop work
/// 6. OnDuty + time_to_patrol? → Patrol
/// 7. Resting + rested? → Wake up
/// 8. Idle → Score Eat/Rest/Work/Wander (utility AI)
pub fn decision_system(
    mut commands: Commands,
    // Main query: core NPC data (15 elements - at Bevy tuple limit)
    mut query: Query<
        (Entity, &NpcIndex, &Job, &mut Energy, &Health, &Home, &Personality, &TownId, &Faction,
         Option<&InCombat>, Option<&Recovering>, Option<&Working>, Option<&OnDuty>,
         Option<&Resting>, Option<&Raiding>),
        Without<Dead>
    >,
    // Bundled params (scalable - add to bundles, not here)
    mut npc_states: NpcStateParams,
    combat: CombatParams,
    mut farms: FarmParams,
    mut economy: EconomyParams,
    // Core resources
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
) {
    let frame = DECISION_FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let positions = &gpu_state.positions;
    let factions_buf = &gpu_state.factions;
    let health_buf = &gpu_state.health;

    const THREAT_RADIUS: f32 = 200.0;
    const CHECK_INTERVAL: usize = 30;
    const FARM_ARRIVAL_RADIUS: f32 = 20.0;

    for (entity, npc_idx, job, mut energy, health, home, personality, town_id, faction,
         in_combat, recovering, working, on_duty, resting, raiding) in query.iter_mut()
    {
        let idx = npc_idx.0;

        // Get transit states from bundled query
        let (patrolling, going_work, going_rest, wandering, at_destination, wounded) =
            npc_states.transit.get(entity).unwrap_or((None, None, None, None, None, None));

        // ====================================================================
        // Priority 0: AtDestination → Handle arrival transition
        // ====================================================================
        if at_destination.is_some() {
            commands.entity(entity).remove::<AtDestination>();

            // Handle each transit state's arrival
            if patrolling.is_some() {
                commands.entity(entity)
                    .remove::<Patrolling>()
                    .insert(OnDuty { ticks_waiting: 0 });
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ OnDuty".into());
            } else if going_rest.is_some() {
                commands.entity(entity)
                    .remove::<GoingToRest>()
                    .insert(Resting);
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Resting".into());
            } else if going_work.is_some() {
                // Farmers: find farm at WorkPosition and start working
                if *job == Job::Farmer {
                    let search_pos = npc_states.work.get(entity)
                        .map(|wp| wp.0)
                        .unwrap_or_else(|_| {
                            if idx * 2 + 1 < positions.len() {
                                Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
                            } else {
                                Vector2::new(0.0, 0.0)
                            }
                        });

                    if let Some((farm_idx, farm_pos)) = find_location_within_radius(search_pos, &farms.world, LocationKind::Farm, FARM_ARRIVAL_RADIUS) {
                        // Reserve farm using position key
                        let farm_key = pos_to_key(farm_pos);
                        *farms.occupancy.occupants.entry(farm_key).or_insert(0) += 1;

                        commands.entity(entity)
                            .remove::<GoingToWork>()
                            .insert(Working)
                            .insert(AssignedFarm(farm_pos));  // Store position, not index
                        pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));

                        // Check if farm is ready to harvest
                        if farm_idx < farms.states.states.len()
                            && farms.states.states[farm_idx] == FarmGrowthState::Ready
                        {
                            let town_idx = town_id.0 as usize;
                            if town_idx < economy.food_storage.food.len() {
                                economy.food_storage.food[town_idx] += 1;
                                economy.food_events.consumed.push(FoodConsumed {
                                    location_idx: farm_idx as u32,
                                    is_camp: false,
                                });
                            }
                            farms.states.states[farm_idx] = FarmGrowthState::Growing;
                            farms.states.progress[farm_idx] = 0.0;
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Harvested → Working".into());
                        } else {
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (tending)".into());
                        }
                    } else {
                        // No farm found - just work at current position
                        commands.entity(entity)
                            .remove::<GoingToWork>()
                            .insert(Working);
                        pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: search_pos.x, y: search_pos.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working (no farm)".into());
                    }
                } else {
                    // Non-farmers just transition to Working
                    let current_pos = if idx * 2 + 1 < positions.len() {
                        Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
                    } else {
                        Vector2::new(0.0, 0.0)
                    };
                    commands.entity(entity)
                        .remove::<GoingToWork>()
                        .insert(Working);
                    pop_inc_working(&mut economy.pop_stats, *job, town_id.0);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: current_pos.x, y: current_pos.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Working".into());
                }
            } else if raiding.is_some() {
                // Raider arrived at farm - check if ready to steal
                if idx * 2 + 1 < positions.len() {
                    let pos = Vector2::new(positions[idx * 2], positions[idx * 2 + 1]);

                    let ready_farm = find_location_within_radius(pos, &farms.world, LocationKind::Farm, FARM_ARRIVAL_RADIUS)
                        .filter(|(farm_idx, _)| {
                            *farm_idx < farms.states.states.len()
                                && farms.states.states[*farm_idx] == FarmGrowthState::Ready
                        });

                    if let Some((farm_idx, _)) = ready_farm {
                        // Steal food and head home
                        farms.states.states[farm_idx] = FarmGrowthState::Growing;
                        farms.states.progress[farm_idx] = 0.0;

                        commands.entity(entity)
                            .remove::<Raiding>()
                            .insert(CarryingFood)
                            .insert(CarriedItem(CarriedItem::FOOD))
                            .insert(Returning);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetCarriedItem { idx, item_id: CarriedItem::FOOD }));
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Stole food → Returning".into());
                    } else {
                        // Farm not ready - find another
                        if let Some(farm_pos) = find_nearest_location(pos, &farms.world, LocationKind::Farm) {
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));
                            npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Farm not ready, seeking another".into());
                        }
                    }
                }
            } else if wandering.is_some() {
                commands.entity(entity).remove::<Wandering>();
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Idle".into());
            }

            // Check wounded threshold on arrival
            if let Some(w) = wounded {
                let health_pct = health.0 / 100.0;
                if health_pct < w.pct {
                    commands.entity(entity)
                        .insert(Recovering { threshold: 0.75 })
                        .insert(Resting);
                }
            }

            continue;
        }

        // ====================================================================
        // Skip NPCs in transit states (they're walking to their destination)
        // ====================================================================
        if patrolling.is_some() || going_work.is_some() || going_rest.is_some() || wandering.is_some() {
            continue;
        }

        // ====================================================================
        // Priority 1-3: Combat decisions (flee/leash/skip)
        // ====================================================================
        if in_combat.is_some() {
            // Priority 1: Should flee?
            if let Ok((flee_threshold, carrying)) = combat.flee_data.get(entity) {
                let should_check_threat = (frame + idx) % CHECK_INTERVAL == 0;
                let effective_threshold = if should_check_threat {
                    let (enemies, allies) = count_nearby_factions(
                        positions, factions_buf, health_buf, idx, faction.0, THREAT_RADIUS
                    );
                    let ratio = (enemies as f32 + 1.0) / (allies as f32 + 1.0);
                    (flee_threshold.pct * ratio).min(1.0)
                } else {
                    flee_threshold.pct
                };

                if health.0 / 100.0 < effective_threshold {
                    let mut cmds = commands.entity(entity);
                    cmds.remove::<InCombat>().remove::<CombatOrigin>().remove::<Raiding>().insert(Returning);
                    if carrying.is_some() {
                        cmds.remove::<CarryingFood>().remove::<CarriedItem>();
                        let (r, g, b, a) = raider_faction_color(faction);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetColor { idx, r, g, b, a }));
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetCarriedItem { idx, item_id: CarriedItem::NONE }));
                    }
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Fled combat".into());
                    continue;
                }
            }

            // Priority 2: Should leash?
            if let Ok((leash, origin)) = combat.leash_data.get(entity) {
                if idx * 2 + 1 < positions.len() {
                    let dx = positions[idx * 2] - origin.x;
                    let dy = positions[idx * 2 + 1] - origin.y;
                    if (dx * dx + dy * dy).sqrt() > leash.distance {
                        commands.entity(entity)
                            .remove::<InCombat>().remove::<CombatOrigin>().remove::<Raiding>()
                            .insert(Returning);
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Leashed → Returning".into());
                        continue;
                    }
                }
            }

            // Priority 3: Still in combat, attack_system handles targeting
            continue;
        }

        // ====================================================================
        // Priority 4: Recovering + healed?
        // ====================================================================
        if let Some(rec) = recovering {
            if health.0 / 100.0 >= rec.threshold {
                commands.entity(entity).remove::<Recovering>();
                if resting.is_some() { commands.entity(entity).remove::<Resting>(); }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Recovered".into());
            }
            continue;
        }

        // ====================================================================
        // Priority 5: Working + tired?
        // ====================================================================
        if working.is_some() {
            if energy.0 < ENERGY_TIRED_THRESHOLD {
                commands.entity(entity).remove::<Working>();
                if let Ok(assigned) = npc_states.assigned.get(entity) {
                    // Release farm using position key
                    let farm_key = pos_to_key(assigned.0);
                    if let Some(count) = farms.occupancy.occupants.get_mut(&farm_key) {
                        *count = count.saturating_sub(1);
                    }
                    commands.entity(entity).remove::<AssignedFarm>();
                }
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Tired → Stopped".into());
            }
            continue;
        }

        // ====================================================================
        // Priority 6: OnDuty + time to patrol?
        // ====================================================================
        if let Some(duty) = on_duty {
            // Note: We need mutable access to increment ticks, but we have immutable here.
            // The tick increment happens via the mutable query below.
            if duty.ticks_waiting >= GUARD_PATROL_WAIT {
                if let Ok(mut patrol) = npc_states.patrols.get_mut(entity) {
                    if !patrol.posts.is_empty() {
                        patrol.current = (patrol.current + 1) % patrol.posts.len();
                    }
                    commands.entity(entity).remove::<OnDuty>().insert(Patrolling);
                    if let Some(pos) = patrol.posts.get(patrol.current) {
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                    }
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "→ Patrolling".into());
                }
            }
            continue;
        }

        // ====================================================================
        // Priority 7: Resting + rested?
        // ====================================================================
        if resting.is_some() {
            if energy.0 >= ENERGY_WAKE_THRESHOLD {
                commands.entity(entity).remove::<Resting>();
                npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(), "Woke up".into());
                // Fall through to make a decision
            } else {
                continue;
            }
        }

        // ====================================================================
        // Priority 8: Idle → Score Eat/Rest/Work/Wander
        // ====================================================================
        let en = energy.0;
        let (_fight_m, _flee_m, rest_m, eat_m, work_m, wander_m) = personality.get_multipliers();

        let town_idx = town_id.0 as usize;
        let food_available = town_idx < economy.food_storage.food.len() && economy.food_storage.food[town_idx] > 0;
        let mut scores: Vec<(Action, f32)> = Vec::with_capacity(4);

        if food_available {
            let eat_score = (100.0 - en) * SCORE_EAT_MULT * eat_m;
            if eat_score > 0.0 { scores.push((Action::Eat, eat_score)); }
        }

        let rest_score = (100.0 - en) * SCORE_REST_MULT * rest_m;
        if rest_score > 0.0 && home.is_valid() { scores.push((Action::Rest, rest_score)); }

        let can_work = match job {
            Job::Farmer => npc_states.work.get(entity).is_ok(),
            Job::Guard => npc_states.patrols.get(entity).is_ok(),
            Job::Raider => true,
            Job::Fighter => false,
        };
        if can_work {
            let hp_pct = health.0 / 100.0;
            let hp_mult = if hp_pct < 0.5 { 0.0 } else { (hp_pct - 0.5) * 2.0 };
            let work_score = SCORE_WORK_BASE * work_m * hp_mult;
            if work_score > 0.0 { scores.push((Action::Work, work_score)); }
        }

        scores.push((Action::Wander, SCORE_WANDER_BASE * wander_m));

        let action = weighted_random(&scores, idx, frame);
        npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
            format!("{:?} (e:{:.0} h:{:.0})", action, energy.0, health.0));

        match action {
            Action::Eat => {
                if town_idx < economy.food_storage.food.len() && economy.food_storage.food[town_idx] > 0 {
                    let old_energy = energy.0;
                    economy.food_storage.food[town_idx] -= 1;
                    energy.0 = (energy.0 + ENERGY_FROM_EATING).min(100.0);
                    npc_logs.push(idx, game_time.day(), game_time.hour(), game_time.minute(),
                        format!("Ate (e:{:.0}→{:.0})", old_energy, energy.0));
                }
            }
            Action::Rest => {
                if home.is_valid() {
                    commands.entity(entity).insert(GoingToRest);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: home.0.x, y: home.0.y }));
                }
            }
            Action::Work => {
                match job {
                    Job::Farmer => {
                        if let Ok(wp) = npc_states.work.get(entity) {
                            commands.entity(entity).insert(GoingToWork);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: wp.0.x, y: wp.0.y }));
                        }
                    }
                    Job::Guard => {
                        if let Ok(patrol) = npc_states.patrols.get(entity) {
                            commands.entity(entity).insert(Patrolling);
                            if let Some(pos) = patrol.posts.get(patrol.current) {
                                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: pos.x, y: pos.y }));
                            }
                        }
                    }
                    Job::Raider => {
                        let pos = if idx * 2 + 1 < positions.len() {
                            Vector2::new(positions[idx * 2], positions[idx * 2 + 1])
                        } else {
                            home.0
                        };
                        if let Some(farm_pos) = find_nearest_location(pos, &farms.world, LocationKind::Farm) {
                            commands.entity(entity).insert(Raiding);
                            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: farm_pos.x, y: farm_pos.y }));
                        }
                    }
                    Job::Fighter => {}
                }
            }
            Action::Wander => {
                if idx * 2 + 1 < positions.len() {
                    let x = positions[idx * 2];
                    let y = positions[idx * 2 + 1];
                    let offset_x = (pseudo_random(idx, frame + 1) - 0.5) * 200.0;
                    let offset_y = (pseudo_random(idx, frame + 2) - 0.5) * 200.0;
                    commands.entity(entity).insert(Wandering);
                    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: x + offset_x, y: y + offset_y }));
                }
            }
            Action::Fight | Action::Flee => {}
        }
    }
}

/// Increment OnDuty tick counters (runs every frame for guards at posts).
/// Separated from decision_system because we need mutable OnDuty access.
pub fn on_duty_tick_system(
    mut query: Query<&mut OnDuty, Without<InCombat>>,
) {
    for mut duty in query.iter_mut() {
        duty.ticks_waiting += 1;
    }
}
