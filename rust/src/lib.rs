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
pub mod save;
pub mod settings;
pub mod systemparams;
pub mod systems;
pub mod tests;
pub mod ui;
pub mod world;

// ============================================================================
// IMPORTS
// ============================================================================

use bevy::prelude::*;

use messages::{
    SpawnNpcMsg, DamageMsg, GpuUpdateMsg, CombatLogMsg, DestroyBuildingMsg,
    SelectFactionMsg,
    ProjGpuUpdateMsg,
    BuildingGridDirtyMsg, TerrainDirtyMsg, PatrolsDirtyMsg, PatrolPerimeterDirtyMsg,
    HealingZonesDirtyMsg, SquadsDirtyMsg, MiningDirtyMsg, PatrolSwapMsg,
};
use resources::{
    MigrationState, EndlessMode,
    EntityMap, PopulationStats, GameConfig, GameTime,
    HealthDebug, CombatDebug, KillStats, SelectedNpc,
    NpcMetaCache, NpcsByTownCache, NpcLogCache,
    GpuReadState, GpuSlotPool, ProjSlotAllocator, NextEntityUid,
    FoodStorage, GoldStorage, FactionStats, RaiderState, SystemTimings,
    DebugFlags, ProjHitState, ProjPositionState, UiState, CombatLog, BuildMenuContext,
    TowerState, FollowSelected, TownPolicies, SelectedBuilding,
    AutoUpgrade, SquadState, HelpCatalog,
    Difficulty, HealingZoneCache, GameAudio, PlaySfxMsg, TutorialState, MiningPolicy,
    BuildingHealState, ActiveHealingSlots, NpcTargetThrashDebug, MovementIntents,
};
use systems::{AiPlayerConfig, AiPlayerState};
use systems::*;


// ============================================================================
// APP STATE
// ============================================================================

/// Application state machine.
/// MainMenu → Playing (real game) or TestMenu → Running (debug tests).
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    MainMenu,
    Playing,
    TestMenu,
    Running,
}

// ============================================================================
// HELPERS
// ============================================================================

// derive_npc_state removed — NPC state lives in NpcInstance, not ECS

/// Get job name from job ID. Delegates to NPC registry.
pub fn job_name(job: i32) -> &'static str {
    components::Job::from_i32(job).label()
}

/// Get trait name from trait ID.
pub fn trait_name(trait_id: i32) -> &'static str {
    components::TraitKind::from_id(trait_id).map(|t| t.name()).unwrap_or("")
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

fn frame_timer_start(timings: Res<SystemTimings>, time: Res<Time>) {
    timings.record_frame_delta(time.delta_secs());
    // Drain render-world atomic timings into SystemTimings
    if timings.enabled {
        use std::sync::atomic::Ordering;
        use crate::messages::{RENDER_TIMINGS, RT_NAMES, RT_COUNT};
        for i in 0..RT_COUNT {
            let bits = RENDER_TIMINGS[i].swap(0, Ordering::Relaxed);
            if bits != 0 {
                timings.record(RT_NAMES[i], f32::from_bits(bits));
            }
        }
    }
}

fn startup_system() {
    info!("Endless ECS initialized - systems registered");
}

/// Sync debug settings from UserSettings into DebugFlags + SystemTimings resources.
fn sync_debug_settings(
    settings: Res<crate::settings::UserSettings>,
    mut flags: ResMut<DebugFlags>,
    mut timings: ResMut<SystemTimings>,
) {
    flags.readback = settings.debug_readback;
    flags.combat = settings.debug_combat;
    flags.spawns = settings.debug_spawns;
    flags.behavior = settings.debug_behavior;
    timings.enabled = settings.debug_profiler;
    crate::messages::RENDER_PROFILING.store(settings.debug_profiler, std::sync::atomic::Ordering::Relaxed);
}

