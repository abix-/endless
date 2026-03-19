//! Endless ECS - Pure Bevy colony simulation.
//! See docs/ for architecture documentation.
#![allow(
    clippy::too_many_arguments, // Bevy systems have many params by design
    clippy::type_complexity,    // Bevy Query types are inherently complex
    clippy::collapsible_if,     // Nested ifs are often clearer in game logic
)]

// ============================================================================
// MODULES
// ============================================================================

pub mod components;
pub mod constants;
pub mod entity_map;
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
pub mod tracing_layer;
pub mod ui;
pub mod world;

// ============================================================================
// IMPORTS
// ============================================================================

use bevy::prelude::*;
use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};

use messages::{
    BuildingGridDirtyMsg, CombatLogMsg, DamageMsg, DestroyBuildingMsg, GpuUpdateMsg,
    HealingZonesDirtyMsg, MiningDirtyMsg, PatrolPerimeterDirtyMsg, PatrolSwapMsg, PatrolsDirtyMsg,
    ProjGpuUpdateMsg, SelectFactionMsg, SpawnNpcMsg, SquadsDirtyMsg, TerrainDirtyMsg,
};
use resources::{
    ActiveHealingSlots, AutoUpgrade, BuildMenuContext, BuildingHealState, CombatDebug, CombatLog,
    DebugFlags, DeltaTime, Difficulty, EndlessMode, EntityMap, FactionList, FactionStats,
    FollowSelected, GameAudio, GameConfig, GameTime, GpuReadState, GpuSlotPool, HealingZoneCache,
    HealthDebug, HelpCatalog, KillStats, MerchantInventory, MigrationState, MiningPolicy,
    NextLootItemId, NpcLogCache, NpcTargetThrashDebug, PlaySfxMsg, PopulationStats, ProjHitState,
    ProjPositionState, ProjSlotAllocator, RaiderState, Reputation, SelectedBuilding, SelectedNpc,
    SquadState, SystemTimings, TowerState, TutorialState, UiState, UpsCounter,
};
use systems::*;
use systems::{AiPlayerConfig, AiPlayerState};

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

fn ups_tick(mut ups: ResMut<UpsCounter>) {
    ups.ticks_this_second += 1;
}

/// EMA-smoothed delta for visual systems (GPU movement, camera pan).
/// Filters Bevy's jittery Time::delta_secs() to prevent microstutter.
fn smooth_delta(time: Res<Time>, game_time: Res<GameTime>, mut dt: ResMut<DeltaTime>) {
    let raw = game_time.delta(&time);
    let alpha = 0.1;
    dt.0 = if dt.0 == 0.0 {
        raw
    } else {
        dt.0 * (1.0 - alpha) + raw * alpha
    };
}

/// Scale FixedUpdate period with sqrt(time_scale) to balance cascade prevention with UPS.
/// At 4x speed: period = sqrt(4)/60 = 2/60s (30 Hz). At 16x: sqrt(16)/60 = 4/60 (15 Hz).
/// Linear scaling (ts/60) cut UPS too aggressively (8x -> 7.5 UPS, game looks frozen).
/// Sqrt keeps UPS playable (8x -> 21 Hz) while still preventing the tick cascade.
/// game_time.delta() = sqrt(ts)/60 * ts per tick; ticks/s = 60/sqrt(ts);
/// game-s/real-s = (60/sqrt(ts)) * sqrt(ts)/60 * ts = ts. Proportionality preserved.
fn sync_fixed_hz(game_time: Res<GameTime>, mut fixed_time: ResMut<Time<Fixed>>) {
    // Clamp below 1.0 so slow-motion keeps Fixed at 60 Hz (game_time.delta handles it).
    // Cap at 32 to limit per-tick dt size for timer-based systems.
    let ts = (game_time.time_scale.max(1.0) as f64).min(32.0);
    let period = std::time::Duration::from_secs_f64(ts.sqrt() / 60.0);
    if fixed_time.timestep() != period {
        fixed_time.set_timestep(period);
    }
}

