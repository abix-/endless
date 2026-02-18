//! Test Framework - UI-selectable integration tests for Endless systems.
//!
//! Each test is a file in src/tests/ exporting setup (OnEnter) + tick (Update) systems.
//! Tests are selected from a bevy_egui menu and run within the full Bevy app.

pub mod vertical_slice;
pub mod spawning;
pub mod energy;
pub mod movement;
pub mod archer_patrol;
pub mod farmer_cycle;
pub mod raider_cycle;
pub mod combat;
pub mod projectiles;
pub mod fountain_shot_stale;
pub mod friendly_fire_buildings;
pub mod healing;
pub mod economy;
pub mod world_gen;
pub mod sleep_visual;
pub mod farm_visual;
pub mod heal_visual;
pub mod npc_visuals;
pub mod terrain_visual;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use std::collections::HashMap;

use crate::components::{NpcIndex, FarmReadyMarker};
use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world;

// ============================================================================
// SYSTEM PARAM BUNDLES (keeps cleanup under 16-param limit)
// ============================================================================

#[derive(SystemParam)]
pub struct CleanupCore<'w> {
    pub slot_alloc: ResMut<'w, crate::resources::SlotAllocator>,
    pub world_data: ResMut<'w, crate::world::WorldData>,
    pub food_storage: ResMut<'w, crate::resources::FoodStorage>,
    pub farm_states: ResMut<'w, crate::resources::GrowthStates>,
    pub faction_stats: ResMut<'w, crate::resources::FactionStats>,
    pub gpu_state: ResMut<'w, crate::resources::GpuReadState>,
    pub game_time: ResMut<'w, crate::resources::GameTime>,
}

#[derive(SystemParam)]
pub struct CleanupExtra<'w> {
    pub combat_debug: ResMut<'w, crate::resources::CombatDebug>,
    pub health_debug: ResMut<'w, crate::resources::HealthDebug>,
    pub kill_stats: ResMut<'w, crate::resources::KillStats>,
    pub farm_occ: ResMut<'w, crate::world::BuildingOccupancy>,
    pub camp_state: ResMut<'w, crate::resources::CampState>,
    pub raid_queue: ResMut<'w, crate::resources::RaidQueue>,
    pub proj_alloc: ResMut<'w, crate::resources::ProjSlotAllocator>,
    pub world_grid: ResMut<'w, crate::world::WorldGrid>,
    pub debug_flags: ResMut<'w, crate::resources::DebugFlags>,
    pub spawner_state: ResMut<'w, crate::resources::SpawnerState>,
    pub tilemap_spawned: ResMut<'w, crate::render::TilemapSpawned>,
    pub building_hp: ResMut<'w, crate::resources::BuildingHpState>,
}

// ============================================================================
// TEST SETUP PARAMS (shared by most test setup functions)
// ============================================================================

#[derive(SystemParam)]
pub struct TestSetupParams<'w> {
    pub slot_alloc: ResMut<'w, SlotAllocator>,
    pub spawn_events: MessageWriter<'w, SpawnNpcMsg>,
    pub world_data: ResMut<'w, world::WorldData>,
    pub food_storage: ResMut<'w, FoodStorage>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub game_time: ResMut<'w, GameTime>,
    pub test_state: ResMut<'w, TestState>,
    pub spawner_state: ResMut<'w, SpawnerState>,
}