/// Debug: log NPC count every second, plus optional detailed logs.
fn debug_tick_system(
    time: Res<Time>,
    slots: Res<GpuSlotPool>,
    gpu_state: Res<GpuReadState>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    flags: Res<DebugFlags>,
    mut last_log: Local<f32>,
) {
    *last_log += time.delta_secs();
    if *last_log >= 1.0 {
        if flags.readback && slots.alive() > 0 {
            info!("Tick: {} NPCs active", slots.alive());
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


/// Build the Bevy application.
pub fn build_app(app: &mut App) {
    // Game systems run during both real game and debug tests
    let game_active = in_state(AppState::Playing).or(in_state(AppState::Running));

    app
       // State
       .init_state::<AppState>()
       // Events
       .add_message::<SpawnNpcMsg>()
       .add_message::<DamageMsg>()
       .add_message::<GpuUpdateMsg>()
       .add_message::<ProjGpuUpdateMsg>()
       .add_message::<CombatLogMsg>()
       .add_message::<BuildingGridDirtyMsg>()
       .add_message::<TerrainDirtyMsg>()
       .add_message::<PatrolsDirtyMsg>()
       .add_message::<PatrolPerimeterDirtyMsg>()
       .add_message::<HealingZonesDirtyMsg>()
       .add_message::<SquadsDirtyMsg>()
       .add_message::<MiningDirtyMsg>()
       .add_message::<PatrolSwapMsg>()
       .add_message::<save::SaveGameMsg>()
       .add_message::<save::LoadGameMsg>()
       // Resources
       .init_resource::<Difficulty>()
       .init_resource::<EntityMap>()
       .init_resource::<PopulationStats>()
       .init_resource::<GameConfig>()
       .init_resource::<GameTime>()
       .init_resource::<world::WorldData>()
       .init_resource::<HealthDebug>()
       .init_resource::<CombatDebug>()
       .init_resource::<NpcTargetThrashDebug>()
       .init_resource::<MovementIntents>()
       .init_resource::<KillStats>()
       .init_resource::<SelectedNpc>()
       .init_resource::<SelectedBuilding>()
       .init_resource::<FollowSelected>()
       .init_resource::<NpcMetaCache>()
       .init_resource::<NpcsByTownCache>()
       .init_resource::<NpcLogCache>()
       .init_resource::<DebugFlags>()
       .init_resource::<GpuReadState>()
       .init_resource::<ProjHitState>()
       .init_resource::<ProjPositionState>()
       .init_resource::<GpuSlotPool>()
       .init_resource::<NextEntityUid>()

       .init_resource::<ProjSlotAllocator>()
       .init_resource::<FoodStorage>()
       .init_resource::<GoldStorage>()
       .init_resource::<FactionStats>()
       .init_resource::<RaiderState>()
       .init_resource::<BuildingHealState>()
       .init_resource::<ActiveHealingSlots>()
       .init_resource::<HealingZoneCache>()
       .init_resource::<SystemTimings>()
       .init_resource::<world::WorldGrid>()
       .init_resource::<world::WorldGenConfig>()
       .init_resource::<UiState>()
       .init_resource::<CombatLog>()
       .init_resource::<world::TownGrids>()
       .init_resource::<BuildMenuContext>()
       .add_message::<DestroyBuildingMsg>()
       .add_message::<SelectFactionMsg>()
       .init_resource::<TowerState>()
       .init_resource::<resources::BuildingHpRender>()
       .init_resource::<SquadState>()
       .insert_resource(HelpCatalog::new())
       .init_resource::<TutorialState>()
       .init_resource::<MigrationState>()
       .init_resource::<EndlessMode>()
       .init_resource::<AiPlayerState>()
       .init_resource::<AiPlayerConfig>()
       .init_resource::<systems::ai_player::AiSnapshotDirty>()
       .init_resource::<systems::ai_player::PerimeterSyncDirty>()
       .init_resource::<resources::NpcDecisionConfig>()
       .init_resource::<systems::stats::CombatConfig>()
       .init_resource::<systems::stats::TownUpgrades>()
       .add_message::<systems::stats::UpgradeMsg>()
       .init_resource::<AutoUpgrade>()
       .init_resource::<TownPolicies>()
       .init_resource::<MiningPolicy>()
       .init_resource::<save::SaveLoadRequest>()
       .init_resource::<save::SaveToast>()
       .init_resource::<GameAudio>()
       .add_message::<PlaySfxMsg>()
       .insert_resource(settings::load_settings())
       // Plugins
       .add_plugins(bevy_egui::EguiPlugin::default())
       .add_plugins(gpu::GpuComputePlugin)
       .add_plugins(render::RenderPlugin)
       .add_plugins(npc_render::NpcRenderPlugin)
       // Startup
       .add_systems(Startup, startup_system)
       .add_systems(Startup, systems::audio::load_music)
       // Music lifecycle
       .add_systems(OnEnter(AppState::Playing), systems::audio::start_music)
       .add_systems(OnExit(AppState::Playing), systems::audio::stop_music)
       .add_systems(Update, systems::audio::jukebox_system.run_if(in_state(AppState::Playing)))
       .add_systems(Update, systems::audio::play_sfx_system.run_if(game_active.clone()))
       // System sets — game systems only run during AppState::Running
       .configure_sets(Update, (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior).chain()
           .run_if(game_active.clone()))
       .add_systems(Update, frame_timer_start.before(Step::Drain).run_if(game_active.clone()))
       .add_systems(Update, ApplyDeferred.after(Step::Spawn).before(Step::Combat).run_if(game_active.clone()))
       // Drain
       .add_systems(Update, (
           drain_game_config,
           drain_combat_log,
       ).in_set(Step::Drain))
       // GPU→ECS position readback
       .add_systems(Update, gpu_position_readback.after(Step::Drain).before(Step::Spawn).run_if(game_active.clone()))
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
           building_tower_system,
       ).chain().in_set(Step::Combat))
       // Behavior
       .add_systems(Update, (
           world::rebuild_building_grid_system.before(decision_system).before(spawner_respawn_system),
           arrival_system,
           energy_system,
           update_healing_zone_cache.before(healing_system),
           healing_system,
           on_duty_tick_system,
           game_time_system,
           growth_system,
           raider_forage_system,
           spawner_respawn_system,
           mining_policy_system.after(spawner_respawn_system).before(decision_system),
           starvation_system,
           decision_system,
           farm_visual_system,
           auto_upgrade_system,
           process_upgrades_system.after(auto_upgrade_system),
           systems::ai_player::ai_dirty_drain_system.before(ai_decision_system),
           ai_decision_system,
           endless_system,
           (rebuild_patrol_routes_system, squad_cleanup_system),
       ).in_set(Step::Behavior))
       .add_systems(Update, systems::ai_player::perimeter_dirty_drain_system.before(sync_patrol_perimeter_system).in_set(Step::Behavior))
       .add_systems(Update, sync_patrol_perimeter_system.before(rebuild_patrol_routes_system).in_set(Step::Behavior))
       .add_systems(Update, ai_squad_commander_system.after(ai_decision_system).before(decision_system).in_set(Step::Behavior))
       .add_systems(Update, sync_building_hp_render.in_set(Step::Behavior))
       // Movement intent resolution — single owner of SetTarget, runs after all intent producers
       .add_systems(Update, resolve_movement_system.after(Step::Behavior).run_if(game_active.clone()))
       // Debug settings sync + tick logging
       .add_systems(Update, (sync_debug_settings, debug_tick_system).run_if(game_active.clone()))
       // Save/Load — F5/F9 input + save + load + toast
       .add_systems(Update, save::save_load_input_system.run_if(in_state(AppState::Playing)))
       .add_systems(Update, save::save_game_system
           .after(save::save_load_input_system).run_if(in_state(AppState::Playing)))
       .add_systems(Update, save::load_game_system
           .after(save::save_load_input_system).run_if(in_state(AppState::Playing)))
       .add_systems(Update, save::autosave_system
           .after(save::save_game_system).run_if(in_state(AppState::Playing)))
       .add_systems(Update, save::save_toast_tick_system.run_if(in_state(AppState::Playing)));

    // Test framework (registers TestState, menu UI, all tests)
    tests::register_tests(app);

    // UI (main menu, game startup, in-game HUD)
    ui::register_ui(app);
}
