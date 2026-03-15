//! Patrol systems - OnDuty tick counters and patrol route rebuilding

use crate::components::*;
use crate::resources::GameTime;
use bevy::prelude::*;

/// Increment OnDuty tick counters (runs every frame for guards at posts).
/// Separated from decision_system because we need mutable Activity access.
pub fn on_duty_tick_system(
    game_time: Res<GameTime>,
    mut q: Query<
        (&mut Activity, &CombatState),
        (With<PatrolRoute>, Without<Building>, Without<Dead>),
    >,
) {
    if game_time.is_paused() {
        return;
    }
    for (mut activity, combat_state) in q.iter_mut() {
        if combat_state.is_fighting() {
            continue;
        }
        if activity.kind == ActivityKind::Patrol && activity.phase == ActivityPhase::Holding {
            activity.ticks_waiting += 1;
        }
    }
}

/// Rebuild all guards' patrol routes when WorldData changes (waypoint added/removed/reordered).
pub fn rebuild_patrol_routes_system(
    entity_map: Res<crate::entity_map::EntityMap>,
    mut patrols_dirty: MessageReader<crate::messages::PatrolsDirtyMsg>,
    mut patrol_swaps: MessageReader<crate::messages::PatrolSwapMsg>,
    mut patrol_route_q: Query<&mut PatrolRoute>,
    mut commands: Commands,
    patrol_npc_q: Query<(Entity, &GpuSlot, &Job, &TownId), (Without<Building>, Without<Dead>)>,
    mut waypoint_q: Query<&mut WaypointOrder, With<Building>>,
) {
    if patrols_dirty.read().count() == 0 {
        return;
    }

    // Apply pending patrol order swap from UI
    if let Some(swap) = patrol_swaps.read().last() {
        let (sa, sb) = (swap.slot_a, swap.slot_b);
        let order_a = entity_map
            .entities
            .get(&sa)
            .and_then(|&e| waypoint_q.get(e).ok())
            .map(|w| w.0)
            .unwrap_or(0);
        let order_b = entity_map
            .entities
            .get(&sb)
            .and_then(|&e| waypoint_q.get(e).ok())
            .map(|w| w.0)
            .unwrap_or(0);
        if let Some(&entity) = entity_map.entities.get(&sa) {
            if let Ok(mut w) = waypoint_q.get_mut(entity) {
                w.0 = order_b;
            }
        }
        if let Some(&entity) = entity_map.entities.get(&sb) {
            if let Ok(mut w) = waypoint_q.get_mut(entity) {
                w.0 = order_a;
            }
        }
    }

    // Collect patrol unit slots + towns via ECS query
    let patrol_slots: Vec<(Entity, usize, i32)> = patrol_npc_q
        .iter()
        .filter(|(_, _, job, _)| job.is_patrol_unit())
        .map(|(entity, slot, _, town)| (entity, slot.0, town.0))
        .collect();

    // Build routes once per town (immutable entity_map access for building queries)
    let mut town_routes: std::collections::HashMap<u32, Vec<Vec2>> =
        std::collections::HashMap::new();
    for &(_, _, town_idx) in &patrol_slots {
        let tid = town_idx as u32;
        town_routes.entry(tid).or_insert_with(|| {
            crate::systems::spawn::build_patrol_route_ecs(&entity_map, tid, &waypoint_q)
        });
    }

    // Write routes back via ECS
    for (entity, _slot, town_idx) in patrol_slots {
        let tid = town_idx as u32;
        let Some(new_posts) = town_routes.get(&tid) else {
            continue;
        };
        if new_posts.is_empty() {
            continue;
        }
        if let Ok(mut route) = patrol_route_q.get_mut(entity) {
            if route.current >= new_posts.len() {
                route.current = 0;
            }
            route.posts = new_posts.clone();
        } else {
            commands.entity(entity).insert(PatrolRoute {
                posts: new_posts.clone(),
                current: 0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::GameTime;
    use bevy::time::TimeUpdateStrategy;

    fn setup_on_duty_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, on_duty_tick_system);
        app.update();
        app.update();
        app
    }

    #[test]
    fn on_duty_increments_ticks_waiting() {
        let mut app = setup_on_duty_app();
        let npc = app
            .world_mut()
            .spawn((
                Activity {
                    kind: ActivityKind::Patrol,
                    phase: ActivityPhase::Holding,
                    target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                    ..Default::default()
                },
                CombatState::None,
                PatrolRoute {
                    posts: vec![],
                    current: 0,
                },
            ))
            .id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if activity.kind == ActivityKind::Patrol {
            assert!(
                activity.ticks_waiting > 0,
                "ticks_waiting should increment: {}",
                activity.ticks_waiting
            );
        } else {
            panic!("activity should still be OnDuty");
        }
    }

    #[test]
    fn on_duty_fighting_skipped() {
        let mut app = setup_on_duty_app();
        let npc = app
            .world_mut()
            .spawn((
                Activity {
                    kind: ActivityKind::Patrol,
                    phase: ActivityPhase::Holding,
                    target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                    ..Default::default()
                },
                CombatState::Fighting { origin: Vec2::ZERO },
                PatrolRoute {
                    posts: vec![],
                    current: 0,
                },
            ))
            .id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if activity.kind == ActivityKind::Patrol {
            assert_eq!(
                activity.ticks_waiting, 0,
                "fighting NPCs should not increment ticks"
            );
        } else {
            panic!("activity should still be OnDuty");
        }
    }

    #[test]
    fn on_duty_paused_no_change() {
        let mut app = setup_on_duty_app();
        app.world_mut().resource_mut::<GameTime>().paused = true;
        let npc = app
            .world_mut()
            .spawn((
                Activity {
                    kind: ActivityKind::Patrol,
                    phase: ActivityPhase::Holding,
                    target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                    ticks_waiting: 5,
                    ..Default::default()
                },
                CombatState::None,
                PatrolRoute {
                    posts: vec![],
                    current: 0,
                },
            ))
            .id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if activity.kind == ActivityKind::Patrol {
            assert_eq!(
                activity.ticks_waiting, 5,
                "paused should not increment: {}",
                activity.ticks_waiting
            );
        }
    }

    #[test]
    fn on_duty_dead_excluded() {
        let mut app = setup_on_duty_app();
        let npc = app
            .world_mut()
            .spawn((
                Activity {
                    kind: ActivityKind::Patrol,
                    phase: ActivityPhase::Holding,
                    target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                    ..Default::default()
                },
                CombatState::None,
                Dead,
            ))
            .id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        if activity.kind == ActivityKind::Patrol {
            assert_eq!(activity.ticks_waiting, 0, "dead NPC should not increment");
        }
    }

    #[test]
    fn on_duty_buildings_excluded() {
        let mut app = setup_on_duty_app();
        let bld = app
            .world_mut()
            .spawn((
                Activity {
                    kind: ActivityKind::Patrol,
                    phase: ActivityPhase::Holding,
                    target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                    ..Default::default()
                },
                CombatState::None,
                Building {
                    kind: crate::world::BuildingKind::Tower,
                },
            ))
            .id();

        app.update();
        let activity = app.world().get::<Activity>(bld).unwrap();
        if activity.kind == ActivityKind::Patrol {
            assert_eq!(activity.ticks_waiting, 0, "buildings should not increment");
        }
    }

    #[test]
    fn non_on_duty_activity_unchanged() {
        let mut app = setup_on_duty_app();
        let npc = app
            .world_mut()
            .spawn((Activity::new(ActivityKind::Work), CombatState::None))
            .id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        assert!(
            activity.kind == ActivityKind::Work,
            "non-OnDuty activity should not change"
        );
    }

    #[test]
    fn on_duty_tick_only_during_holding() {
        let mut app = setup_on_duty_app();
        // Transit phase should NOT increment ticks_waiting
        let npc = app
            .world_mut()
            .spawn((
                Activity {
                    kind: ActivityKind::Patrol,
                    phase: ActivityPhase::Transit,
                    target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                    ..Default::default()
                },
                CombatState::None,
                PatrolRoute {
                    posts: vec![],
                    current: 0,
                },
            ))
            .id();

        app.update();
        let activity = app.world().get::<Activity>(npc).unwrap();
        assert_eq!(
            activity.ticks_waiting, 0,
            "transit phase should not increment ticks"
        );
    }
}
