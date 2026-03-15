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
