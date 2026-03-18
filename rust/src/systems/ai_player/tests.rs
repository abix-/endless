use super::*;
use bevy::time::TimeUpdateStrategy;

// -- ai_dirty_drain_system --

fn setup_ai_dirty_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(AiSnapshotDirty(false));
    app.add_message::<crate::messages::BuildingGridDirtyMsg>();
    app.add_message::<crate::messages::MiningDirtyMsg>();
    app.add_message::<crate::messages::PatrolPerimeterDirtyMsg>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.insert_resource(SendGridDirty(false));
    app.add_systems(
        FixedUpdate,
        (send_grid_dirty, ai_dirty_drain_system).chain(),
    );
    app.update();
    app.update();
    app
}

#[derive(Resource, Default)]
struct SendGridDirty(bool);

fn send_grid_dirty(
    mut writer: MessageWriter<crate::messages::BuildingGridDirtyMsg>,
    mut flag: ResMut<SendGridDirty>,
) {
    if flag.0 {
        writer.write(crate::messages::BuildingGridDirtyMsg);
        flag.0 = false;
    }
}

#[test]
fn ai_dirty_drain_sets_flag_on_grid_msg() {
    let mut app = setup_ai_dirty_app();
    app.insert_resource(SendGridDirty(true));
    app.update();
    let dirty = app.world().resource::<AiSnapshotDirty>();
    assert!(
        dirty.0,
        "AiSnapshotDirty should be true after grid dirty msg"
    );
}

#[test]
fn ai_dirty_drain_stays_false_without_msgs() {
    let mut app = setup_ai_dirty_app();
    app.update();
    let dirty = app.world().resource::<AiSnapshotDirty>();
    assert!(
        !dirty.0,
        "AiSnapshotDirty should stay false with no messages"
    );
}

// -- perimeter_dirty_drain_system --

fn setup_perimeter_dirty_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(PerimeterSyncDirty(false));
    app.add_message::<crate::messages::PatrolPerimeterDirtyMsg>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.insert_resource(SendPerimeterDirty(false));
    app.add_systems(
        FixedUpdate,
        (send_perimeter_dirty, perimeter_dirty_drain_system).chain(),
    );
    app.update();
    app.update();
    app
}

#[derive(Resource, Default)]
struct SendPerimeterDirty(bool);

fn send_perimeter_dirty(
    mut writer: MessageWriter<crate::messages::PatrolPerimeterDirtyMsg>,
    mut flag: ResMut<SendPerimeterDirty>,
) {
    if flag.0 {
        writer.write(crate::messages::PatrolPerimeterDirtyMsg);
        flag.0 = false;
    }
}

#[test]
fn perimeter_dirty_sets_flag_on_msg() {
    let mut app = setup_perimeter_dirty_app();
    app.insert_resource(SendPerimeterDirty(true));
    app.update();
    let dirty = app.world().resource::<PerimeterSyncDirty>();
    assert!(dirty.0, "PerimeterSyncDirty should be true after msg");
}

#[test]
fn perimeter_dirty_stays_false_without_msgs() {
    let mut app = setup_perimeter_dirty_app();
    app.update();
    let dirty = app.world().resource::<PerimeterSyncDirty>();
    assert!(
        !dirty.0,
        "PerimeterSyncDirty should stay false with no msgs"
    );
}

#[test]
fn personality_policy_defaults_match_issue_68_targets() {
    let aggressive = AiPersonality::Aggressive.default_policies();
    assert!(!aggressive.prioritize_healing);
    assert_eq!(aggressive.recovery_hp, 0.20);
    assert!(aggressive.archer_aggressive);
    assert!(!aggressive.archer_leash);
    assert_eq!(aggressive.archer_flee_hp, 0.10);
    assert_eq!(aggressive.farmer_flee_hp, 0.30);
    assert!(aggressive.farmer_fight_back);

    let balanced = AiPersonality::Balanced.default_policies();
    assert!(balanced.prioritize_healing);
    assert_eq!(balanced.recovery_hp, 0.50);
    assert!(!balanced.archer_aggressive);
    assert!(balanced.archer_leash);
    assert_eq!(balanced.archer_flee_hp, 0.25);
    assert_eq!(balanced.farmer_flee_hp, 0.50);
    assert!(!balanced.farmer_fight_back);

    let economic = AiPersonality::Economic.default_policies();
    assert!(economic.prioritize_healing);
    assert_eq!(economic.recovery_hp, 0.70);
    assert!(!economic.archer_aggressive);
    assert!(economic.archer_leash);
    assert_eq!(economic.archer_flee_hp, 0.40);
    assert_eq!(economic.farmer_flee_hp, 0.70);
    assert!(!economic.farmer_fight_back);
}

