//! Test Framework - UI-selectable integration tests for Endless systems.
//!
//! Each test is a file in src/tests/ exporting setup (OnEnter) + tick (Update) systems.
//! Tests are selected from a bevy_egui menu and run within the full Bevy app.

pub mod vertical_slice;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPrimaryContextPass, egui};
use std::collections::HashMap;

use crate::components::NpcIndex;

// ============================================================================
// SYSTEM PARAM BUNDLES (keeps cleanup under 16-param limit)
// ============================================================================

#[derive(SystemParam)]
pub struct CleanupCore<'w> {
    pub npc_count: ResMut<'w, crate::resources::NpcCount>,
    pub slot_alloc: ResMut<'w, crate::resources::SlotAllocator>,
    pub world_data: ResMut<'w, crate::world::WorldData>,
    pub food_storage: ResMut<'w, crate::resources::FoodStorage>,
    pub farm_states: ResMut<'w, crate::resources::FarmStates>,
    pub faction_stats: ResMut<'w, crate::resources::FactionStats>,
    pub gpu_state: ResMut<'w, crate::resources::GpuReadState>,
    pub game_time: ResMut<'w, crate::resources::GameTime>,
}

#[derive(SystemParam)]
pub struct CleanupExtra<'w> {
    pub combat_debug: ResMut<'w, crate::resources::CombatDebug>,
    pub health_debug: ResMut<'w, crate::resources::HealthDebug>,
    pub kill_stats: ResMut<'w, crate::resources::KillStats>,
    pub bed_occ: ResMut<'w, crate::world::BedOccupancy>,
    pub farm_occ: ResMut<'w, crate::world::FarmOccupancy>,
    pub camp_state: ResMut<'w, crate::resources::CampState>,
    pub raid_queue: ResMut<'w, crate::resources::RaidQueue>,
    pub proj_alloc: ResMut<'w, crate::resources::ProjSlotAllocator>,
}

// ============================================================================
// APP STATE
// ============================================================================

/// Two-state machine: menu or running a test.
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum AppState {
    #[default]
    TestMenu,
    Running,
}

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

    // Resources
    app.init_state::<AppState>();
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
        time_scale: 10.0,
    });
    app.add_systems(OnEnter(AppState::Running),
        vertical_slice::setup.run_if(test_is("vertical-slice")));
    app.add_systems(Update,
        vertical_slice::tick
            .run_if(in_state(AppState::Running))
            .run_if(test_is("vertical-slice"))
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
            if ui.button("Cancel").clicked() {
                next_state.set(AppState::TestMenu);
            }
        });
    Ok(())
}

// ============================================================================
// TEST COMPLETION
// ============================================================================

/// Detects when a test passes or fails and handles transition.
fn test_completion_system(
    test_state: Res<TestState>,
    mut run_all: ResMut<RunAllState>,
    registry: Res<TestRegistry>,
    mut next_state: ResMut<NextState<AppState>>,
    mut delayed: Local<Option<f32>>,
    time: Res<Time>,
) {
    if !test_state.passed && !test_state.failed {
        *delayed = None;
        return;
    }

    // Delay 1.5s so user can see the result
    let now = time.elapsed_secs();
    if delayed.is_none() {
        *delayed = Some(now);
    }
    if now - delayed.unwrap() < 1.5 {
        return;
    }

    // Record result for Run All
    if run_all.active {
        let name = test_state.test_name.clone().unwrap_or_default();
        run_all.results.push((name, test_state.passed));

        // Start next test or finish
        if let Some(next_name) = run_all.queue.pop_front() {
            if let Some(_entry) = registry.tests.iter().find(|t| t.name == next_name) {
                // Can't call start_test here because we need mutable test_state
                // but it's only Res. The state transition + OnEnter will handle it.
                // We'll set it up in a follow-up frame via OnExit/OnEnter cycle.
                info!("Run All: next test '{}'", next_name);
                // Transition to menu briefly, then back to running
                // Actually, we just go to menu — the menu system detects run_all.active
                // and auto-starts the next test.
            }
        } else {
            run_all.active = false;
            info!("Run All: complete — {}/{} passed",
                run_all.results.iter().filter(|(_, p)| *p).count(),
                run_all.results.len());
        }
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
    npc_query: Query<Entity, With<NpcIndex>>,
    mut core: CleanupCore,
    mut extra: CleanupExtra,
) {
    let count = npc_query.iter().count();
    for entity in npc_query.iter() {
        commands.entity(entity).despawn();
    }

    *core.npc_count = Default::default();
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
    *extra.bed_occ = Default::default();
    *extra.farm_occ = Default::default();
    *extra.camp_state = Default::default();
    *extra.raid_queue = Default::default();
    *extra.proj_alloc = Default::default();

    info!("Test cleanup: despawned {} NPCs, reset resources", count);
}
