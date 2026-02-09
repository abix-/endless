//! Endless ECS - Pure Bevy colony simulation.
//! See docs/ for architecture documentation.

// ============================================================================
// MODULES
// ============================================================================

pub mod components;
pub mod constants;
pub mod gpu;
pub mod messages;
pub mod npc_render;
pub mod render;
pub mod resources;
pub mod systems;
pub mod world;

// ============================================================================
// IMPORTS
// ============================================================================

use bevy::prelude::*;

// Explicit imports to avoid ambiguity between messages and resources
use messages::{SpawnNpcMsg, SetTargetMsg, ArrivalMsg, DamageMsg, GpuUpdateMsg};
use resources::{
    NpcCount, NpcEntityMap, PopulationStats, GameConfig, GameTime, RespawnTimers,
    FarmStates, FarmGrowthState, HealthDebug, CombatDebug, KillStats, SelectedNpc,
    NpcMetaCache, NpcEnergyCache, NpcsByTownCache, NpcLogCache, FoodEvents,
    ResetFlag, GpuReadState, GpuDispatchCount, SlotAllocator, ProjSlotAllocator,
    FoodStorage, FactionStats, CampState, RaidQueue, BevyFrameTimer, PERF_STATS,
    DebugFlags, Test12,
};
// Systems are re-exported via glob from systems/mod.rs
use systems::*;
use components::*;

// ============================================================================
// HELPERS
// ============================================================================

/// Derive NPC state name from ECS components.
pub fn derive_npc_state(world: &World, entity: Entity) -> &'static str {
    if world.get::<Dead>(entity).is_some() { return "Dead"; }
    if world.get::<InCombat>(entity).is_some() { return "Fighting"; }
    if world.get::<Recovering>(entity).is_some() { return "Recovering"; }
    if world.get::<Resting>(entity).is_some() { return "Resting"; }
    if world.get::<Working>(entity).is_some() { return "Working"; }
    if world.get::<OnDuty>(entity).is_some() { return "On Duty"; }
    if world.get::<Patrolling>(entity).is_some() { return "Patrolling"; }
    if world.get::<GoingToRest>(entity).is_some() { return "Going to Rest"; }
    if world.get::<GoingToWork>(entity).is_some() { return "Going to Work"; }
    if world.get::<Raiding>(entity).is_some() { return "Raiding"; }
    if world.get::<Returning>(entity).is_some() { return "Returning"; }
    if world.get::<Wandering>(entity).is_some() { return "Wandering"; }
    "Idle"
}

/// Get job name from job ID.
pub fn job_name(job: i32) -> &'static str {
    match job {
        0 => "Farmer",
        1 => "Guard",
        2 => "Raider",
        3 => "Fighter",
        _ => "Unknown",
    }
}

/// Get trait name from trait ID.
pub fn trait_name(trait_id: i32) -> &'static str {
    match trait_id {
        0 => "",
        1 => "Brave",
        2 => "Coward",
        3 => "Efficient",
        4 => "Hardy",
        5 => "Lazy",
        6 => "Strong",
        7 => "Swift",
        8 => "Sharpshot",
        9 => "Berserker",
        _ => "",
    }
}

// ============================================================================
// BEVY APP
// ============================================================================

/// System execution phases.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    Drain,
    Spawn,
    Combat,
    Behavior,
}

fn bevy_timer_start(mut timer: ResMut<BevyFrameTimer>) {
    timer.start = Some(std::time::Instant::now());
}

fn startup_system() {
    info!("Endless ECS initialized - systems registered");
}

// ============================================================================
// TEST 12: VERTICAL SLICE INTEGRATION TEST
// ============================================================================
// Validates full core loop: spawn → work → raid → combat → death → respawn
// 5 farmers + 2 guards + 5 raiders, phased assertions with time gates.

/// Farm positions for test 12 (5 farms near villager town).
const TEST12_FARMS: [(f32, f32); 5] = [
    (300.0, 350.0), (350.0, 350.0), (400.0, 350.0), (450.0, 350.0), (500.0, 350.0),
];

