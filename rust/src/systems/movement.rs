//! Movement systems - Target tracking, arrival detection, intent resolution

use bevy::prelude::*;

use crate::components::*;
use crate::constants::ARRIVAL_THRESHOLD;
use crate::gpu::EntityGpuState;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{GameTime, GpuReadState, MovementIntents, NpcTargetThrashDebug};

/// Read positions from GPU readback buffer → ECS Position + arrival detection.
/// GPU is movement authority; ECS Position is read-model synced here.
/// Query-first: iterates ECS archetypes, not HashMap.
pub fn gpu_position_readback(
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<EntityGpuState>,
    mut npc_q: Query<(&GpuSlot, &mut Position, &Activity, &mut NpcFlags)>,
) {
    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    let threshold_sq = ARRIVAL_THRESHOLD * ARRIVAL_THRESHOLD;

    for (es, mut pos, activity, mut flags) in npc_q.iter_mut() {
        let i = es.0;
        if i * 2 + 1 >= positions.len() {
            continue;
        }

        let gpu_x = positions[i * 2];
        let gpu_y = positions[i * 2 + 1];

        if gpu_x < -9000.0 {
            continue;
        }

        pos.x = gpu_x;
        pos.y = gpu_y;

        // CPU-side arrival detection
        if activity.is_transit() && !flags.at_destination {
            if i * 2 + 1 < targets.len() {
                let goal_x = targets[i * 2];
                let goal_y = targets[i * 2 + 1];
                let dx = gpu_x - goal_x;
                let dy = gpu_y - goal_y;
                if dx * dx + dy * dy <= threshold_sq {
                    flags.at_destination = true;
                }
            }
        }
    }
}

