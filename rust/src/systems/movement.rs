//! Movement systems - Target tracking, arrival detection, path routing
//!
//! Single owner of SetTarget GPU messages. Drains PathRequestQueue pending intents,
//! routes via LOS bypass or A*, respects time/count budget.

use std::time::Instant;

use bevy::prelude::*;

use crate::components::*;
use crate::constants::{ARRIVAL_THRESHOLD, INTERMEDIATE_ARRIVAL_THRESHOLD, PATH_SPREAD_COST, PATH_SPREAD_RADIUS};
use crate::gpu::EntityGpuState;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{
    GameTime, GpuReadState, NpcTargetThrashDebug, PathRequest,
    PathRequestQueue, PathSource, PathfindConfig, PathfindStats,
};
use crate::systems::pathfinding::{accumulate_path_cost, line_of_sight, pathfind_with_costs};
use crate::world::WorldGrid;

/// Read positions from GPU readback buffer → ECS Position + arrival detection.
/// GPU is movement authority; ECS Position is read-model synced here.
/// Query-first: iterates ECS archetypes, not HashMap.
pub fn gpu_position_readback(
    gpu_state: Res<GpuReadState>,
    buffer_writes: Res<EntityGpuState>,
    mut npc_q: Query<(&GpuSlot, &mut Position, &Activity, &mut NpcFlags, &NpcPath)>,
) {
    let positions = &gpu_state.positions;
    let targets = &buffer_writes.targets;
    let threshold_sq = ARRIVAL_THRESHOLD * ARRIVAL_THRESHOLD;
    let intermediate_sq = INTERMEDIATE_ARRIVAL_THRESHOLD * INTERMEDIATE_ARRIVAL_THRESHOLD;

    for (es, mut pos, activity, mut flags, path) in npc_q.iter_mut() {
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
                let dist_sq = dx * dx + dy * dy;
                // Relaxed threshold for intermediate waypoints — prevents pile-up
                // when boid separation pushes NPCs away from shared A* waypoints
                let is_intermediate = path.current + 1 < path.waypoints.len();
                let thresh_sq = if is_intermediate { intermediate_sq } else { threshold_sq };
                if dist_sq <= thresh_sq {
                    flags.at_destination = true;
                }
            }
        }
    }
}

/// Advance NPC path waypoints when at_destination triggers and more waypoints remain.
/// Clears at_destination and sets new goal for the next waypoint.
/// Runs after gpu_position_readback so at_destination is fresh.
pub fn advance_waypoints_system(
    grid: Res<crate::world::WorldGrid>,
    game_time: Res<GameTime>,
    time: Res<Time>,
    mut npc_q: Query<
        (Entity, &GpuSlot, &mut NpcPath, &mut NpcFlags, &Activity),
        (Without<Building>, Without<Dead>),
    >,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
) {
    if game_time.is_paused() {
        return;
    }
    let dt = time.delta_secs();
    for (_entity, slot, mut path, mut flags, activity) in npc_q.iter_mut() {
        // Tick down path cooldown (from A* failure backoff)
        if path.path_cooldown > 0.0 {
            path.path_cooldown = (path.path_cooldown - dt).max(0.0);
        }
        if !flags.at_destination || !activity.is_transit() {
            continue;
        }
        if path.waypoints.is_empty() || path.current >= path.waypoints.len() {
            continue;
        }

        // Check if there are more waypoints
        if path.current + 1 < path.waypoints.len() {
            path.current += 1;
            let next = path.waypoints[path.current];
            let world_pos = grid.grid_to_world(next.x as usize, next.y as usize);

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx: slot.0,
                x: world_pos.x,
                y: world_pos.y,
            }));

            // Clear at_destination so GPU resumes movement toward new waypoint
            flags.at_destination = false;
        }
        // else: at final waypoint — leave at_destination=true for decision system to handle
    }
}