/// Startup system: populate world, spawn NPCs, init resources.
fn test12_setup(
    mut slot_alloc: ResMut<SlotAllocator>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut food_storage: ResMut<FoodStorage>,
    mut farm_states: ResMut<FarmStates>,
    mut faction_stats: ResMut<FactionStats>,
    mut game_time: ResMut<GameTime>,
    mut flags: ResMut<DebugFlags>,
) {
    // --- World data ---
    // Town 0: Villagers
    world_data.towns.push(world::Town {
        name: "Harvest".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    // Town 1: Raider camp
    world_data.towns.push(world::Town {
        name: "Raiders".into(),
        center: Vec2::new(400.0, 100.0),
        faction: 1,
        sprite_type: 1,
    });

    // 5 farms near town 0
    for &(fx, fy) in &TEST12_FARMS {
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

    // --- Resources ---
    food_storage.init(2);
    food_storage.food[1] = 10; // Camp starts with 10 food (2 respawns worth)
    faction_stats.init(2);
    game_time.time_scale = 10.0; // 1 game hour = 0.5 real seconds

    // --- Spawn 5 farmers ---
    for (i, &(fx, fy)) in TEST12_FARMS.iter().enumerate() {
        let slot = slot_alloc.alloc().expect("slot alloc");
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x: fx, y: fy + 200.0, // 200px south of farm — must walk to arrive
            job: 0, // Farmer
            faction: 0,
            town_idx: 0,
            home_x: 300.0 + (i as f32 * 50.0), home_y: 450.0,
            work_x: fx, work_y: fy,
            starting_post: -1,
            attack_type: 0,
        });
    }

    // --- Spawn 2 guards ---
    for i in 0..2 {
        let slot = slot_alloc.alloc().expect("slot alloc");
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x: 400.0, y: 400.0, // Start at town center
            job: 1, // Guard
            faction: 0,
            town_idx: 0,
            home_x: 400.0, home_y: 400.0,
            work_x: -1.0, work_y: -1.0,
            starting_post: i,
            attack_type: 0,
        });
    }

    // --- Spawn 5 raiders ---
    for i in 0..5 {
        let slot = slot_alloc.alloc().expect("slot alloc");
        spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x: 380.0 + (i as f32 * 10.0), y: 100.0, // Near camp
            job: 2, // Raider
            faction: 1,
            town_idx: 1,
            home_x: 400.0, home_y: 100.0,
            work_x: -1.0, work_y: -1.0,
            starting_post: -1,
            attack_type: 0,
        });
    }

    flags.combat = true;
    flags.readback = true;
    info!("Test12: Setup complete — 5 farmers, 2 guards, 5 raiders");
}