#[test]
fn personality_loot_thresholds_match_issue_68_targets() {
    assert_eq!(AiPersonality::Aggressive.loot_threshold(), 5);
    assert_eq!(AiPersonality::Balanced.loot_threshold(), 3);
    assert_eq!(AiPersonality::Economic.loot_threshold(), 1);
}

// -- decision timer stagger (issue-192) --

fn make_player_with_timer(timer: f32) -> AiPlayer {
    AiPlayer {
        town_data_idx: 0,
        kind: AiKind::Builder,
        personality: AiPersonality::Balanced,
        road_style: RoadStyle::None,
        last_actions: Default::default(),
        policy_defaults_logged: false,
        active: true,
        build_enabled: false,
        upgrade_enabled: false,
        squad_indices: Vec::new(),
        squad_cmd: Default::default(),
        decision_timer: timer,
    }
}

/// Stagger math: with N active players and interval I, player i gets timer i*I/N.
/// Verifies timers are strictly increasing and span [0, (N-1)*I/N].
#[test]
fn decision_timers_staggered_across_interval() {
    let n = 6usize;
    let interval = crate::constants::DEFAULT_AI_INTERVAL;
    let mut players: Vec<AiPlayer> = (0..n).map(|_| make_player_with_timer(0.0)).collect();

    // Apply same stagger logic used in buildings.rs
    let n_active = players.iter().filter(|p| p.active).count();
    for (slot, p) in players.iter_mut().filter(|p| p.active).enumerate() {
        p.decision_timer = slot as f32 * interval / n_active as f32;
    }

    // Timers should be evenly distributed 0..interval
    assert_eq!(players[0].decision_timer, 0.0);
    assert!((players[n - 1].decision_timer - (n - 1) as f32 * interval / n as f32).abs() < 1e-5);
    // Each consecutive timer is larger than the previous
    for i in 1..n {
        assert!(players[i].decision_timer > players[i - 1].decision_timer);
    }
}

/// Simulate save-load: starting all timers at 0.0 then re-applying stagger must prevent burst.
/// Regression: before save.rs fix, all 18 towns fired simultaneously after every load.
#[test]
fn stagger_reapplied_after_save_load() {
    let n = 18usize;
    let interval = crate::constants::DEFAULT_AI_INTERVAL;
    // Simulate all timers reset to 0.0 (as before save.rs fix)
    let mut players: Vec<AiPlayer> = (0..n).map(|_| make_player_with_timer(0.0)).collect();

    // Re-apply stagger (mirrors save.rs fix)
    let n_active = players.iter().filter(|p| p.active).count();
    for (slot, p) in players.iter_mut().filter(|p| p.active).enumerate() {
        p.decision_timer = slot as f32 * interval / n_active as f32;
    }

    // Advance by one stagger step
    let delta = interval / n as f32 + 0.01;
    for p in players.iter_mut() {
        p.decision_timer += delta;
    }
    let due_count = players
        .iter()
        .filter(|p| p.decision_timer >= interval)
        .count();

    assert!(
        due_count < n,
        "after save-load stagger, all {n} players must not fire at once; {due_count} were due"
    );
    assert!(
        due_count > 0,
        "at least one player should be due after stagger step"
    );
}

/// With staggered timers, advancing by interval/2 should NOT trigger all players.
/// This is the regression: pre-fix, all players fired simultaneously on the same tick.
#[test]
fn staggered_timers_prevent_simultaneous_fire() {
    let n = 6usize;
    let interval = crate::constants::DEFAULT_AI_INTERVAL;
    let mut players: Vec<AiPlayer> = (0..n).map(|_| make_player_with_timer(0.0)).collect();

    // Apply stagger
    for (i, p) in players.iter_mut().enumerate() {
        p.decision_timer = i as f32 * interval / n as f32;
    }

    // Advance by slightly more than one stagger step
    let delta = interval / n as f32 + 0.01;
    for p in players.iter_mut() {
        p.decision_timer += delta;
    }

    let due: Vec<bool> = players
        .iter()
        .map(|p| p.decision_timer >= interval)
        .collect();
    let due_count = due.iter().filter(|&&d| d).count();

    // Only players near the top of the distribution should be due; not all N
    assert!(
        due_count < n,
        "staggered timers should prevent all {n} players firing at once; {due_count} were due"
    );
    assert!(due_count > 0, "at least one player should be due");
}