/// Resolve movement intents: pick the highest-priority intent per NPC,
/// emit exactly one SetTarget when the target actually changed.
/// Runs after all intent-producing systems (decision, combat, health, render).
pub fn resolve_movement_system(
    mut intents: ResMut<MovementIntents>,
    npc_query: Query<&GpuSlot>,
    npc_gpu: Res<EntityGpuState>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut target_thrash: ResMut<NpcTargetThrashDebug>,
    game_time: Res<GameTime>,
) {
    if game_time.is_paused() {
        return;
    }
    let targets = &npc_gpu.targets;
    let minute_key = game_time.day() * 24 * 60 + game_time.hour() * 60 + game_time.minute();

    for (entity, intent) in intents.drain() {
        let Ok(npc_idx) = npc_query.get(entity) else {
            continue;
        };
        let idx = npc_idx.0;

        // Skip if target unchanged (same check as combat's target_changed)
        let i = idx * 2;
        if i + 1 < targets.len() {
            let dx = targets[i] - intent.target.x;
            let dy = targets[i + 1] - intent.target.y;
            if dx * dx + dy * dy <= 1.0 {
                continue;
            }
        }

        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
            idx,
            x: intent.target.x,
            y: intent.target.y,
        }));
        target_thrash.record(
            idx,
            intent.source,
            minute_key,
            intent.target.x,
            intent.target.y,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use crate::resources::MovementPriority;

    // ── resolve_movement_system ────────────────────────────────────────

    #[derive(Resource, Default)]
    struct CollectedGpuUpdates(Vec<GpuUpdate>);

    fn collect_gpu_updates(
        mut reader: MessageReader<GpuUpdateMsg>,
        mut collected: ResMut<CollectedGpuUpdates>,
    ) {
        for msg in reader.read() {
            collected.0.push(msg.0.clone());
        }
    }

    fn setup_movement_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(MovementIntents::default());
        app.insert_resource(EntityGpuState::default());
        app.insert_resource(NpcTargetThrashDebug::default());
        app.insert_resource(CollectedGpuUpdates::default());
        app.add_message::<GpuUpdateMsg>();
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(
            FixedUpdate,
            (resolve_movement_system, collect_gpu_updates).chain(),
        );
        app.update();
        app.update();
        app
    }

    #[test]
    fn resolve_movement_emits_set_target() {
        let mut app = setup_movement_app();
        let entity = app.world_mut().spawn(GpuSlot(0)).id();
        // Pre-fill targets so the system can compare (current target = 0,0)
        app.world_mut().resource_mut::<EntityGpuState>().targets = vec![0.0, 0.0];
        // Submit an intent to a different position
        app.world_mut()
            .resource_mut::<MovementIntents>()
            .submit(entity, Vec2::new(100.0, 200.0), MovementPriority::Combat, "test");
        app.update();
        let collected = app.world().resource::<CollectedGpuUpdates>();
        assert!(
            collected.0.iter().any(|u| matches!(u, GpuUpdate::SetTarget { idx: 0, x, y } if (*x - 100.0).abs() < 0.1 && (*y - 200.0).abs() < 0.1)),
            "should emit SetTarget for moved NPC, got {:?}", collected.0
        );
    }

    #[test]
    fn resolve_movement_skips_same_target() {
        let mut app = setup_movement_app();
        let entity = app.world_mut().spawn(GpuSlot(0)).id();
        // Current target IS (100, 200) — submit the same
        app.world_mut().resource_mut::<EntityGpuState>().targets = vec![100.0, 200.0];
        app.world_mut()
            .resource_mut::<MovementIntents>()
            .submit(entity, Vec2::new(100.0, 200.0), MovementPriority::Combat, "test");
        app.update();
        let collected = app.world().resource::<CollectedGpuUpdates>();
        let set_targets: Vec<_> = collected.0.iter().filter(|u| matches!(u, GpuUpdate::SetTarget { .. })).collect();
        assert!(set_targets.is_empty(), "should skip SetTarget when target unchanged");
    }

    #[test]
    fn resolve_movement_paused_no_resolve() {
        let mut app = setup_movement_app();
        let entity = app.world_mut().spawn(GpuSlot(0)).id();
        app.world_mut().resource_mut::<EntityGpuState>().targets = vec![0.0, 0.0];
        app.world_mut()
            .resource_mut::<MovementIntents>()
            .submit(entity, Vec2::new(100.0, 200.0), MovementPriority::Combat, "test");
        app.world_mut().resource_mut::<GameTime>().paused = true;
        app.update();
        let collected = app.world().resource::<CollectedGpuUpdates>();
        assert!(collected.0.is_empty(), "should not resolve when paused");
    }

    // ── gpu_position_readback ──────────────────────────────────────────

    fn setup_readback_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GpuReadState::default());
        app.insert_resource(EntityGpuState::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, gpu_position_readback);
        app.update();
        app.update();
        app
    }

    #[test]
    fn readback_syncs_position_from_gpu() {
        let mut app = setup_readback_app();
        app.world_mut().resource_mut::<GpuReadState>().positions = vec![42.0, 84.0];
        app.world_mut().resource_mut::<EntityGpuState>().targets = vec![42.0, 84.0];
        app.world_mut().spawn((
            GpuSlot(0),
            Position { x: 0.0, y: 0.0 },
            Activity::Idle,
            NpcFlags::default(),
        ));
        app.update();
        let pos = app.world_mut().query::<&Position>().single(app.world()).unwrap();
        assert!((pos.x - 42.0).abs() < 0.1, "x should sync from GPU, got {}", pos.x);
        assert!((pos.y - 84.0).abs() < 0.1, "y should sync from GPU, got {}", pos.y);
    }

    #[test]
    fn readback_skips_hidden_entities() {
        let mut app = setup_readback_app();
        // GPU position -9999 means hidden
        app.world_mut().resource_mut::<GpuReadState>().positions = vec![-9999.0, -9999.0];
        app.world_mut().spawn((
            GpuSlot(0),
            Position { x: 5.0, y: 5.0 },
            Activity::Idle,
            NpcFlags::default(),
        ));
        app.update();
        let pos = app.world_mut().query::<&Position>().single(app.world()).unwrap();
        assert!((pos.x - 5.0).abs() < 0.1, "hidden entity position should not change");
    }

    #[test]
    fn readback_sets_arrival_flag() {
        let mut app = setup_readback_app();
        // NPC at exactly the target position
        app.world_mut().resource_mut::<GpuReadState>().positions = vec![100.0, 200.0];
        app.world_mut().resource_mut::<EntityGpuState>().targets = vec![100.0, 200.0];
        app.world_mut().spawn((
            GpuSlot(0),
            Position { x: 0.0, y: 0.0 },
            Activity::GoingToWork,
            NpcFlags::default(),
        ));
        app.update();
        let flags = app.world_mut().query::<&NpcFlags>().single(app.world()).unwrap();
        assert!(flags.at_destination, "should set at_destination when near target");
    }
}
