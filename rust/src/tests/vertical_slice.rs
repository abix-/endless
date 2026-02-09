//! Vertical Slice Test (relocated Test12)
//! Validates full core loop: spawn → work → raid → combat → death → respawn.
//! 5 farmers + 2 guards + 5 raiders, phased assertions with time gates.

use bevy::prelude::*;
use crate::components::*;
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world;

use super::TestState;

/// Farm positions for the vertical slice test.
const FARMS: [(f32, f32); 5] = [
    (300.0, 350.0), (350.0, 350.0), (400.0, 350.0), (450.0, 350.0), (500.0, 350.0),
];

/// Setup: populate world, spawn NPCs, init resources.
pub fn setup(
    mut slot_alloc: ResMut<SlotAllocator>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut food_storage: ResMut<FoodStorage>,
    mut farm_states: ResMut<FarmStates>,
    mut faction_stats: ResMut<FactionStats>,
    mut game_time: ResMut<GameTime>,
    mut flags: ResMut<DebugFlags>,
    mut test_state: ResMut<TestState>,
) {
    // World data: 2 towns
    world_data.towns.push(world::Town {
        name: "Harvest".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.towns.push(world::Town {
        name: "Raiders".into(),
        center: Vec2::new(400.0, 100.0),
        faction: 1,
        sprite_type: 1,
    });

    // 5 farms near town 0
    for &(fx, fy) in &FARMS {
        world_data.farms.push(world::Farm {
            position: Vec2::new(fx, fy),
            town_idx: 0,
        });
        farm_states.states.push(FarmGrowthState::Ready);
        farm_states.progress.push(1.0);
    }

    // 5 beds near town 0
    for i in 0..5 {
        world_data.beds.push(world::Bed {
            position: Vec2::new(300.0 + (i as f32 * 50.0), 450.0),
            town_idx: 0,
        });
    }

    // 4 guard posts (square patrol around town)
    for (order, &(gx, gy)) in [(250.0, 250.0), (550.0, 250.0), (550.0, 550.0), (250.0, 550.0)].iter().enumerate() {
        world_data.guard_posts.push(world::GuardPost {
            position: Vec2::new(gx, gy),
            town_idx: 0,
            patrol_order: order as u32,
        });
    }

    // Resources
    food_storage.init(2);
    food_storage.food[1] = 10;
    faction_stats.init(2);
    game_time.time_scale = 10.0;

    // Spawn 5 farmers
    for (i, &(fx, fy)) in FARMS.iter().enumerate() {
        let slot = slot_alloc.alloc().expect("slot alloc");
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x: fx, y: fy + 200.0,
            job: 0, faction: 0, town_idx: 0,
            home_x: 300.0 + (i as f32 * 50.0), home_y: 450.0,
            work_x: fx, work_y: fy,
            starting_post: -1,
            attack_type: 0,
        });
    }

    // Spawn 2 guards
    for i in 0..2 {
        let slot = slot_alloc.alloc().expect("slot alloc");
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x: 400.0, y: 400.0,
            job: 1, faction: 0, town_idx: 0,
            home_x: 400.0, home_y: 400.0,
            work_x: -1.0, work_y: -1.0,
            starting_post: i,
            attack_type: 0,
        });
    }

    // Spawn 5 raiders
    for i in 0..5 {
        let slot = slot_alloc.alloc().expect("slot alloc");
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x: 380.0 + (i as f32 * 10.0), y: 100.0,
            job: 2, faction: 1, town_idx: 1,
            home_x: 400.0, home_y: 100.0,
            work_x: -1.0, work_y: -1.0,
            starting_post: -1,
            attack_type: 0,
        });
    }

    flags.combat = true;
    flags.readback = true;

    test_state.phase_name = "Waiting for spawns...".into();
    info!("vertical-slice: setup complete — 5 farmers, 2 guards, 5 raiders");
}

