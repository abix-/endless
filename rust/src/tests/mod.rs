//! Test Framework - UI-selectable integration tests for Endless systems.
//!
//! Each test is a file in src/tests/ exporting setup (OnEnter) + tick (Update) systems.
//! Tests are selected from a bevy_egui menu and run within the full Bevy app.

pub mod ai_building;
pub mod archer_patrol;
pub mod archer_tent_reliability;
pub mod coalesce_safety;
pub mod combat;
pub mod economy;
pub mod endless_mode;
pub mod energy;
pub mod farm_visual;
pub mod farmer_cycle;
pub mod fountain_shot_stale;
pub mod friendly_fire_buildings;
pub mod heal_visual;
pub mod healing;
pub mod loot_cycle;
pub mod miner_cycle;
pub mod movement;
pub mod npc_visuals;
pub mod pathfind_maze;
pub mod projectiles;
pub mod raider_cycle;
pub mod sandbox;
pub mod sleep_visual;
pub mod slot_reuse_wave;
pub mod spawning;
pub mod stress_archer_towns;
pub mod terrain_visual;
pub mod vertical_slice;
pub mod world_gen;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use std::collections::HashMap;

use crate::messages::SpawnNpcMsg;
use crate::render::MainCamera;
use crate::resources::*;
use crate::world;

// ============================================================================
// TEST SETUP PARAMS (shared by most test setup functions)
// ============================================================================

#[derive(SystemParam)]
pub struct TestSetupParams<'w, 's> {
    pub slot_alloc: ResMut<'w, GpuSlotPool>,
    pub spawn_events: MessageWriter<'w, SpawnNpcMsg>,
    pub world_data: ResMut<'w, world::WorldData>,
    pub entity_map: ResMut<'w, EntityMap>,
    pub food_storage: ResMut<'w, FoodStorage>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub game_time: ResMut<'w, GameTime>,
    pub test_state: ResMut<'w, TestState>,
    pub world_grid: ResMut<'w, world::WorldGrid>,
    pub camera_q: Query<'w, 's, &'static mut Transform, With<MainCamera>>,
    pub uid_alloc: ResMut<'w, crate::resources::NextEntityUid>,
    pub commands: Commands<'w, 's>,
    pub gpu_updates: MessageWriter<'w, crate::messages::GpuUpdateMsg>,
}

/// Shared test setup params bundle — stays under 16-param limit.
#[derive(SystemParam)]
pub struct BuildingInitParams<'w> {
    pub entity_map: ResMut<'w, EntityMap>,
}

impl TestSetupParams<'_, '_> {
    /// Focus camera on a world position so the test scene is visible.
    pub fn focus_camera(&mut self, x: f32, y: f32) {
        if let Ok(mut cam) = self.camera_q.single_mut() {
            cam.translation.x = x;
            cam.translation.y = y;
        }
    }

    /// Add a default faction-0 town at (400,400).
    /// Auto-inits WorldGrid on first call so building atlas composites correctly.
    pub fn add_town(&mut self, name: &str) {
        if self.world_grid.width == 0 {
            self.world_grid.width = 25;
            self.world_grid.height = 25;
            self.world_grid.cell_size = crate::constants::TOWN_GRID_SPACING;
            self.world_grid.cells = vec![world::WorldCell::default(); 25 * 25];
        }
        if self.entity_map.spatial_cell_size() <= 0.0 {
            let world_size_px = self.world_grid.width as f32 * self.world_grid.cell_size;
            self.entity_map.init_spatial(world_size_px);
        }
        self.world_data.towns.push(world::Town {
            name: name.into(),
            center: Vec2::new(384.0, 384.0),
            faction: 0,
            sprite_type: 0,
            area_level: 0,
        });
    }

    /// Add a bed at the given position for town 0.
    pub fn add_bed(&mut self, x: f32, y: f32) {
        self.add_building(world::BuildingKind::Bed, x, y, 0);
    }

