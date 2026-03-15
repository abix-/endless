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