/// Tick: phased assertions with time gates.
pub fn tick(
    time: Res<Time>,
    npc_count: Res<NpcCount>,
    gpu_state: Res<GpuReadState>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    working_query: Query<(), With<Working>>,
    raiding_query: Query<(), With<Raiding>>,
    going_to_work_query: Query<(), With<GoingToWork>>,
    at_dest_query: Query<(), With<AtDestination>>,
    has_target_query: Query<(), With<HasTarget>>,
    mut test: ResMut<TestState>,
) {
    if test.passed || test.failed { return; }

    let now = time.elapsed_secs();
    if test.start == 0.0 { test.start = now; }
    let elapsed = now - test.start;

    // Track lowest NPC count for death detection
    if npc_count.0 > 0 && npc_count.0 < test.count("lowest_npc") as usize {
        test.counters.insert("lowest_npc".into(), npc_count.0 as u32);
    }
    if !test.counters.contains_key("lowest_npc") && npc_count.0 > 0 {
        test.counters.insert("lowest_npc".into(), npc_count.0 as u32);
    }

    match test.phase {
        // Phase 1: All 12 NPCs spawned
        1 => {
            test.phase_name = format!("npc_count={}/12", npc_count.0);
            if npc_count.0 == 12 {
                test.pass_phase(elapsed, format!("npc_count={}", npc_count.0));
            } else if elapsed > 2.0 {
                test.fail_phase(elapsed, format!("npc_count={} (expected 12)", npc_count.0));
            }
        }
        // Phase 2: GPU readback has valid positions
        2 => {
            let has_positions = gpu_state.positions.len() >= 24
                && gpu_state.positions.iter().take(24).any(|&v| v != 0.0);
            test.phase_name = format!("positions_len={}", gpu_state.positions.len());
            if has_positions {
                let p0 = (gpu_state.positions.get(0).copied().unwrap_or(0.0),
                           gpu_state.positions.get(1).copied().unwrap_or(0.0));
                test.pass_phase(elapsed, format!("positions valid, sample=({:.0},{:.0})", p0.0, p0.1));
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("positions_len={}, all zeros", gpu_state.positions.len()));
            }
        }
        // Phase 3: Farmers arrive and start working
        3 => {
            let working = working_query.iter().count();
            let going_to_work = going_to_work_query.iter().count();
            test.phase_name = format!("working={} going_to_work={}", working, going_to_work);
            if working >= 3 {
                test.pass_phase(elapsed, format!("working={}", working));
            } else if elapsed > 8.0 {
                let at_dest = at_dest_query.iter().count();
                let has_target = has_target_query.iter().count();
                test.fail_phase(elapsed, format!(
                    "working={} going_to_work={} at_dest={} has_target={}", working, going_to_work, at_dest, has_target));
            }
        }
        // Phase 4: Raiders dispatched
        4 => {
            let raiding = raiding_query.iter().count();
            test.phase_name = format!("raiding={}/3", raiding);
            if raiding >= 3 {
                test.pass_phase(elapsed, format!("raiding={}", raiding));
            } else if elapsed > 15.0 {
                test.fail_phase(elapsed, format!("raiding={} (expected >=3)", raiding));
            }
        }
        // Phase 5: Combat targets acquired
        5 => {
            test.phase_name = format!("targets_found={}", combat_debug.targets_found);
            if combat_debug.targets_found > 0 {
                test.pass_phase(elapsed, format!("targets_found={}", combat_debug.targets_found));
            } else if elapsed > 25.0 {
                test.fail_phase(elapsed, "targets_found=0");
            }
        }
        // Phase 6: Damage applied
        6 => {
            test.phase_name = format!("damage_processed={}", health_debug.damage_processed);
            if health_debug.damage_processed > 0 {
                test.pass_phase(elapsed, format!("damage_processed={}", health_debug.damage_processed));
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, "damage_processed=0");
            }
        }
        // Phase 7: At least one death
        7 => {
            test.phase_name = format!("npc_count={} deaths_frame={}", npc_count.0, health_debug.deaths_this_frame);
            if npc_count.0 < 12 || health_debug.deaths_this_frame > 0 {
                test.set_flag("death_seen", true);
                test.pass_phase(elapsed, format!("npc_count={}, deaths_frame={}", npc_count.0, health_debug.deaths_this_frame));
            } else if elapsed > 40.0 {
                test.fail_phase(elapsed, format!("npc_count={} (no deaths)", npc_count.0));
            }
        }
        // Phase 8: Respawn
        8 => {
            test.phase_name = format!("npc_count={}/12 (waiting for respawn)", npc_count.0);
            if test.get_flag("death_seen") && npc_count.0 >= 12 {
                test.pass_phase(elapsed, format!("npc_count={} (recovered)", npc_count.0));
                test.complete(elapsed);
            } else if elapsed > 60.0 {
                let lowest = test.count("lowest_npc");
                let death_seen = test.get_flag("death_seen");
                test.fail_phase(elapsed, format!(
                    "npc_count={}, lowest={}, death_seen={}", npc_count.0, lowest, death_seen));
            }
        }
        _ => {}
    }
}