/// Unified movement resolution + path routing.
/// 1. Drain pending world-space intents → filter → enqueue as grid-space PathRequests
/// 2. Drain PathRequestQueue (budget-limited) → route via LOS bypass or A*
///    Runs after all intent-producing systems and after invalidate_paths_on_building_change.
pub fn resolve_movement_system(
    npc_query: Query<&GpuSlot>,
    npc_gpu: Res<EntityGpuState>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut target_thrash: ResMut<NpcTargetThrashDebug>,
    game_time: Res<GameTime>,
    grid: Res<WorldGrid>,
    mut path_queue: ResMut<PathRequestQueue>,
    config: Res<PathfindConfig>,
    gpu_state: Res<GpuReadState>,
    mut path_q: Query<&mut NpcPath>,
    mut stats: ResMut<PathfindStats>,
) {
    if game_time.is_paused() {
        return;
    }
    let targets = &npc_gpu.targets;
    let positions = &gpu_state.positions;
    let minute_key = game_time.day() * 24 * 60 + game_time.hour() * 60 + game_time.minute();
    let has_grid = grid.width > 0 && grid.height > 0;

    // ── Phase 1: Drain world-space intents → enqueue as PathRequests ──
    let intents: Vec<_> = path_queue.drain_intents().collect();
    for (entity, intent) in intents {
        let Ok(npc_idx) = npc_query.get(entity) else {
            continue;
        };
        let idx = npc_idx.0;

        let i = idx * 2;

        // "Stop in place" — intent target ≈ current position: skip cooldown, write directly
        if i + 1 < positions.len() {
            let dx = positions[i] - intent.target.x;
            let dy = positions[i + 1] - intent.target.y;
            if dx * dx + dy * dy <= 4.0 {
                // Only write if GPU target actually differs (avoid no-op writes)
                if i + 1 < targets.len() {
                    let tdx = targets[i] - intent.target.x;
                    let tdy = targets[i + 1] - intent.target.y;
                    if tdx * tdx + tdy * tdy > 4.0 {
                        if let Ok(mut npc_path) = path_q.get_mut(entity) {
                            npc_path.waypoints.clear();
                            npc_path.current = 0;
                        }
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                            idx,
                            x: intent.target.x,
                            y: intent.target.y,
                        }));
                        target_thrash.record(idx, intent.source, minute_key, intent.target.x, intent.target.y);
                    }
                }
                continue;
            }
        }

        // Skip if already pathing to the same goal (prevents re-path thrash)
        if let Ok(npc_path) = path_q.get(entity) {
            if !npc_path.waypoints.is_empty() {
                let dg = npc_path.goal_world - intent.target;
                if dg.length_squared() <= 1.0 {
                    continue;
                }
            }
            if npc_path.path_cooldown > 0.0 {
                continue;
            }
        }

        // Skip if target unchanged vs GPU waypoint
        if i + 1 < targets.len() {
            let dx = targets[i] - intent.target.x;
            let dy = targets[i + 1] - intent.target.y;
            if dx * dx + dy * dy <= 1.0 {
                continue;
            }
        }

        // No grid or no position data — direct SetTarget fallback
        if !has_grid || i + 1 >= positions.len() {
            if let Ok(mut npc_path) = path_q.get_mut(entity) {
                npc_path.waypoints.clear();
                npc_path.current = 0;
            }
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx,
                x: intent.target.x,
                y: intent.target.y,
            }));
            target_thrash.record(idx, intent.source, minute_key, intent.target.x, intent.target.y);
            continue;
        }

        let npc_pos = Vec2::new(positions[i], positions[i + 1]);
        let (sc, sr) = grid.world_to_grid(npc_pos);
        let (gc, gr) = grid.world_to_grid(intent.target);

        path_queue.enqueue(PathRequest {
            entity,
            slot: idx,
            start: IVec2::new(sc as i32, sr as i32),
            goal: IVec2::new(gc as i32, gr as i32),
            goal_world: intent.target,
            priority: 1,
            source: PathSource::Movement,
        });

        target_thrash.record(idx, intent.source, minute_key, intent.target.x, intent.target.y);
    }

    // ── Phase 2: Drain queue → route (LOS bypass or A*) ──────────────
    if path_queue.is_empty() || grid.pathfind_costs.is_empty() {
        return;
    }

    let batch = path_queue.drain_budget(config.max_per_frame);
    let start_time = Instant::now();
    let time_budget = std::time::Duration::from_secs_f32(config.max_time_budget_ms / 1000.0);
    let mut processed = 0usize;
    let mut los_bypass = 0usize;
    let mut astar_calls = 0usize;
    let mut astar_fails = 0usize;
    let mut budget_reason: &'static str = "count";
    let mut consumed = 0usize;

    // Path cost accumulation: clone the cost grid so each successive A* call
    // sees inflated costs along previously-found paths, spreading routes apart.
    let mut accum_costs = grid.pathfind_costs.clone();

    for (i, req) in batch.iter().enumerate() {
        consumed = i + 1;
        if processed > 0 && start_time.elapsed() >= time_budget {
            budget_reason = "time";
            consumed = i;
            break;
        }

        if npc_query.get(req.entity).is_err() {
            continue;
        }

        processed += 1;

        let dist = (req.goal - req.start).abs();
        let manhattan = dist.x + dist.y;

        // Short-distance LOS bypass — direct SetTarget
        if manhattan <= config.short_distance_tiles
            && line_of_sight(&grid, req.start, req.goal)
        {
            los_bypass += 1;
            if let Ok(mut path) = path_q.get_mut(req.entity) {
                path.waypoints.clear();
                path.current = 0;
                path.path_cooldown = 0.0;
            }
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx: req.slot,
                x: req.goal_world.x,
                y: req.goal_world.y,
            }));
            continue;
        }

        // Full A* pathfinding (using accumulated costs for route spreading)
        astar_calls += 1;
        if let Some(path_points) = pathfind_with_costs(
            &accum_costs, grid.width, grid.height,
            req.start, req.goal, config.max_nodes,
        ) {
            if path_points.len() < 2 {
                continue;
            }

            // Inflate costs along this path so subsequent paths spread to different routes
            accumulate_path_cost(
                &mut accum_costs, grid.width, grid.height,
                &path_points, PATH_SPREAD_RADIUS, PATH_SPREAD_COST,
            );

            let first_wp = path_points[1];
            let world_pos = grid.grid_to_world(first_wp.x as usize, first_wp.y as usize);

            if let Ok(mut npc_path) = path_q.get_mut(req.entity) {
                npc_path.waypoints = path_points;
                npc_path.current = 1;
                npc_path.goal_world = req.goal_world;
                npc_path.path_cooldown = 0.0;
            }

            gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                idx: req.slot,
                x: world_pos.x,
                y: world_pos.y,
            }));
        } else {
            astar_fails += 1;
            if let Ok(mut npc_path) = path_q.get_mut(req.entity) {
                npc_path.path_cooldown = 2.0;
            }
        }
    }

    // Re-queue unprocessed batch items (time budget exceeded)
    for req in batch.into_iter().skip(consumed) {
        path_queue.enqueue(req);
    }

    let elapsed_ms = start_time.elapsed().as_secs_f32() * 1000.0;
    let remaining = path_queue.total_len();
    stats.update(processed, los_bypass, astar_calls, astar_fails, elapsed_ms, remaining, budget_reason);
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use crate::resources::{EntityMap, MovementPriority};

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
        app.insert_resource(EntityGpuState::default());
        app.insert_resource(NpcTargetThrashDebug::default());
        app.insert_resource(CollectedGpuUpdates::default());
        app.insert_resource(GpuReadState::default());
        app.insert_resource(WorldGrid::default());
        app.insert_resource(EntityMap::default());
        app.insert_resource(PathRequestQueue::default());
        app.insert_resource(PathfindConfig::default());
        app.insert_resource(PathfindStats::default());
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
            .resource_mut::<PathRequestQueue>()
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
            .resource_mut::<PathRequestQueue>()
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
            .resource_mut::<PathRequestQueue>()
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
            NpcPath::default(),
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
            NpcPath::default(),
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
            NpcPath::default(),
        ));
        app.update();
        let flags = app.world_mut().query::<&NpcFlags>().single(app.world()).unwrap();
        assert!(flags.at_destination, "should set at_destination when near target");
    }
}