fn frame_timer_start(timings: Res<SystemTimings>, time: Res<Time>) {
    timings.record_frame_delta(time.delta_secs());
    // Drain render-world atomic timings into SystemTimings
    if timings.enabled {
        use crate::messages::{RENDER_TIMINGS, RT_COUNT, RT_NAMES};
        use std::sync::atomic::Ordering;
        for i in 0..RT_COUNT {
            let bits = RENDER_TIMINGS[i].swap(0, Ordering::Relaxed);
            if bits != 0 {
                timings.record(RT_NAMES[i], f32::from_bits(bits));
            }
        }
        // Drain tracing-captured system timings (Bevy auto-spans)
        if let Ok(mut map) = crate::tracing_layer::TRACING_TIMINGS.lock() {
            for (name, ms) in map.iter_mut() {
                timings.record_traced(name, *ms);
                // Decay stale entries — active systems overwrite via on_exit each frame
                *ms *= 0.9;
            }
        }
    }
}

fn startup_system() {
    info!("Endless ECS initialized - systems registered");
}

/// Skip main menu when --autostart is passed. Loads saved settings and starts a new game.
fn autostart_system(
    auto: Res<resources::AutoStart>,
    cli: Res<resources::CliOverrides>,
    mut commands: Commands,
    mut wg_config: ResMut<world::WorldGenConfig>,
    mut ai_config: ResMut<AiPlayerConfig>,
    mut npc_config: ResMut<resources::NpcDecisionConfig>,
    mut pathfind_config: ResMut<resources::PathfindConfig>,
    user_settings: Res<settings::UserSettings>,
    mut save_request: ResMut<save::SaveLoadRequest>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if !auto.0 {
        return;
    }
    info!("--autostart: bypassing main menu");

    let saved = settings::load_settings();

    // World gen config
    wg_config.gen_style = world::WorldGenStyle::from_index(saved.gen_style);
    wg_config.world_width = saved.world_size;
    wg_config.world_height = saved.world_size;
    wg_config.num_towns = 1;
    wg_config.farms_per_town = saved.farms;
    wg_config.npc_counts = saved
        .npc_counts
        .iter()
        .filter_map(|(k, &v)| {
            let job = match k.as_str() {
                "Farmer" => components::Job::Farmer,
                "Archer" => components::Job::Archer,
                "Raider" => components::Job::Raider,
                "Fighter" => components::Job::Fighter,
                "Miner" => components::Job::Miner,
                "Crossbow" => components::Job::Crossbow,
                "Boat" => components::Job::Boat,
                _ => return None,
            };
            Some((job, v))
        })
        .collect();
    let ai_builder_count = saved.ai_slots.iter().filter(|s| s.kind == 0).count();
    let ai_raider_count = saved.ai_slots.iter().filter(|s| s.kind == 1).count();
    wg_config.ai_towns = ai_builder_count;
    wg_config.raider_towns = ai_raider_count;
    wg_config.gold_mines_per_town = saved.gold_mines_per_town;

    // CLI overrides
    if cli.no_raiders {
        wg_config.raider_towns = 0;
        info!("--no-raiders: disabled raider towns");
    }
    if let Some(farms) = cli.farms {
        wg_config.farms_per_town = farms;
        info!("--farms={}: overriding farms per town", farms);
    }

    // AI/NPC config
    ai_config.decision_interval = saved.ai_interval;
    npc_config.interval = saved.npc_interval;
    pathfind_config.max_per_frame = user_settings.pathfind_max_per_frame.max(1);

    // Runtime resources
    commands.insert_resource(saved.difficulty);
    commands.insert_resource(EndlessMode {
        enabled: true,
        strength_fraction: saved.endless_strength,
        pending_spawns: Vec::new(),
    });

    // LLM-allowed towns
    let num_player_towns = wg_config.num_towns;
    let mut llm_towns = Vec::new();
    let mut builder_idx = 0usize;
    let mut raider_idx = 0usize;
    for slot in &saved.ai_slots {
        if slot.kind == 0 {
            if slot.llm {
                llm_towns.push(num_player_towns + builder_idx);
            }
            builder_idx += 1;
        } else {
            if slot.llm {
                llm_towns.push(num_player_towns + ai_builder_count + raider_idx);
            }
            raider_idx += 1;
        }
    }
    if let Some(&first_llm) = llm_towns.first() {
        commands.insert_resource(systems::llm_player::LlmPlayerState::new(first_llm));
    }
    commands.insert_resource(resources::RemoteAllowedTowns { towns: llm_towns });

    save_request.autosave_hours = saved.autosave_hours;
    next_state.set(AppState::Playing);
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
    crate::messages::RENDER_PROFILING.store(
        settings.debug_profiler,
        std::sync::atomic::Ordering::Relaxed,
    );
}

