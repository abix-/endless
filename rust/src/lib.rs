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
pub mod tests;
pub mod world;

// ============================================================================
// IMPORTS
// ============================================================================

use bevy::prelude::*;

use messages::{SpawnNpcMsg, DamageMsg, GpuUpdateMsg};
use resources::{
    NpcCount, NpcEntityMap, PopulationStats, GameConfig, GameTime, RespawnTimers,
    FarmStates, HealthDebug, CombatDebug, KillStats, SelectedNpc,
    NpcMetaCache, NpcEnergyCache, NpcsByTownCache, NpcLogCache, FoodEvents,
    ResetFlag, GpuReadState, GpuDispatchCount, SlotAllocator, ProjSlotAllocator,
    FoodStorage, FactionStats, CampState, RaidQueue, BevyFrameTimer, PERF_STATS,
    DebugFlags,
};
use systems::*;
use components::*;
use tests::AppState;

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
    let running = in_state(AppState::Running);

    app
       // Events
       .add_message::<SpawnNpcMsg>()
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
       .init_resource::<world::WorldGrid>()
       .init_resource::<world::WorldGenConfig>()
       // Plugins
       .add_plugins(bevy_egui::EguiPlugin::default())
       .add_plugins(gpu::GpuComputePlugin)
       .add_plugins(render::RenderPlugin)
       .add_plugins(npc_render::NpcRenderPlugin)
       // Startup
       .add_systems(Startup, startup_system)
       // System sets — game systems only run during AppState::Running
       .configure_sets(Update, (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior).chain()
           .run_if(running.clone()))
       .add_systems(Update, bevy_timer_start.before(Step::Drain).run_if(running.clone()))
       .add_systems(Update, ApplyDeferred.after(Step::Spawn).before(Step::Combat).run_if(running.clone()))
       // Drain
       .add_systems(Update, (
           reset_bevy_system,
           drain_game_config,
           sync_gpu_state_to_bevy,
       ).in_set(Step::Drain))
       // GPU→ECS position readback
       .add_systems(Update, gpu_position_readback.after(Step::Drain).before(Step::Spawn).run_if(running.clone()))
       // Spawn
       .add_systems(Update,
           spawn_npc_system.in_set(Step::Spawn))
       // Combat
       .add_systems(Update, (
           process_proj_hits,
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
       .add_systems(Update, collect_gpu_updates.after(Step::Behavior).run_if(running.clone()))
       .add_systems(Update, bevy_timer_end.after(collect_gpu_updates).run_if(running.clone()))
       // Debug (F1=readback, F2=combat, F3=spawns, F4=behavior)
       .add_systems(Update, (debug_toggle_system, debug_tick_system).run_if(running));

    // Test framework (registers AppState, TestState, menu UI, all tests)
    tests::register_tests(app);
}
