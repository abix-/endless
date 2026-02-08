//! Endless ECS - Pure Bevy colony simulation.
//! See docs/ for architecture documentation.

// ============================================================================
// MODULES
// ============================================================================

pub mod components;
pub mod constants;
pub mod gpu;
pub mod messages;
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
    FarmStates, HealthDebug, CombatDebug, KillStats, SelectedNpc,
    NpcMetaCache, NpcEnergyCache, NpcsByTownCache, NpcLogCache, FoodEvents,
    ResetFlag, GpuReadState, GpuDispatchCount, SlotAllocator, ProjSlotAllocator,
    FoodStorage, FactionStats, CampState, RaidQueue, BevyFrameTimer, PERF_STATS,
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

/// Debug: log NPC count every second
fn debug_tick_system(
    time: Res<Time>,
    npc_count: Res<NpcCount>,
    mut last_log: Local<f32>,
) {
    *last_log += time.delta_secs();
    if *last_log >= 1.0 {
        info!("Tick: {} NPCs active", npc_count.0);
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
       // GPU compute plugin
       .add_plugins(gpu::GpuComputePlugin)
       // Startup
       .add_systems(Startup, startup_system)
       // System sets
       .configure_sets(Update, (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior).chain())
       .add_systems(Update, bevy_timer_start.before(Step::Drain))
       .add_systems(Update, ApplyDeferred.after(Step::Spawn).before(Step::Combat))
       // Drain
       .add_systems(Update, (
           reset_bevy_system,
           drain_arrival_queue,
           drain_game_config,
       ).in_set(Step::Drain))
       // Spawn
       .add_systems(Update, (
           spawn_npc_system,
           apply_targets_system,
       ).in_set(Step::Spawn))
       // Combat
       .add_systems(Update, (
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
       .add_systems(Update, bevy_timer_end.after(collect_gpu_updates))
       // Debug (remove when GPU compute working)
       .add_systems(Update, debug_tick_system);
}