/// Debug: log NPC count every second, plus optional detailed logs.
fn debug_tick_system(
    time: Res<Time>,
    entity_map: Res<EntityMap>,
    gpu_state: Res<GpuReadState>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    flags: Res<DebugFlags>,
    mut last_log: Local<f32>,
) {
    *last_log += time.delta_secs();
    if *last_log >= 1.0 {
        let npc_count = entity_map.npc_count();
        if flags.readback && npc_count > 0 {
            info!("Tick: {} NPCs active", npc_count);
            let n = npc_count.min(5);
            for i in 0..n {
                let x = gpu_state.positions.get(i * 2).copied().unwrap_or(0.0);
                let y = gpu_state.positions.get(i * 2 + 1).copied().unwrap_or(0.0);
                let ct = gpu_state.combat_targets.get(i).copied().unwrap_or(-99);
                info!("  NPC[{}] pos=({:.1},{:.1}) target={}", i, x, y, ct);
            }
        }

        if flags.combat {
            info!(
                "  Combat: targets={} attacks={} chases={} in_range={}",
                combat_debug.targets_found,
                combat_debug.attacks_made,
                combat_debug.chases_started,
                combat_debug.in_range_count
            );
            info!(
                "  Health: damage={} deaths={}",
                health_debug.damage_processed, health_debug.deaths_this_frame
            );
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
        .add_message::<messages::WorkIntentMsg>()
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
        .init_resource::<DeltaTime>()
        .init_resource::<world::WorldData>()
        .init_resource::<HealthDebug>()
        .init_resource::<systems::DeathQueue>()
        .init_resource::<CombatDebug>()
        .init_resource::<NpcTargetThrashDebug>()
        .init_resource::<resources::PathRequestQueue>()
        .init_resource::<resources::PathfindConfig>()
        .init_resource::<resources::PathfindStats>()
        .init_resource::<KillStats>()
        .init_resource::<SelectedNpc>()
        .init_resource::<SelectedBuilding>()
        .init_resource::<FollowSelected>()
        .init_resource::<resources::ReturningSet>()
        .init_resource::<NpcLogCache>()
        .init_resource::<DebugFlags>()
        .init_resource::<GpuReadState>()
        .init_resource::<ProjHitState>()
        .init_resource::<ProjPositionState>()
        .init_resource::<GpuSlotPool>()
        .init_resource::<ProjSlotAllocator>()
        .init_resource::<resources::TownIndex>()
        .init_resource::<FactionStats>()
        .init_resource::<FactionList>()
        .init_resource::<Reputation>()
        .init_resource::<RaiderState>()
        .init_resource::<BuildingHealState>()
        .init_resource::<ActiveHealingSlots>()
        .init_resource::<HealingZoneCache>()
        .init_resource::<resources::AutoStart>()
        .init_resource::<resources::CliTestMode>()
        .init_resource::<SystemTimings>()
        .init_resource::<UpsCounter>()
        .init_resource::<world::WorldGrid>()
        .init_resource::<world::WorldGenConfig>()
        .init_resource::<UiState>()
        .init_resource::<CombatLog>()
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
        .add_message::<systems::stats::UpgradeMsg>()
        .add_message::<systems::stats::EquipItemMsg>()
        .add_message::<systems::stats::UnequipItemMsg>()
        .add_message::<systems::stats::AutoEquipNowMsg>()
        .init_resource::<AutoUpgrade>()
        .init_resource::<MiningPolicy>()
        .init_resource::<save::SaveLoadRequest>()
        .init_resource::<save::SaveToast>()
        .init_resource::<GameAudio>()
        .init_resource::<NextLootItemId>()
        .init_resource::<MerchantInventory>()
        .add_message::<PlaySfxMsg>()
        .insert_resource(settings::load_settings())
        // Fixed 60 UPS game loop (Factorio model)
        .insert_resource(Time::<Fixed>::from_hz(60.0))
        // Plugins
        .add_plugins(bevy_framepace::FramepacePlugin)
        .add_plugins(bevy_egui::EguiPlugin::default())
        .add_plugins(gpu::GpuComputePlugin)
        .add_plugins(render::RenderPlugin)
        .add_plugins(npc_render::NpcRenderPlugin)
        // BRP: live game data access via HTTP JSON-RPC on localhost:15702
        .add_plugins(
            RemotePlugin::default()
                // Read
                .with_method("endless/get_summary", systems::remote::summary_handler)
                .with_method("endless/get_perf", systems::remote::perf_handler)
                .with_method("endless/get_entity", systems::remote::debug_handler)
                .with_method("endless/get_squad", systems::remote::squad_handler)
                .with_method(
                    "endless/list_buildings",
                    systems::remote::list_buildings_handler,
                )
                .with_method("endless/list_npcs", systems::remote::list_npcs_handler)
                // Create / Delete
                .with_method("endless/create_building", systems::remote::build_handler)
                .with_method("endless/delete_building", systems::remote::destroy_handler)
                // Update
                .with_method("endless/set_time", systems::remote::time_handler)
                .with_method("endless/set_policy", systems::remote::policy_handler)
                .with_method(
                    "endless/set_ai_manager",
                    systems::remote::ai_manager_handler,
                )
                .with_method(
                    "endless/set_squad_target",
                    systems::remote::squad_target_handler,
                )
                // Actions
                .with_method("endless/apply_upgrade", systems::remote::upgrade_handler)
                .with_method("endless/send_chat", systems::remote::chat_handler)
                .with_method(
                    "endless/recruit_squad",
                    systems::remote::squad_recruit_handler,
                )
                .with_method(
                    "endless/dismiss_squad",
                    systems::remote::squad_dismiss_handler,
                ),
        )
        .add_plugins(RemoteHttpPlugin::default())
        .init_resource::<systems::remote::RemoteBuildQueue>()
        .init_resource::<systems::remote::RemoteDestroyQueue>()
        .init_resource::<systems::remote::RemoteUpgradeQueue>()
        .init_resource::<systems::remote::RemoteLlmLogQueue>()
        .init_resource::<systems::remote::RemoteCombatLogRing>()
        .init_resource::<resources::RemoteAllowedTowns>()
        .init_resource::<resources::ChatInbox>()
        // Register reflected types for BRP queries
        .register_type::<components::GpuSlot>()
        .register_type::<components::Position>()
        .register_type::<components::Job>()
        .register_type::<components::Speed>()
        .register_type::<components::TownId>()
        .register_type::<components::Energy>()
        .register_type::<components::Home>()
        .register_type::<components::PatrolRoute>()
        .register_type::<components::NpcWorkState>()
        .register_type::<components::CarriedLoot>()
        .register_type::<components::Activity>()
        .register_type::<components::CombatState>()
        .register_type::<components::ManualTarget>()
        .register_type::<components::NpcFlags>()
        .register_type::<components::NpcPath>()
        .register_type::<components::SquadId>()
        .register_type::<components::Dead>()
        .register_type::<components::Health>()
        .register_type::<components::BaseAttackType>()
        .register_type::<components::CachedStats>()
        .register_type::<components::Faction>()
        .register_type::<components::AttackTimer>()
        .register_type::<components::Stealer>()
        .register_type::<components::HasEnergy>()
        .register_type::<components::NpcEquipment>()
        .register_type::<components::NpcStats>()
        .register_type::<components::NpcSkills>()
        .register_type::<components::LastHitBy>()
        .register_type::<components::Building>()
        .register_type::<components::FarmReadyMarker>()
        .register_type::<components::FleeThreshold>()
        .register_type::<components::LeashRange>()
        .register_type::<components::WoundedThreshold>()
        .register_type::<components::Personality>()
        .register_type::<components::TraitKind>()
        .register_type::<components::TraitInstance>()
        .register_type::<components::EquipLayer>()
        // Reflected resources
        .register_type::<GameTime>()
        .register_type::<UpsCounter>()
        .register_type::<KillStats>()
        .register_type::<FactionStats>()
        .register_type::<resources::FactionStat>()
        .register_type::<resources::RemoteAllowedTowns>()
        .register_type::<resources::PolicySet>()
        .register_type::<resources::WorkSchedule>()
        .register_type::<resources::OffDutyBehavior>()
        // Reflected nested types
        .register_type::<constants::LootItem>()
        .register_type::<constants::ItemKind>()
        .register_type::<constants::Rarity>()
        .register_type::<world::BuildingKind>()
        .register_type::<Difficulty>()
        // Startup
        .add_systems(Startup, startup_system)
        .add_systems(Startup, systems::audio::load_music)
        .add_systems(Startup, systems::audio::load_sfx)
        // Autostart: skip main menu if --autostart was passed
        .add_systems(OnEnter(AppState::MainMenu), autostart_system)
        // Music lifecycle
        .add_systems(OnEnter(AppState::Playing), systems::audio::start_music)
        .add_systems(OnExit(AppState::Playing), systems::audio::stop_music)
        .add_systems(Update, smooth_delta)
        .add_systems(Update, sync_fixed_hz)
        .add_systems(
            Update,
            systems::audio::jukebox_system.run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            systems::audio::play_sfx_system.run_if(game_active.clone()),
        )
        // System sets — game systems run at fixed 60 UPS
        .configure_sets(
            FixedUpdate,
            (Step::Drain, Step::Spawn, Step::Combat, Step::Behavior)
                .chain()
                .run_if(game_active.clone()),
        )
        .add_systems(FixedUpdate, ups_tick)
        .add_systems(
            FixedUpdate,
            frame_timer_start
                .before(Step::Drain)
                .run_if(game_active.clone()),
        )
        .add_systems(
            FixedUpdate,
            ApplyDeferred
                .after(Step::Spawn)
                .before(Step::Combat)
                .run_if(game_active.clone()),
        )
        // Drain
        .add_systems(
            FixedUpdate,
            (drain_game_config, drain_combat_log).in_set(Step::Drain),
        )
        // GPU→ECS position readback
        .add_systems(
            FixedUpdate,
            gpu_position_readback
                .after(Step::Drain)
                .before(Step::Spawn)
                .run_if(game_active.clone()),
        )
        // Remote action queue drain (BRP → game actions)
        .add_systems(
            FixedUpdate,
            systems::remote::drain_remote_queues
                .in_set(Step::Spawn)
                .before(spawn_npc_system),
        )
        // Spawn
        .add_systems(FixedUpdate, spawn_npc_system.in_set(Step::Spawn))
        // Combat
        .add_systems(
            FixedUpdate,
            (
                process_proj_hits,
                cooldown_system,
                attack_system,
                damage_system,
                death_system,
                building_tower_system,
            )
                .chain()
                .in_set(Step::Combat),
        )
        // Behavior
        .add_systems(
            FixedUpdate,
            (
                world::rebuild_building_grid_system
                    .before(decision_system)
                    .before(spawner_respawn_system),
                (sync_returning_set, arrival_system.after(sync_returning_set)),
                energy_system,
                (
                    update_healing_zone_cache.before(healing_system),
                    healing_system,
                    npc_regen_system,
                ),
                on_duty_tick_system,
                game_time_system,
                (
                    construction_tick_system.before(growth_system),
                    growth_system,
                    farming_skill_system,
                    sync_sleeping_system,
                ),
                raider_forage_system,
                spawner_respawn_system,
                mining_policy_system
                    .after(spawner_respawn_system)
                    .before(decision_system),
                starvation_system,
                decision_system,
                farm_visual_system,
                (
                    auto_upgrade_system,
                    systems::stats::auto_tower_upgrade_system,
                    systems::stats::auto_equip_system,
                    systems::stats::prune_town_equipment_system,
                ),
                process_upgrades_system.after(auto_upgrade_system),
                systems::stats::process_equip_system
                    .after(process_upgrades_system)
                    .after(systems::stats::auto_equip_system),
                systems::ai_player::ai_dirty_drain_system.before(ai_decision_system),
                ai_decision_system,
                endless_system,
                (rebuild_patrol_routes_system, squad_cleanup_system),
            )
                .in_set(Step::Behavior),
        )
        .add_systems(
            FixedUpdate,
            systems::ai_player::perimeter_dirty_drain_system
                .before(sync_patrol_perimeter_system)
                .in_set(Step::Behavior),
        )
        .add_systems(
            FixedUpdate,
            sync_patrol_perimeter_system
                .before(rebuild_patrol_routes_system)
                .in_set(Step::Behavior),
        )
        .add_systems(
            FixedUpdate,
            ai_squad_commander_system
                .after(ai_decision_system)
                .before(decision_system)
                .in_set(Step::Behavior),
        )
        .add_systems(
            FixedUpdate,
            systems::llm_player::llm_player_system
                .run_if(resource_exists::<systems::llm_player::LlmPlayerState>)
                .in_set(Step::Behavior),
        )
        .add_systems(FixedUpdate, sync_building_hp_render.in_set(Step::Behavior))
        .add_systems(FixedUpdate, merchant_tick_system.in_set(Step::Behavior))
        .add_systems(
            FixedUpdate,
            systems::work_targeting::resolve_work_targets
                .after(decision_system)
                .in_set(Step::Behavior),
        )
        // Waypoint advancement — advance NpcPath after gpu_position_readback sets at_destination
        .add_systems(
            FixedUpdate,
            advance_waypoints_system
                .after(gpu_position_readback)
                .before(Step::Spawn)
                .run_if(game_active.clone()),
        )
        // Pathfinding cost sync + path invalidation on building changes
        .add_systems(
            FixedUpdate,
            (
                systems::pathfinding::sync_pathfind_costs_system
                    .after(world::rebuild_building_grid_system),
                systems::pathfinding::invalidate_paths_on_building_change
                    .after(world::rebuild_building_grid_system),
            )
                .in_set(Step::Behavior),
        )
        // Movement intent resolution — single owner of SetTarget, runs after all intent producers
        .add_systems(
            FixedUpdate,
            resolve_movement_system
                .after(Step::Behavior)
                .run_if(game_active.clone()),
        )
        // Debug settings sync + tick logging
        .add_systems(
            FixedUpdate,
            (sync_debug_settings, debug_tick_system).run_if(game_active.clone()),
        )
        // Save/Load — F5/F9 input + save + load + toast
        .add_systems(
            Update,
            save::save_load_input_system.run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            save::save_game_system
                .after(save::save_load_input_system)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            save::load_game_system
                .after(save::save_load_input_system)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            save::autosave_system
                .after(save::save_game_system)
                .run_if(in_state(AppState::Playing)),
        )
        .add_systems(
            Update,
            save::save_toast_tick_system.run_if(in_state(AppState::Playing)),
        );

    // Test framework (registers TestState, menu UI, all tests)
    tests::register_tests(app);

    // UI (main menu, game startup, in-game HUD)
    ui::register_ui(app);
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests_fixed_hz {
    use super::*;
    use bevy::time::TimeUpdateStrategy;

    fn make_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.add_systems(Update, sync_fixed_hz);
        app.update(); // prime resources
        app
    }

    /// Fixed period must be 1/60s at 1x speed (baseline: no regression on 1x).
    #[test]
    fn fixed_period_is_baseline_at_1x() {
        let app = make_app();
        // time_scale = 1.0 (default) -- sync_fixed_hz already primed in make_app
        let period = app.world().resource::<Time<Fixed>>().timestep();
        let expected = std::time::Duration::from_secs_f64(1.0 / 60.0);
        assert_eq!(
            period, expected,
            "1x speed must keep 60 Hz Fixed period, got {:?}",
            period
        );
    }

    /// Fixed period must scale to sqrt(4)/60s at 4x speed (prevents cascade, keeps 30 UPS).
    /// This test FAILS if sync_fixed_hz is removed or the period is not scaled.
    #[test]
    fn fixed_period_scales_at_4x() {
        let mut app = make_app();
        app.world_mut().resource_mut::<GameTime>().time_scale = 4.0;
        app.update();
        let period = app.world().resource::<Time<Fixed>>().timestep();
        let expected = std::time::Duration::from_secs_f64(2.0 / 60.0); // sqrt(4)/60
        assert_eq!(
            period, expected,
            "4x speed must use 30 Hz Fixed period (sqrt scaling), got {:?}",
            period
        );
    }

    /// Fixed period must scale to sqrt(16)/60s at 16x speed (15 Hz, playable).
    /// This test FAILS if sync_fixed_hz is removed.
    #[test]
    fn fixed_period_scales_at_16x() {
        let mut app = make_app();
        app.world_mut().resource_mut::<GameTime>().time_scale = 16.0;
        app.update();
        let period = app.world().resource::<Time<Fixed>>().timestep();
        let expected = std::time::Duration::from_secs_f64(4.0 / 60.0); // sqrt(16)/60
        assert_eq!(
            period, expected,
            "16x speed must use 15 Hz Fixed period (sqrt scaling), got {:?}",
            period
        );
    }

    /// At slow speed (< 1x), Fixed period must stay at 1/60s (clamp prevents faster-than-60Hz).
    #[test]
    fn fixed_period_clamps_at_slow_speed() {
        let mut app = make_app();
        app.world_mut().resource_mut::<GameTime>().time_scale = 0.5;
        app.update();
        let period = app.world().resource::<Time<Fixed>>().timestep();
        let expected = std::time::Duration::from_secs_f64(1.0 / 60.0);
        assert_eq!(
            period, expected,
            "slow speed must stay at 60 Hz (clamped), got {:?}",
            period
        );
    }

    /// game_time.delta() must advance proportionally to time_scale at all speeds.
    /// Sqrt scaling: tick period = sqrt(ts)/60, game_time.delta/tick = ts*sqrt(ts)/60,
    /// ticks/s = 60/sqrt(ts) => game-s/real-s = ts. Proportionality preserved.
    #[test]
    fn game_time_advances_proportionally_to_time_scale() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(Time::<Fixed>::from_hz(60.0));
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0 / 60.0),
        ));
        app.add_systems(Update, sync_fixed_hz);
        app.add_systems(FixedUpdate, crate::game_time_system);

        // Prime
        app.update();
        app.update();

        // Run 60 real frames at 1x speed, measure game time advance
        let before_1x = app.world().resource::<GameTime>().total_seconds;
        for _ in 0..60 {
            app.update();
        }
        let advance_1x = app.world().resource::<GameTime>().total_seconds - before_1x;

        // Now switch to 4x and run another 60 real frames
        app.world_mut().resource_mut::<GameTime>().time_scale = 4.0;
        let before_4x = app.world().resource::<GameTime>().total_seconds;
        for _ in 0..60 {
            app.update();
        }
        let advance_4x = app.world().resource::<GameTime>().total_seconds - before_4x;

        // At 4x, game time should advance ~4x faster per real frame
        let ratio = advance_4x / advance_1x;
        assert!(
            (ratio - 4.0).abs() < 0.5,
            "4x speed should advance game time ~4x faster: ratio={ratio:.2} (advance_1x={advance_1x:.3}, advance_4x={advance_4x:.3})"
        );
    }

    /// At 4x speed, Fixed must run at ~1/2 the rate of 1x -- verifying tick cascade prevention.
    /// Without the fix, Bevy would queue 4 Fixed ticks per frame at 4x, overflowing budget.
    /// With sqrt scaling, Fixed runs at 30 Hz: 1 tick per ~2 real frames at 60Hz.
    /// This test FAILS if sync_fixed_hz is removed or the period is not scaled.
    #[test]
    fn fixed_ticks_per_second_scale_with_speed() {
        use std::sync::{Arc, Mutex};

        // Count total Fixed ticks via a counting system
        let tick_count_1x = Arc::new(Mutex::new(0u32));
        let tick_count_4x = Arc::new(Mutex::new(0u32));

        // 1x speed: run 60 real frames, count Fixed ticks
        {
            let counter = tick_count_1x.clone();
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            app.insert_resource(GameTime::default());
            app.insert_resource(Time::<Fixed>::from_hz(60.0));
            app.insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1.0 / 60.0),
            ));
            app.add_systems(Update, sync_fixed_hz);
            app.add_systems(FixedUpdate, move || {
                *counter.lock().unwrap() += 1;
            });
            app.update(); // prime
            for _ in 0..60 {
                app.update();
            }
        }

        // 4x speed: run 60 real frames, count Fixed ticks
        {
            let counter = tick_count_4x.clone();
            let mut app = App::new();
            app.add_plugins(MinimalPlugins);
            let mut gt = GameTime::default();
            gt.time_scale = 4.0;
            app.insert_resource(gt);
            app.insert_resource(Time::<Fixed>::from_hz(60.0));
            app.insert_resource(TimeUpdateStrategy::ManualDuration(
                std::time::Duration::from_secs_f32(1.0 / 60.0),
            ));
            app.add_systems(Update, sync_fixed_hz);
            app.add_systems(FixedUpdate, move || {
                *counter.lock().unwrap() += 1;
            });
            app.update(); // prime
            for _ in 0..60 {
                app.update();
            }
        }

        let ticks_1x = *tick_count_1x.lock().unwrap();
        let ticks_4x = *tick_count_4x.lock().unwrap();

        // At 1x: ~60 ticks/s * 1s = ~60 ticks
        // At 4x (sqrt scaling): ~30 ticks/s * 1s = ~30 ticks (1/2 as many)
        // Without the fix: 4x would queue 4 ticks/frame = ~240 ticks (4x more than 1x)
        assert!(
            ticks_1x >= 50 && ticks_1x <= 70,
            "1x should tick ~60 times in 60 frames, got {ticks_1x}"
        );
        assert!(
            ticks_4x >= 20 && ticks_4x <= 40,
            "4x (sqrt scaling) should tick ~30 times in 60 frames, got {ticks_4x} (1x={ticks_1x})"
        );
    }
}