    /// Add a building instance at the given position for a town.
    pub fn add_building(&mut self, kind: world::BuildingKind, x: f32, y: f32, town_idx: u32) {
        let faction = self
            .world_data
            .towns
            .get(town_idx as usize)
            .map(|t| t.faction)
            .unwrap_or(0);
        let _ = world::place_building(
            &mut self.slot_alloc,
            &mut self.entity_map,
            &mut self.uid_alloc,
            &mut self.commands,
            &mut self.gpu_updates,
            kind,
            Vec2::new(x, y),
            town_idx,
            faction,
            0,
            0,
            None,
            None,
            None,
            None,
        );
    }

    /// Add a waypoint with patrol_order at the given position for a town.
    pub fn add_waypoint(&mut self, x: f32, y: f32, town_idx: u32, patrol_order: u32) {
        let faction = self
            .world_data
            .towns
            .get(town_idx as usize)
            .map(|t| t.faction)
            .unwrap_or(0);
        let _ = world::place_building(
            &mut self.slot_alloc,
            &mut self.entity_map,
            &mut self.uid_alloc,
            &mut self.commands,
            &mut self.gpu_updates,
            world::BuildingKind::Waypoint,
            Vec2::new(x, y),
            town_idx,
            faction,
            patrol_order,
            0,
            None,
            None,
            None,
            None,
        );
    }

    /// Initialize pathfind cost grid from terrain + buildings.
    /// Call after all buildings are placed so A* works in the test.
    pub fn finalize_grid(&mut self) {
        self.world_grid.init_pathfind_costs();
        self.world_grid.sync_building_costs(&self.entity_map);
    }

    /// Init food_storage + faction_stats for N towns.
    pub fn init_economy(&mut self, town_count: usize) {
        self.food_storage.init(town_count);
        self.faction_stats.init(town_count);
    }

    /// Alloc a slot and write a SpawnNpcMsg with sensible defaults.
    /// Returns the allocated slot index.
    pub fn spawn_npc(&mut self, job: i32, x: f32, y: f32, home_x: f32, home_y: f32) -> usize {
        let slot = self.slot_alloc.alloc_reset().expect("slot alloc");
        self.spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x,
            y,
            job,
            faction: 0,
            town_idx: 0,
            home_x,
            home_y,
            work_x: -1.0,
            work_y: -1.0,
            starting_post: -1,
            attack_type: 0,
            uid_override: None,
        });
        slot
    }
}

// ============================================================================
// TEST HELPERS
// ============================================================================

use crate::AppState;

// ============================================================================
// TEST STATE
// ============================================================================

/// Shared test state for all tests. Reset between tests.
#[derive(Resource)]
pub struct TestState {
    pub test_name: Option<String>,
    pub phase: u32,
    pub total_phases: u32,
    pub phase_name: String,
    pub start: f32,
    pub results: Vec<TestResult>,
    pub passed: bool,
    pub failed: bool,
    pub counters: HashMap<String, u32>,
    pub flags: HashMap<String, bool>,
    /// When set, test_menu_system will auto-relaunch this test name.
    pub pending_relaunch: Option<String>,
}

impl Default for TestState {
    fn default() -> Self {
        Self {
            test_name: None,
            phase: 0,
            total_phases: 0,
            phase_name: String::new(),
            start: 0.0,
            results: Vec::new(),
            passed: false,
            failed: false,
            counters: HashMap::new(),
            flags: HashMap::new(),
            pending_relaunch: None,
        }
    }
}

impl TestState {
    pub fn pass_phase(&mut self, elapsed: f32, message: impl Into<String>) {
        let msg = message.into();
        info!("  Phase {}: PASS — {}", self.phase, msg);
        self.results.push(TestResult {
            phase: self.phase,
            elapsed,
            message: msg,
            passed: true,
        });
        self.phase += 1;
    }

    pub fn fail_phase(&mut self, elapsed: f32, message: impl Into<String>) {
        let msg = message.into();
        info!("  Phase {}: FAIL — {}", self.phase, msg);
        self.results.push(TestResult {
            phase: self.phase,
            elapsed,
            message: msg,
            passed: false,
        });
        self.failed = true;
    }