impl TestSetupParams<'_> {
    /// Add a default faction-0 town at (400,400).
    pub fn add_town(&mut self, name: &str) {
        self.world_data.towns.push(world::Town {
            name: name.into(),
            center: Vec2::new(400.0, 400.0),
            faction: 0,
            sprite_type: 0,
        });
    }

    /// Add a bed at the given position for town 0.
    pub fn add_bed(&mut self, x: f32, y: f32) {
        self.world_data.beds.push(world::Bed {
            position: Vec2::new(x, y),
            town_idx: 0,
        });
    }

    /// Init food_storage + faction_stats for N towns.
    pub fn init_economy(&mut self, town_count: usize) {
        self.food_storage.init(town_count);
        self.faction_stats.init(town_count);
    }

    /// Alloc a slot and write a SpawnNpcMsg with sensible defaults.
    /// Returns the allocated slot index.
    pub fn spawn_npc(&mut self, job: i32, x: f32, y: f32, home_x: f32, home_y: f32) -> usize {
        let slot = self.slot_alloc.alloc().expect("slot alloc");
        self.spawn_events.write(SpawnNpcMsg {
            slot_idx: slot,
            x, y,
            job, faction: 0, town_idx: 0,
            home_x, home_y,
            work_x: -1.0, work_y: -1.0,
            starting_post: -1,
            attack_type: 0,
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
        info!("Test '{}': ALL {} PHASES PASSED ({:.1}s)",
            self.test_name.as_deref().unwrap_or("?"), self.total_phases, elapsed);
        info!("========================================");
        for r in &self.results {
            let status = if r.passed { "PASS" } else { "FAIL" };
            info!("  Phase {}: {} ({:.1}s) — {}", r.phase, status, r.elapsed, r.message);
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
        if self.passed || self.failed { return None; }
        let now = time.elapsed_secs();
        if self.start == 0.0 { self.start = now; }
        Some(now - self.start)
    }

    /// Fail if no entities found within 3s. Returns false to signal early return.
    pub fn require_entity(&mut self, count: usize, elapsed: f32, name: &str) -> bool {
        if count == 0 {
            self.phase_name = format!("Waiting for {}...", name);
            if elapsed > 3.0 { self.fail_phase(elapsed, format!("No {} entity", name)); }
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
    move |state: Res<TestState>| {
        state.test_name.as_deref() == Some(name)
    }
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
    app.add_systems(EguiPrimaryContextPass, test_menu_system.run_if(in_state(AppState::TestMenu)));
    app.add_systems(EguiPrimaryContextPass, test_hud_system.run_if(in_state(AppState::Running)));
    app.add_systems(OnEnter(AppState::TestMenu), auto_start_next_test);

    // Cleanup when leaving Running
    app.add_systems(OnExit(AppState::Running), cleanup_test_world);

    // Test completion detection (returns to menu or starts next test)
    app.add_systems(Update, test_completion_system
        .run_if(in_state(AppState::Running))
        .after(Step::Behavior));

    // Register individual tests
    let mut registry = TestRegistry::default();

    // vertical-slice (relocated Test12)
    registry.tests.push(TestEntry {
        name: "vertical-slice".into(),
        description: "Full core loop: spawn → work → raid → combat → death → respawn".into(),
        phase_count: 8,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        vertical_slice::setup.run_if(test_is("vertical-slice")));
    app.add_systems(Update,
        vertical_slice::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("vertical-slice"))
            .after(Step::Behavior));

    // spawning
    registry.tests.push(TestEntry {
        name: "spawning".into(),
        description: "Spawn 5 NPCs, kill one, slot freed, slot reused".into(),
        phase_count: 4,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        spawning::setup.run_if(test_is("spawning")));
    app.add_systems(Update,
        spawning::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("spawning"))
            .after(Step::Behavior));

    // energy
    registry.tests.push(TestEntry {
        name: "energy".into(),
        description: "Energy starts at 100, drains, reaches hungry threshold".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        energy::setup.run_if(test_is("energy")));
    app.add_systems(Update,
        energy::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("energy"))
            .after(Step::Behavior));

    // movement
    registry.tests.push(TestEntry {
        name: "movement".into(),
        description: "NPCs get targets, GPU moves them, arrive at destination".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        movement::setup.run_if(test_is("movement")));
    app.add_systems(Update,
        movement::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("movement"))
            .after(Step::Behavior));

    // archer-patrol
    registry.tests.push(TestEntry {
        name: "archer-patrol".into(),
        description: "Archer: OnDuty → Patrol → OnDuty → rest → resume".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        archer_patrol::setup.run_if(test_is("archer-patrol")));
    app.add_systems(Update,
        archer_patrol::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("archer-patrol"))
            .after(Step::Behavior));

    // farmer-cycle
    registry.tests.push(TestEntry {
        name: "farmer-cycle".into(),
        description: "Farmer: work → tired → rest → recover → return".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        farmer_cycle::setup.run_if(test_is("farmer-cycle")));
    app.add_systems(Update,
        farmer_cycle::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("farmer-cycle"))
            .after(Step::Behavior));

    // raider-cycle
    registry.tests.push(TestEntry {
        name: "raider-cycle".into(),
        description: "Raiders: dispatch → steal → return → deliver food".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        raider_cycle::setup.run_if(test_is("raider-cycle")));
    app.add_systems(Update,
        raider_cycle::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("raider-cycle"))
            .after(Step::Behavior));

    // combat
    registry.tests.push(TestEntry {
        name: "combat".into(),
        description: "GPU targeting → Fighting → damage → death → slot freed".into(),
        phase_count: 6,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        combat::setup.run_if(test_is("combat")));
    app.add_systems(Update,
        combat::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("combat"))
            .after(Step::Behavior));

    // projectiles
    registry.tests.push(TestEntry {
        name: "projectiles".into(),
        description: "Ranged targeting → projectile spawn → hit → slot freed".into(),
        phase_count: 4,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        projectiles::setup.run_if(test_is("projectiles")));
    app.add_systems(Update,
        projectiles::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("projectiles"))
            .after(Step::Behavior));

    // fountain-shot-stale
    registry.tests.push(TestEntry {
        name: "fountain-shot-stale".into(),
        description: "Fountain projectiles: detect alternating stale readback position pattern".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        fountain_shot_stale::setup.run_if(test_is("fountain-shot-stale")));
    app.add_systems(Update,
        fountain_shot_stale::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("fountain-shot-stale"))
            .after(Step::Behavior));

    // friendly-fire-buildings
    registry.tests.push(TestEntry {
        name: "friendly-fire-buildings".into(),
        description: "Single ranged shooter; same-faction building in shot lane must not take damage".into(),
        phase_count: 4,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        friendly_fire_buildings::setup.run_if(test_is("friendly-fire-buildings")));
    app.add_systems(Update,
        friendly_fire_buildings::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("friendly-fire-buildings"))
            .after(Step::Behavior));

    // healing
    registry.tests.push(TestEntry {
        name: "healing".into(),
        description: "Damaged NPC near town → Healing → health recovers".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        healing::setup.run_if(test_is("healing")));
    app.add_systems(Update,
        healing::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("healing"))
            .after(Step::Behavior));

    // economy
    registry.tests.push(TestEntry {
        name: "economy".into(),
        description: "Farm growth → harvest → camp forage → raider respawn".into(),
        phase_count: 5,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        economy::setup.run_if(test_is("economy")));
    app.add_systems(Update,
        economy::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("economy"))
            .after(Step::Behavior));

    // world-gen
    registry.tests.push(TestEntry {
        name: "world-gen".into(),
        description: "Grid dimensions, town placement, buildings, terrain, camps".into(),
        phase_count: 6,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        world_gen::setup.run_if(test_is("world-gen")));
    app.add_systems(Update,
        world_gen::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("world-gen"))
            .after(Step::Behavior));

    // sleep-visual
    registry.tests.push(TestEntry {
        name: "sleep-visual".into(),
        description: "Resting NPC gets sleep icon, cleared on wake".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        sleep_visual::setup.run_if(test_is("sleep-visual")));
    app.add_systems(Update,
        sleep_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("sleep-visual"))
);

    // farm-visual
    registry.tests.push(TestEntry {
        name: "farm-visual".into(),
        description: "Ready farm spawns FarmReadyMarker, removed on harvest".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        farm_visual::setup.run_if(test_is("farm-visual")));
    app.add_systems(Update,
        farm_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("farm-visual"))
            .after(Step::Behavior));

    // heal-visual
    registry.tests.push(TestEntry {
        name: "heal-visual".into(),
        description: "Healing NPC gets heal icon, cleared when healed".into(),
        phase_count: 3,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        heal_visual::setup.run_if(test_is("heal-visual")));
    app.add_systems(Update,
        heal_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("heal-visual"))
);

    // npc-visuals
    registry.tests.push(TestEntry {
        name: "npc-visuals".into(),
        description: "Visual showcase: all NPC types with individual layer breakdown".into(),
        phase_count: 1,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        npc_visuals::setup.run_if(test_is("npc-visuals")));
    app.add_systems(Update,
        npc_visuals::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("npc-visuals"))
);

    // terrain-visual
    registry.tests.push(TestEntry {
        name: "terrain-visual".into(),
        description: "Visual showcase: all terrain biomes and building types".into(),
        phase_count: 1,
        time_scale: 1.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        terrain_visual::setup.run_if(test_is("terrain-visual")));
    app.add_systems(Update,
        terrain_visual::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("terrain-visual"))
            .after(Step::Behavior));

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
            let summary_color = if passed == total { egui::Color32::GREEN } else { egui::Color32::RED };
            ui.colored_label(summary_color, format!("{}/{} passed", passed, total));
            ui.separator();
        }

        // Test list
        ui.label(egui::RichText::new("Tests").strong().size(14.0));
        ui.add_space(4.0);

        for entry in &registry.tests {
            ui.horizontal(|ui| {
                if ui.button(&entry.name).clicked() {
                    start_test(&entry.name, entry.phase_count, entry.time_scale,
                        &mut test_state, &mut next_state);
                }
                ui.label(format!("({} phases) {}", entry.phase_count, entry.description));
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
                    start_test(&first, entry.phase_count, entry.time_scale,
                        &mut test_state, &mut next_state);
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
    info!("Starting test: {} ({} phases, time_scale={})", name, phase_count, time_scale);
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
        .anchor(egui::Align2::LEFT_TOP, [8.0, 8.0])
        .resizable(false)
        .collapsible(false)
        .show(ctx, |ui| {
            let name = test_state.test_name.as_deref().unwrap_or("?");
            ui.label(egui::RichText::new(name).strong().size(16.0));
            ui.label(format!("Phase {}/{} — {:.1}s",
                test_state.phase.min(test_state.total_phases),
                test_state.total_phases,
                elapsed));

            // Phase checklist — show all phases, check off as they complete
            ui.add_space(4.0);
            for p in 1..=test_state.total_phases {
                let result = test_state.results.iter().find(|r| r.phase == p);
                let (icon, color, label) = if let Some(r) = result {
                    if r.passed {
                        ("✓", egui::Color32::GREEN, format!("Phase {} ({:.1}s): {}", p, r.elapsed, r.message))
                    } else {
                        ("✗", egui::Color32::RED, format!("Phase {} ({:.1}s): {}", p, r.elapsed, r.message))
                    }
                } else if test_state.phase == p {
                    ("▶", egui::Color32::YELLOW, format!("Phase {}: {}", p, test_state.phase_name))
                } else {
                    ("○", egui::Color32::GRAY, format!("Phase {}", p))
                };
                ui.colored_label(color, format!("{} {}", icon, label));
            }

            // Overall status
            ui.add_space(4.0);
            if test_state.passed {
                ui.colored_label(egui::Color32::GREEN,
                    egui::RichText::new("ALL PASSED").strong().size(14.0));
            } else if test_state.failed {
                ui.colored_label(egui::Color32::RED,
                    egui::RichText::new("FAILED").strong().size(14.0));
            }

            ui.separator();
            let label = if test_state.passed || test_state.failed { "Back" } else { "Cancel" };
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
        info!("Run All: complete — {}/{} passed",
            run_all.results.iter().filter(|(_, p)| *p).count(),
            run_all.results.len());
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
            start_test(&next_name, entry.phase_count, entry.time_scale,
                &mut test_state, &mut next_state);
        }
    } else {
        run_all.active = false;
    }
}

// ============================================================================
// CLEANUP
// ============================================================================

/// Despawn all NPC entities and reset resources when leaving a test.
fn cleanup_test_world(
    mut commands: Commands,
    entity_query: Query<Entity, Or<(With<NpcIndex>, With<FarmReadyMarker>)>>,
    tilemap_query: Query<Entity, With<crate::render::TerrainChunk>>,
    mut core: CleanupCore,
    mut extra: CleanupExtra,
) {
    let count = entity_query.iter().count();
    for entity in entity_query.iter() {
        commands.entity(entity).despawn();
    }
    let tilemap_count = tilemap_query.iter().count();
    for entity in tilemap_query.iter() {
        commands.entity(entity).despawn();
    }

    *core.slot_alloc = Default::default();
    *core.world_data = Default::default();
    *core.food_storage = Default::default();
    *core.farm_states = Default::default();
    *core.faction_stats = Default::default();
    *core.gpu_state = Default::default();
    *core.game_time = Default::default();

    *extra.combat_debug = Default::default();
    *extra.health_debug = Default::default();
    *extra.kill_stats = Default::default();
    *extra.farm_occ = Default::default();
    *extra.camp_state = Default::default();
    *extra.raid_queue = Default::default();
    *extra.proj_alloc = Default::default();
    *extra.world_grid = Default::default();
    *extra.debug_flags = Default::default();
    *extra.spawner_state = Default::default();
    *extra.building_hp = Default::default();
    extra.tilemap_spawned.0 = false;

    info!("Test cleanup: despawned {} NPCs + {} tilemap chunks, reset resources", count, tilemap_count);
}