/// Update system: phased assertions with time gates.
fn test12_tick(
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
    mut test: ResMut<Test12>,
) {
    if test.passed || test.failed {
        return;
    }

    let now = time.elapsed_secs();
    if test.start == 0.0 {
        test.start = now;
    }
    let elapsed = now - test.start;

    // Track lowest NPC count for death detection
    if npc_count.0 > 0 && npc_count.0 < test.lowest_npc_count {
        test.lowest_npc_count = npc_count.0;
    }

    match test.phase {
        // Phase 1: All 12 NPCs spawned
        1 => {
            if npc_count.0 == 12 {
                let msg = format!("npc_count={}", npc_count.0);
                info!("Test12 Phase 1: PASS — {}", msg);
                test.results.push((1, elapsed, msg));
                test.phase = 2;
            } else if elapsed > 2.0 {
                let msg = format!("npc_count={} (expected 12)", npc_count.0);
                info!("Test12 Phase 1: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((1, elapsed, msg));
                test.failed = true;
            }
        }
        // Phase 2: GPU readback has valid positions
        2 => {
            let has_positions = gpu_state.positions.len() >= 24
                && gpu_state.positions.iter().take(24).any(|&v| v != 0.0);
            if has_positions {
                let p0 = (gpu_state.positions.get(0).copied().unwrap_or(0.0),
                           gpu_state.positions.get(1).copied().unwrap_or(0.0));
                let msg = format!("positions_len={}, sample=({:.0},{:.0})", gpu_state.positions.len(), p0.0, p0.1);
                info!("Test12 Phase 2: PASS — {}", msg);
                test.results.push((2, elapsed, msg));
                test.phase = 3;
            } else if elapsed > 3.0 {
                let msg = format!("positions_len={}, all zeros", gpu_state.positions.len());
                info!("Test12 Phase 2: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((2, elapsed, msg));
                test.failed = true;
            }
        }
        // Phase 3: Farmers arrive at farms and start working
        3 => {
            let working = working_query.iter().count();
            let going_to_work = going_to_work_query.iter().count();
            let at_dest = at_dest_query.iter().count();
            let has_target = has_target_query.iter().count();
            if working >= 3 {
                let msg = format!("working={}", working);
                info!("Test12 Phase 3: PASS — {}", msg);
                test.results.push((3, elapsed, msg));
                test.phase = 4;
            } else if elapsed > 8.0 {
                let msg = format!("working={} going_to_work={} at_dest={} has_target={} (expected working>=3)", working, going_to_work, at_dest, has_target);
                info!("Test12 Phase 3: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((3, elapsed, msg));
                test.failed = true;
            }
        }
        // Phase 4: Raiders form group and get dispatched
        4 => {
            let raiding = raiding_query.iter().count();
            if raiding >= 3 {
                let msg = format!("raiding={}", raiding);
                info!("Test12 Phase 4: PASS — {}", msg);
                test.results.push((4, elapsed, msg));
                test.phase = 5;
            } else if elapsed > 15.0 {
                let msg = format!("raiding={} (expected >=3)", raiding);
                info!("Test12 Phase 4: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((4, elapsed, msg));
                test.failed = true;
            }
        }
        // Phase 5: Guards acquire combat targets
        5 => {
            if combat_debug.targets_found > 0 {
                let msg = format!("targets_found={}", combat_debug.targets_found);
                info!("Test12 Phase 5: PASS — {}", msg);
                test.results.push((5, elapsed, msg));
                test.phase = 6;
            } else if elapsed > 25.0 {
                let msg = format!("targets_found=0");
                info!("Test12 Phase 5: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((5, elapsed, msg));
                test.failed = true;
            }
        }
        // Phase 6: Damage applied
        6 => {
            if health_debug.damage_processed > 0 {
                let msg = format!("damage_processed={}", health_debug.damage_processed);
                info!("Test12 Phase 6: PASS — {}", msg);
                test.results.push((6, elapsed, msg));
                test.phase = 7;
            } else if elapsed > 30.0 {
                let msg = format!("damage_processed=0");
                info!("Test12 Phase 6: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((6, elapsed, msg));
                test.failed = true;
            }
        }
        // Phase 7: At least one death
        7 => {
            if npc_count.0 < 12 || health_debug.deaths_this_frame > 0 {
                test.death_seen = true;
                let msg = format!("npc_count={}, deaths_frame={}", npc_count.0, health_debug.deaths_this_frame);
                info!("Test12 Phase 7: PASS — {}", msg);
                test.results.push((7, elapsed, msg));
                test.phase = 8;
            } else if elapsed > 40.0 {
                let msg = format!("npc_count={} (no deaths)", npc_count.0);
                info!("Test12 Phase 7: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((7, elapsed, msg));
                test.failed = true;
            }
        }
        // Phase 8: Respawn — NPC count recovers after death
        8 => {
            // Wait for respawn: count should go back up (camp has food)
            if test.death_seen && npc_count.0 >= 12 {
                let msg = format!("npc_count={} (recovered after death)", npc_count.0);
                info!("Test12 Phase 8: PASS — {}", msg);
                test.results.push((8, elapsed, msg));
                test.passed = true;
                info!("========================================");
                info!("Test12: ALL 8 PHASES PASSED ({:.1}s)", elapsed);
                info!("========================================");
                for (phase, t, m) in &test.results {
                    info!("  Phase {}: {:.1}s — {}", phase, t, m);
                }
            } else if elapsed > 60.0 {
                let msg = format!("npc_count={}, lowest={}, death_seen={}", npc_count.0, test.lowest_npc_count, test.death_seen);
                info!("Test12 Phase 8: FAIL — {} (waited {:.1}s)", msg, elapsed);
                test.results.push((8, elapsed, msg));
                test.failed = true;
                info!("========================================");
                info!("Test12: FAILED at Phase 8");
                info!("========================================");
                for (phase, t, m) in &test.results {
                    info!("  Phase {}: {:.1}s — {}", phase, t, m);
                }
            }
        }
        _ => {}
    }
}

/// Toggle debug flags with F1-F4 keys.
fn debug_toggle_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut flags: ResMut<DebugFlags>,
) {
    if keys.just_pressed(KeyCode::F1) {
        flags.readback = !flags.readback;
        info!("Debug readback: {}", if flags.readback { "ON" } else { "OFF" });
    }
    if keys.just_pressed(KeyCode::F2) {
        flags.combat = !flags.combat;
        info!("Debug combat: {}", if flags.combat { "ON" } else { "OFF" });
    }
    if keys.just_pressed(KeyCode::F3) {
        flags.spawns = !flags.spawns;
        info!("Debug spawns: {}", if flags.spawns { "ON" } else { "OFF" });
    }
    if keys.just_pressed(KeyCode::F4) {
        flags.behavior = !flags.behavior;
        info!("Debug behavior: {}", if flags.behavior { "ON" } else { "OFF" });
    }
}

/// Debug: log NPC count every second, plus optional detailed logs.
fn debug_tick_system(
    time: Res<Time>,
    npc_count: Res<NpcCount>,
    gpu_state: Res<GpuReadState>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    flags: Res<DebugFlags>,
    mut last_log: Local<f32>,
) {
    *last_log += time.delta_secs();
    if *last_log >= 1.0 {
        info!("Tick: {} NPCs active", npc_count.0);

        if flags.readback {
            let n = gpu_state.npc_count.min(5);
            for i in 0..n {
                let x = gpu_state.positions.get(i * 2).copied().unwrap_or(0.0);
                let y = gpu_state.positions.get(i * 2 + 1).copied().unwrap_or(0.0);
                let ct = gpu_state.combat_targets.get(i).copied().unwrap_or(-99);
                info!("  NPC[{}] pos=({:.1},{:.1}) target={}", i, x, y, ct);
            }
        }

        if flags.combat {
            info!("  Combat: targets={} attacks={} chases={} in_range={}",
                combat_debug.targets_found, combat_debug.attacks_made,
                combat_debug.chases_started, combat_debug.in_range_count);
            info!("  Health: damage={} deaths={}",
                health_debug.damage_processed, health_debug.deaths_this_frame);
        }

        *last_log = 0.0;
    }
}

fn bevy_timer_end(timer: Res<BevyFrameTimer>) {
    if let Some(start) = timer.start {
        let elapsed = start.elapsed().as_secs_f32() * 1000.0;
        if let Ok(mut stats) = PERF_STATS.lock() {
            stats.bevy_ms = elapsed;
        }
    }
}

/// Build the Bevy application.
pub fn build_app(app: &mut App) {
    app
       // Events
       .add_message::<SpawnNpcMsg>()
       .add_message::<SetTargetMsg>()
       .add_message::<ArrivalMsg>()
       .add_message::<DamageMsg>()
       .add_message::<GpuUpdateMsg>()
       // Resources
       .init_resource::<NpcCount>()
       .init_resource::<NpcEntityMap>()
       .init_resource::<PopulationStats>()
       .init_resource::<GameConfig>()
       .init_resource::<GameTime>()
       .init_resource::<RespawnTimers>()
       .init_resource::<world::WorldData>()
       .init_resource::<world::BedOccupancy>()
       .init_resource::<world::FarmOccupancy>()
       .init_resource::<FarmStates>()
       .init_resource::<HealthDebug>()
       .init_resource::<CombatDebug>()
       .init_resource::<KillStats>()
       .init_resource::<SelectedNpc>()
       .init_resource::<NpcMetaCache>()
       .init_resource::<NpcEnergyCache>()
       .init_resource::<NpcsByTownCache>()
       .init_resource::<NpcLogCache>()
       .init_resource::<FoodEvents>()
       .init_resource::<DebugFlags>()
       .init_resource::<ResetFlag>()
       .init_resource::<GpuReadState>()
       .init_resource::<GpuDispatchCount>()
       .init_resource::<SlotAllocator>()
       .init_resource::<ProjSlotAllocator>()
       .init_resource::<FoodStorage>()
       .init_resource::<FactionStats>()
       .init_resource::<CampState>()
       .init_resource::<RaidQueue>()
       .init_resource::<BevyFrameTimer>()
       .init_resource::<Test12>()
       // Plugins
       .add_plugins(gpu::GpuComputePlugin)
       .add_plugins(render::RenderPlugin)
       .add_plugins(npc_render::NpcRenderPlugin)
       // Startup
       .add_systems(Startup, (startup_system, test12_setup))
       // System sets
       .configure_sets(Update, (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior).chain())
       .add_systems(Update, bevy_timer_start.before(Step::Drain))
       .add_systems(Update, ApplyDeferred.after(Step::Spawn).before(Step::Combat))
       // Drain
       .add_systems(Update, (
           reset_bevy_system,
           drain_arrival_queue,
           drain_game_config,
           sync_gpu_state_to_bevy,
       ).in_set(Step::Drain))
       // GPU→ECS position readback (after drain populates GpuReadState, before spawn reads positions)
       .add_systems(Update, gpu_position_readback.after(Step::Drain).before(Step::Spawn))
       // Spawn
       .add_systems(Update, (
           spawn_npc_system,
           apply_targets_system,
       ).in_set(Step::Spawn))
       // Combat
       .add_systems(Update, (
           process_proj_hits,  // First: recycle slots so attack_system can reuse them
           cooldown_system,
           attack_system,
           damage_system,
           death_system,
           death_cleanup_system,
       ).chain().in_set(Step::Combat))
       // Behavior
       .add_systems(Update, (
           arrival_system,
           energy_system,
           healing_system,
           on_duty_tick_system,
           game_time_system,
           farm_growth_system,
           camp_forage_system,
           raider_respawn_system,
           starvation_system,
           decision_system,
       ).in_set(Step::Behavior))
       .add_systems(Update, collect_gpu_updates.after(Step::Behavior))
       .add_systems(Update, test12_tick.after(Step::Behavior))
       .add_systems(Update, bevy_timer_end.after(collect_gpu_updates))
       // Debug (F1=readback, F2=combat, F3=spawns, F4=behavior)
       .add_systems(Update, (debug_toggle_system, debug_tick_system));
}