    pub fn complete(&mut self, elapsed: f32) {
        self.passed = true;
        info!("========================================");
        info!(
            "Test '{}': ALL {} PHASES PASSED ({:.1}s)",
            self.test_name.as_deref().unwrap_or("?"),
            self.total_phases,
            elapsed
        );
        info!("========================================");
        for r in &self.results {
            let status = if r.passed { "PASS" } else { "FAIL" };
            info!(
                "  Phase {}: {} ({:.1}s) — {}",
                r.phase, status, r.elapsed, r.message
            );
        }
    }

    pub fn set_flag(&mut self, key: &str, value: bool) {
        self.flags.insert(key.to_string(), value);
    }

    pub fn get_flag(&self, key: &str) -> bool {
        self.flags.get(key).copied().unwrap_or(false)
    }

    pub fn inc(&mut self, key: &str) {
        *self.counters.entry(key.to_string()).or_insert(0) += 1;
    }

    pub fn count(&self, key: &str) -> u32 {
        self.counters.get(key).copied().unwrap_or(0)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Tick preamble: returns elapsed seconds, or None if test is done.
    pub fn tick_elapsed(&mut self, time: &Time) -> Option<f32> {
        if self.passed || self.failed {
            return None;
        }
        let now = time.elapsed_secs();
        if self.start == 0.0 {
            self.start = now;
        }
        Some(now - self.start)
    }

    /// Fail if no entities found within 3s. Returns false to signal early return.
    pub fn require_entity(&mut self, count: usize, elapsed: f32, name: &str) -> bool {
        if count == 0 {
            self.phase_name = format!("Waiting for {}...", name);
            if elapsed > 3.0 {
                self.fail_phase(elapsed, format!("No {} entity", name));
            }
            return false;
        }
        true
    }
}

/// Result of a single test phase.
pub struct TestResult {
    pub phase: u32,
    pub elapsed: f32,
    pub message: String,
    pub passed: bool,
}

// ============================================================================
// TEST REGISTRY
// ============================================================================

/// Metadata for a registered test.
pub struct TestEntry {
    pub name: String,
    pub description: String,
    pub phase_count: u32,
    pub time_scale: f32,
}

/// Registry of all available tests.
#[derive(Resource, Default)]
pub struct TestRegistry {
    pub tests: Vec<TestEntry>,
}

/// Run condition: true when the active test matches the given name.
pub fn test_is(name: &'static str) -> impl Fn(Res<TestState>) -> bool {
    move |state: Res<TestState>| state.test_name.as_deref() == Some(name)
}

// ============================================================================
// RUN ALL STATE
// ============================================================================

/// State for running all tests sequentially.
#[derive(Resource, Default)]
pub struct RunAllState {
    pub active: bool,
    pub queue: std::collections::VecDeque<String>,
    pub results: Vec<(String, bool)>,
}

// ============================================================================
// REGISTRATION
// ============================================================================

/// Register all tests and wire systems into the app.
pub fn register_tests(app: &mut App) {
    use crate::Step;

    // Resources (AppState is registered in build_app)
    app.init_resource::<TestState>();
    app.init_resource::<TestRegistry>();
    app.init_resource::<RunAllState>();

    // Menu + HUD UI (must run in EguiPrimaryContextPass, not Update)
    app.add_systems(
        EguiPrimaryContextPass,
        test_menu_system.run_if(in_state(AppState::TestMenu)),
    );
    app.add_systems(
        EguiPrimaryContextPass,
        (
            crate::ui::game_hud::top_bar_system,
            crate::ui::left_panel::left_panel_system,
            crate::ui::game_hud::combat_log_system,
            test_hud_system,
        )
            .chain()
            .run_if(in_state(AppState::Running)),
    );
    app.add_systems(
        Update,
        crate::ui::ui_toggle_system.run_if(in_state(AppState::Running)),
    );
    app.add_systems(OnEnter(AppState::TestMenu), auto_start_next_test);

    // Cleanup when leaving Running — uses same cleanup as game (OnExit Playing)
    app.add_systems(OnExit(AppState::Running), crate::ui::game_cleanup_system);

    // Test completion detection (returns to menu or starts next test)
    app.add_systems(
        FixedUpdate,
        test_completion_system
            .run_if(in_state(AppState::Running))
            .after(Step::Behavior),
    );

    // Register individual tests
    let mut registry = TestRegistry::default();

    // vertical-slice (relocated Test12)
    registry.tests.push(TestEntry {
        name: "vertical-slice".into(),
        description: "Full core loop: spawn → work → raid → combat → death → respawn".into(),
        phase_count: 8,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        vertical_slice::setup.run_if(test_is("vertical-slice")),
    );
    app.add_systems(
        FixedUpdate,
        vertical_slice::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("vertical-slice"))
            .after(Step::Behavior),
    );

    // spawning
    registry.tests.push(TestEntry {
        name: "spawning".into(),
        description: "Spawn 5 NPCs, kill one, slot freed, slot reused".into(),
        phase_count: 4,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        spawning::setup.run_if(test_is("spawning")),
    );
    app.add_systems(
        FixedUpdate,
        spawning::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("spawning"))
            .after(Step::Behavior),
    );

    // energy
    registry.tests.push(TestEntry {
        name: "energy".into(),
        description: "Energy starts at 100, drains, reaches hungry threshold".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        energy::setup.run_if(test_is("energy")),
    );
    app.add_systems(
        FixedUpdate,
        energy::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("energy"))
            .after(Step::Behavior),
    );

    // movement
    registry.tests.push(TestEntry {
        name: "movement".into(),
        description: "Farmer lifecycle: spawn from home, walk to farm, work, rest at home".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        movement::setup.run_if(test_is("movement")),
    );
    app.add_systems(
        FixedUpdate,
        movement::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("movement"))
            .after(Step::Behavior),
    );

    // archer-patrol
    registry.tests.push(TestEntry {
        name: "archer-patrol".into(),
        description: "Archer: OnDuty → Patrol → OnDuty → rest → resume".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        archer_patrol::setup.run_if(test_is("archer-patrol")),
    );
    app.add_systems(
        FixedUpdate,
        archer_patrol::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("archer-patrol"))
            .after(Step::Behavior),
    );

    // farmer-cycle
    registry.tests.push(TestEntry {
        name: "farmer-cycle".into(),
        description: "3 farmer homes + 2 farms: occupancy cap, one farmer idle".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        farmer_cycle::setup.run_if(test_is("farmer-cycle")),
    );
    app.add_systems(
        FixedUpdate,
        farmer_cycle::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("farmer-cycle"))
            .after(Step::Behavior),
    );

    // raider-cycle
    registry.tests.push(TestEntry {
        name: "raider-cycle".into(),
        description: "Raiders: dispatch → steal → return → deliver food".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        raider_cycle::setup.run_if(test_is("raider-cycle")),
    );
    app.add_systems(
        FixedUpdate,
        raider_cycle::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("raider-cycle"))
            .after(Step::Behavior),
    );

    // combat
    registry.tests.push(TestEntry {
        name: "combat".into(),
        description: "GPU targeting → Fighting → damage → death → slot freed".into(),
        phase_count: 6,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        combat::setup.run_if(test_is("combat")),
    );
    app.add_systems(
        FixedUpdate,
        combat::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("combat"))
            .after(Step::Behavior),
    );

    // projectiles
    registry.tests.push(TestEntry {
        name: "projectiles".into(),
        description: "Ranged targeting → projectile spawn → hit → slot freed".into(),
        phase_count: 4,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        projectiles::setup.run_if(test_is("projectiles")),
    );
    app.add_systems(
        FixedUpdate,
        projectiles::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("projectiles"))
            .after(Step::Behavior),
    );

    // archer-tent-reliability
    registry.tests.push(TestEntry {
        name: "archer-tent-reliability".into(),
        description: "Archer vs enemy tent: target lock, projectile activity, sustained tent damage, destruction".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        archer_tent_reliability::setup.run_if(test_is("archer-tent-reliability")),
    );
    app.add_systems(
        FixedUpdate,
        archer_tent_reliability::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("archer-tent-reliability"))
            .after(Step::Behavior),
    );

    // fountain-shot-stale
    registry.tests.push(TestEntry {
        name: "fountain-shot-stale".into(),
        description: "Fountain projectiles: detect alternating stale readback position pattern"
            .into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        fountain_shot_stale::setup.run_if(test_is("fountain-shot-stale")),
    );
    app.add_systems(
        FixedUpdate,
        fountain_shot_stale::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("fountain-shot-stale"))
            .after(Step::Behavior),
    );

    // friendly-fire-buildings
    registry.tests.push(TestEntry {
        name: "friendly-fire-buildings".into(),
        description:
            "Single ranged shooter; same-faction building in shot lane must not take damage".into(),
        phase_count: 4,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        friendly_fire_buildings::setup.run_if(test_is("friendly-fire-buildings")),
    );
    app.add_systems(
        FixedUpdate,
        friendly_fire_buildings::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("friendly-fire-buildings"))
            .after(Step::Behavior),
    );

    // healing
    registry.tests.push(TestEntry {
        name: "healing".into(),
        description: "Damaged NPC near town → Healing → health recovers".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        healing::setup.run_if(test_is("healing")),
    );
    app.add_systems(
        FixedUpdate,
        healing::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("healing"))
            .after(Step::Behavior),
    );

    // economy
    registry.tests.push(TestEntry {
        name: "economy".into(),
        description: "Farm growth → harvest → raider town forage → raider respawn".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        economy::setup.run_if(test_is("economy")),
    );
    app.add_systems(
        FixedUpdate,
        economy::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("economy"))
            .after(Step::Behavior),
    );

    // world-gen
    registry.tests.push(TestEntry {
        name: "world-gen".into(),
        description: "Grid dimensions, town placement, buildings, terrain, raider towns".into(),
        phase_count: 6,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        world_gen::setup.run_if(test_is("world-gen")),
    );
    app.add_systems(
        FixedUpdate,
        world_gen::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("world-gen"))
            .after(Step::Behavior),
    );

    // sleep-visual
    registry.tests.push(TestEntry {
        name: "sleep-visual".into(),
        description: "Resting NPC gets sleep icon, cleared on wake".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        sleep_visual::setup.run_if(test_is("sleep-visual")),
    );
    app.add_systems(
        FixedUpdate,
        sleep_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("sleep-visual")),
    );

    // farm-visual
    registry.tests.push(TestEntry {
        name: "farm-visual".into(),
        description: "Ready farm spawns FarmReadyMarker, removed on harvest".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        farm_visual::setup.run_if(test_is("farm-visual")),
    );
    app.add_systems(
        FixedUpdate,
        farm_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("farm-visual"))
            .after(Step::Behavior),
    );

    // heal-visual
    registry.tests.push(TestEntry {
        name: "heal-visual".into(),
        description: "Healing NPC gets heal icon, cleared when healed".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        heal_visual::setup.run_if(test_is("heal-visual")),
    );
    app.add_systems(
        FixedUpdate,
        heal_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("heal-visual")),
    );

    // npc-visuals
    registry.tests.push(TestEntry {
        name: "npc-visuals".into(),
        description: "Visual showcase: all NPC types with individual layer breakdown".into(),
        phase_count: 1,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        npc_visuals::setup.run_if(test_is("npc-visuals")),
    );
    app.add_systems(
        FixedUpdate,
        npc_visuals::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("npc-visuals")),
    );

    // terrain-visual
    registry.tests.push(TestEntry {
        name: "terrain-visual".into(),
        description: "Visual showcase: all terrain biomes and building types".into(),
        phase_count: 1,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        terrain_visual::setup.run_if(test_is("terrain-visual")),
    );
    app.add_systems(
        FixedUpdate,
        terrain_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("terrain-visual"))
            .after(Step::Behavior),
    );

    // endless-mode
    registry.tests.push(TestEntry {
        name: "endless-mode".into(),
        description: "Builder + raider fountain destroyed → spawn queued → migration → settle"
            .into(),
        phase_count: 14,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        endless_mode::setup.run_if(test_is("endless-mode")),
    );
    app.add_systems(
        FixedUpdate,
        endless_mode::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("endless-mode"))
            .after(Step::Behavior),
    );

    // ai-building
    registry.tests.push(TestEntry {
        name: "ai-building".into(),
        description: "AI town building: pick personality, watch it build with 100K food+gold"
            .into(),
        phase_count: 2,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        ai_building::setup.run_if(test_is("ai-building")),
    );
    app.add_systems(
        EguiPrimaryContextPass,
        ai_building::ui
            .run_if(in_state(AppState::Running))
            .run_if(test_is("ai-building")),
    );
    app.add_systems(
        FixedUpdate,
        ai_building::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("ai-building"))
            .after(Step::Behavior),
    );

    // miner-cycle
    registry.tests.push(TestEntry {
        name: "miner-cycle".into(),
        description: "Miner: walk to mine → tend → harvest gold → deliver → rest".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        miner_cycle::setup.run_if(test_is("miner-cycle")),
    );
    app.add_systems(
        FixedUpdate,
        miner_cycle::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("miner-cycle"))
            .after(Step::Behavior),
    );

    // slot-reuse-wave
    registry.tests.push(TestEntry {
        name: "slot-reuse-wave".into(),
        description: "AI wave vs destroyed building: slot reuse ABA prevents wave end".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        slot_reuse_wave::setup.run_if(test_is("slot-reuse-wave")),
    );
    app.add_systems(
        FixedUpdate,
        slot_reuse_wave::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("slot-reuse-wave"))
            .after(Step::Behavior),
    );

    // coalesce-movement
    registry.tests.push(TestEntry {
        name: "coalesce-movement".into(),
        description: "SetPosition on unused slot must not teleport moving NPCs".into(),
        phase_count: 2,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        coalesce_safety::setup_movement.run_if(test_is("coalesce-movement")),
    );
    app.add_systems(
        FixedUpdate,
        coalesce_safety::tick_movement
            .run_if(in_state(AppState::Running))
            .run_if(test_is("coalesce-movement"))
            .after(Step::Behavior),
    );

    // coalesce-arrival
    registry.tests.push(TestEntry {
        name: "coalesce-arrival".into(),
        description: "Arrival flags not reset for non-dirty slots".into(),
        phase_count: 2,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        coalesce_safety::setup_arrival.run_if(test_is("coalesce-arrival")),
    );
    app.add_systems(
        FixedUpdate,
        coalesce_safety::tick_arrival
            .run_if(in_state(AppState::Running))
            .run_if(test_is("coalesce-arrival"))
            .after(Step::Behavior),
    );

    // sandbox
    registry.tests.push(TestEntry {
        name: "sandbox".into(),
        description: "Human player sandbox: 1 town, 100K food+gold, no AI/raiders".into(),
        phase_count: 1,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        sandbox::setup.run_if(test_is("sandbox")),
    );
    app.add_systems(
        FixedUpdate,
        sandbox::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("sandbox"))
            .after(Step::Behavior),
    );

    // stress-archer-towns
    app.init_resource::<stress_archer_towns::StressArcherConfig>();
    registry.tests.push(TestEntry {
        name: "stress-archer-towns".into(),
        description: "Stress scene: 20 AI builder towns, configurable archer homes".into(),
        phase_count: 1,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        stress_archer_towns::setup.run_if(test_is("stress-archer-towns")),
    );
    app.add_systems(
        FixedUpdate,
        stress_archer_towns::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("stress-archer-towns"))
            .after(Step::Behavior),
    );
    app.add_systems(
        EguiPrimaryContextPass,
        stress_archer_towns::ui
            .run_if(in_state(AppState::Running))
            .run_if(test_is("stress-archer-towns")),
    );

    // pathfind-maze
    app.init_resource::<pathfind_maze::PathfindMazeConfig>();
    registry.tests.push(TestEntry {
        name: "pathfind-maze".into(),
        description: "NPCs navigate serpentine wall maze via A* pathfinding (configurable count)".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        pathfind_maze::setup.run_if(test_is("pathfind-maze")),
    );
    app.add_systems(
        FixedUpdate,
        pathfind_maze::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("pathfind-maze"))
            .after(Step::Behavior),
    );
    app.add_systems(
        EguiPrimaryContextPass,
        pathfind_maze::ui
            .run_if(in_state(AppState::Running))
            .run_if(test_is("pathfind-maze")),
    );

    // loot-cycle
    registry.tests.push(TestEntry {
        name: "loot-cycle".into(),
        description: "Kill raider → carry loot → deposit → equip → stats increase".into(),
        phase_count: 6,
        time_scale: 1.0,
    });
    app.add_systems(
        OnEnter(AppState::Running),
        loot_cycle::setup.run_if(test_is("loot-cycle")),
    );
    app.add_systems(
        FixedUpdate,
        loot_cycle::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("loot-cycle"))
            .after(Step::Behavior),
    );

    // Common test-world materialization:
    app.insert_resource(registry);
}

// ============================================================================
// TEST MENU UI
// ============================================================================

fn test_menu_system(
    mut contexts: EguiContexts,
    registry: Res<TestRegistry>,
    mut test_state: ResMut<TestState>,
    mut run_all: ResMut<RunAllState>,
    mut next_state: ResMut<NextState<AppState>>,
) -> Result {
    // Auto-relaunch if a test requested restart
    if let Some(name) = test_state.pending_relaunch.take() {
        if let Some(entry) = registry.tests.iter().find(|t| t.name == name) {
            start_test(&name, entry.phase_count, entry.time_scale, &mut test_state, &mut next_state);
            return Ok(());
        }
    }

    let ctx = contexts.ctx_mut()?;
    egui::CentralPanel::default().show(ctx, |ui| {
        ui.heading("Endless — Test Framework");
        ui.separator();

        // Show Run All results if we just finished
        if !run_all.active && !run_all.results.is_empty() {
            ui.label(egui::RichText::new("Run All Results").strong().size(16.0));
            for (name, passed) in &run_all.results {
                let (icon, color) = if *passed {
                    ("✓", egui::Color32::GREEN)
                } else {
                    ("✗", egui::Color32::RED)
                };
                ui.colored_label(color, format!("{} {}", icon, name));
            }
            let total = run_all.results.len();
            let passed = run_all.results.iter().filter(|(_, p)| *p).count();
            ui.separator();
            let summary_color = if passed == total {
                egui::Color32::GREEN
            } else {
                egui::Color32::RED
            };
            ui.colored_label(summary_color, format!("{}/{} passed", passed, total));
            ui.separator();
        }

        // Test list
        ui.label(egui::RichText::new("Tests").strong().size(14.0));
        ui.add_space(4.0);

        for entry in &registry.tests {
            ui.horizontal(|ui| {
                if ui.button(&entry.name).clicked() {
                    start_test(
                        &entry.name,
                        entry.phase_count,
                        entry.time_scale,
                        &mut test_state,
                        &mut next_state,
                    );
                }
                ui.label(format!(
                    "({} phases) {}",
                    entry.phase_count, entry.description
                ));
            });
        }

        ui.add_space(16.0);
        ui.separator();

        // Run All button
        ui.horizontal(|ui| {
            if ui.button("Run All").clicked() {
                run_all.active = true;
                run_all.results.clear();
                run_all.queue = registry.tests.iter().map(|t| t.name.clone()).collect();
                // Start first test
                if let Some(first) = run_all.queue.pop_front() {
                    let entry = registry.tests.iter().find(|t| t.name == first).unwrap();
                    start_test(
                        &first,
                        entry.phase_count,
                        entry.time_scale,
                        &mut test_state,
                        &mut next_state,
                    );
                }
            }

            ui.add_space(20.0);

            if ui.button("Back to Menu").clicked() {
                next_state.set(AppState::MainMenu);
            }
        });
    });
    Ok(())
}

fn start_test(
    name: &str,
    phase_count: u32,
    time_scale: f32,
    test_state: &mut TestState,
    next_state: &mut NextState<AppState>,
) {
    test_state.reset();
    test_state.test_name = Some(name.to_string());
    test_state.phase = 1;
    test_state.total_phases = phase_count;
    info!(
        "Starting test: {} ({} phases, time_scale={})",
        name, phase_count, time_scale
    );
    next_state.set(AppState::Running);
}

// ============================================================================
// TEST HUD (overlay during test execution)
// ============================================================================

fn test_hud_system(
    mut contexts: EguiContexts,
    test_state: Res<TestState>,
    time: Res<Time>,
    mut next_state: ResMut<NextState<AppState>>,
) -> Result {
    let elapsed = if test_state.start > 0.0 {
        time.elapsed_secs() - test_state.start
    } else {
        0.0
    };

    let ctx = contexts.ctx_mut()?;
    egui::Window::new("Test")
        .anchor(egui::Align2::RIGHT_TOP, [-8.0, 40.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            let name = test_state.test_name.as_deref().unwrap_or("?");
            ui.label(egui::RichText::new(name).strong().size(16.0));
            ui.label(format!(
                "Phase {}/{} — {:.1}s",
                test_state.phase.min(test_state.total_phases),
                test_state.total_phases,
                elapsed
            ));

            // Phase checklist — show all phases, check off as they complete
            ui.add_space(4.0);
            for p in 1..=test_state.total_phases {
                let result = test_state.results.iter().find(|r| r.phase == p);
                let (icon, color, label) = if let Some(r) = result {
                    if r.passed {
                        (
                            "✓",
                            egui::Color32::GREEN,
                            format!("Phase {} ({:.1}s): {}", p, r.elapsed, r.message),
                        )
                    } else {
                        (
                            "✗",
                            egui::Color32::RED,
                            format!("Phase {} ({:.1}s): {}", p, r.elapsed, r.message),
                        )
                    }
                } else if test_state.phase == p {
                    (
                        "▶",
                        egui::Color32::YELLOW,
                        format!("Phase {}: {}", p, test_state.phase_name),
                    )
                } else {
                    ("○", egui::Color32::GRAY, format!("Phase {}", p))
                };
                ui.colored_label(color, format!("{} {}", icon, label));
            }

            // Overall status
            ui.add_space(4.0);
            if test_state.passed {
                ui.colored_label(
                    egui::Color32::GREEN,
                    egui::RichText::new("ALL PASSED").strong().size(14.0),
                );
            } else if test_state.failed {
                ui.colored_label(
                    egui::Color32::RED,
                    egui::RichText::new("FAILED").strong().size(14.0),
                );
            }

            ui.separator();
            let label = if test_state.passed || test_state.failed {
                "Back"
            } else {
                "Cancel"
            };
            if ui.button(label).clicked() {
                next_state.set(AppState::TestMenu);
            }
        });
    Ok(())
}

// ============================================================================
// TEST COMPLETION
// ============================================================================

/// Detects when a test passes or fails and handles transition.
/// Single tests stay running (user clicks Back in HUD). Run All auto-advances.
fn test_completion_system(
    test_state: Res<TestState>,
    mut run_all: ResMut<RunAllState>,
    mut next_state: ResMut<NextState<AppState>>,
    mut delayed: Local<Option<f32>>,
    time: Res<Time>,
) {
    if !test_state.passed && !test_state.failed {
        *delayed = None;
        return;
    }

    // Single test: stay running — user clicks Back in HUD to return
    if !run_all.active {
        return;
    }

    // Run All mode: auto-advance after 1.5s delay
    let now = time.elapsed_secs();
    if delayed.is_none() {
        *delayed = Some(now);
    }
    if now - delayed.unwrap() < 1.5 {
        return;
    }

    // Record result for Run All
    let name = test_state.test_name.clone().unwrap_or_default();
    run_all.results.push((name, test_state.passed));

    if run_all.queue.is_empty() {
        run_all.active = false;
        info!(
            "Run All: complete — {}/{} passed",
            run_all.results.iter().filter(|(_, p)| *p).count(),
            run_all.results.len()
        );
    }

    next_state.set(AppState::TestMenu);
    *delayed = None;
}

// ============================================================================
// AUTO-START NEXT TEST (for Run All)
// ============================================================================

/// When entering TestMenu with run_all.active, auto-start the next queued test.
pub fn auto_start_next_test(
    mut run_all: ResMut<RunAllState>,
    registry: Res<TestRegistry>,
    mut test_state: ResMut<TestState>,
    mut next_state: ResMut<NextState<AppState>>,
) {
    if !run_all.active {
        return;
    }
    if let Some(next_name) = run_all.queue.pop_front() {
        if let Some(entry) = registry.tests.iter().find(|t| t.name == next_name) {
            start_test(
                &next_name,
                entry.phase_count,
                entry.time_scale,
                &mut test_state,
                &mut next_state,
            );
        }
    } else {
        run_all.active = false;
    }
}

