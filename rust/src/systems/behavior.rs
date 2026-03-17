//! Behavior systems - Unified decision-making and state transitions
//!
//! Key systems:
//! - `arrival_system`: Minimal - marks NPCs as AtDestination, handles proximity delivery
//! - `on_duty_tick_system`: Increments guard wait counters (in patrol.rs)
//! - `decision_system`: Central priority-based decision making for ALL NPCs (in decision.rs)
//!
//! Phase model (Slice 1: Rest, Heal):
//! - ActivityPhase::Transit = walking toward target
//! - ActivityPhase::Active = performing sustained work/recovery at target
//! - Rest: Transit(Home) -> Active(Home) -> Idle+Ready
//! - Heal: Transit(Fountain) -> Active(Fountain) -> Idle+Ready

use crate::components::*;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::{GameTime, GpuReadState, NpcLogCache};
use crate::systemparams::EconomyState;
use bevy::prelude::*;

/// Incrementally maintain `ReturningSet` from `Changed<Activity>`.
/// O(changed) per frame instead of O(all_npcs).
pub fn sync_returning_set(
    mut returning: ResMut<crate::resources::ReturningSet>,
    changed_q: Query<(Entity, &Activity), Changed<Activity>>,
) {
    for (entity, activity) in &changed_q {
        if activity.kind == ActivityKind::ReturnLoot {
            returning.0.insert(entity);
        } else {
            returning.0.remove(&entity);
        }
    }
}

/// Arrival system: proximity-based delivery for Returning NPCs.
///
/// When a Returning NPC is within delivery radius of home, deposit CarriedLoot and go Idle.
/// Arrival detection (transit -> AtDestination) is handled by gpu_position_readback.
/// Farm occupancy and harvest are handled exclusively by decision_system.
pub fn arrival_system(
    mut economy: EconomyState,
    gpu_state: Res<GpuReadState>,
    game_time: Res<GameTime>,
    mut npc_logs: ResMut<NpcLogCache>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut carried_loot_q: Query<&mut CarriedLoot>,
    mut npc_q: Query<
        (
            Entity,
            &GpuSlot,
            &Job,
            &TownId,
            &mut Activity,
            &Home,
            &mut NpcWorkState,
        ),
        (Without<Building>, Without<Dead>),
    >,
    mut returning: ResMut<crate::resources::ReturningSet>,
    _production_q: Query<&mut ProductionState>,
    _miner_cfg_q: Query<&MinerHomeConfig>,
) {
    if game_time.is_paused() {
        return;
    }
    let positions = &gpu_state.positions;
    const DELIVERY_RADIUS: f32 = 50.0;

    // ========================================================================
    // 1. Proximity-based delivery for Returning NPCs (from ReturningSet)
    // ========================================================================
    let mut deliveries: Vec<(usize, Entity, usize)> = Vec::new();
    let tracked_entities: Vec<Entity> = returning.0.drain().collect();
    for entity in tracked_entities {
        let Ok((_, slot, _job, town_id, activity, home, _work_state)) = npc_q.get(entity) else {
            continue;
        };
        if activity.kind != ActivityKind::ReturnLoot {
            continue;
        }
        let idx = slot.0;
        if idx * 2 + 1 >= positions.len() {
            returning.0.insert(entity);
            continue;
        }
        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= DELIVERY_RADIUS && town_id.0 >= 0 {
            deliveries.push((idx, entity, town_id.0 as usize));
            continue;
        }
        returning.0.insert(entity);
    }

    for (idx, entity, town_idx) in deliveries {
        // Read and drain CarriedLoot
        if let Ok(mut loot) = carried_loot_q.get_mut(entity) {
            if loot.food > 0 {
                if let Some(mut f) = economy.towns.food_mut(town_idx as i32) {
                    f.0 += loot.food;
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} food", loot.food),
                );
            }
            if loot.gold > 0 {
                if let Some(mut g) = economy.towns.gold_mut(town_idx as i32) {
                    g.0 += loot.gold;
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} gold", loot.gold),
                );
            }
            if loot.wood > 0 {
                if let Some(mut w) = economy.towns.wood_mut(town_idx as i32) {
                    w.0 += loot.wood;
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} wood", loot.wood),
                );
            }
            if loot.stone > 0 {
                if let Some(mut s) = economy.towns.stone_mut(town_idx as i32) {
                    s.0 += loot.stone;
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} stone", loot.stone),
                );
            }
            if !loot.equipment.is_empty() {
                let count = loot.equipment.len();
                if let Some(mut eq) = economy.towns.equipment_mut(town_idx as i32) {
                    eq.0.append(&mut loot.equipment);
                } else {
                    loot.equipment.clear();
                }
                npc_logs.push(
                    idx,
                    game_time.day(),
                    game_time.hour(),
                    game_time.minute(),
                    format!("Delivered {} equipment", count),
                );
            }
            loot.food = 0;
            loot.gold = 0;
            loot.wood = 0;
            loot.stone = 0;
        }
        if let Ok((_, slot, _, _, mut act, _, mut ws)) = npc_q.get_mut(entity) {
            *act = Activity::default();
            // Clear stale work_target so idle farmers don't carry a phantom target.
            // worksite is NOT cleared here -- decision_system owns occupancy
            // release via entity_map and handles it before setting Returning.
            ws.worksite = None;
            gpu_updates.write(GpuUpdateMsg(GpuUpdate::MarkVisualDirty { idx: slot.0 }));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::ecs::system::RunSystemOnce;

    fn setup_arrival_app() -> (App, Entity) {
        let mut app = App::new();
        app.add_message::<GpuUpdateMsg>()
            .init_resource::<crate::resources::PopulationStats>()
            .init_resource::<crate::resources::TownIndex>()
            .init_resource::<crate::resources::GameTime>()
            .init_resource::<crate::resources::GpuReadState>()
            .init_resource::<crate::resources::NpcLogCache>()
            .init_resource::<crate::resources::ReturningSet>();

        let town_entity = app
            .world_mut()
            .spawn((
                TownMarker,
                FoodStore(0),
                GoldStore(0),
                WoodStore(0),
                StoneStore(0),
                TownPolicy::default(),
                TownUpgradeLevel::default(),
                TownEquipment::default(),
            ))
            .id();
        app.world_mut()
            .resource_mut::<crate::resources::TownIndex>()
            .0
            .insert(0, town_entity);

        (app, town_entity)
    }

    #[test]
    fn arrival_system_delivers_loot_and_clears_returning_entry_same_frame() {
        let (mut app, town_entity) = setup_arrival_app();
        let npc = app
            .world_mut()
            .spawn((
                GpuSlot(0),
                Job::Farmer,
                TownId(0),
                Activity {
                    kind: ActivityKind::ReturnLoot,
                    phase: ActivityPhase::Transit,
                    target: ActivityTarget::Dropoff,
                    ..Default::default()
                },
                Home(Vec2::ZERO),
                NpcWorkState {
                    worksite: Some(town_entity),
                },
                CarriedLoot {
                    food: 3,
                    ..Default::default()
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<crate::resources::GpuReadState>()
            .positions = vec![0.0, 0.0];
        app.world_mut()
            .resource_mut::<crate::resources::ReturningSet>()
            .0
            .insert(npc);

        let _ = app.world_mut().run_system_once(arrival_system);

        assert_eq!(
            app.world().get::<FoodStore>(town_entity).map(|f| f.0),
            Some(3),
            "arrival should deliver carried food to the owning town"
        );
        assert!(
            app.world()
                .get::<CarriedLoot>(npc)
                .is_some_and(CarriedLoot::is_empty),
            "arrival should drain carried loot after delivery"
        );
        assert_eq!(
            app.world().get::<Activity>(npc).copied(),
            Some(Activity::default()),
            "arrival should transition the NPC back to idle-ready after delivery"
        );
        assert_eq!(
            app.world()
                .get::<NpcWorkState>(npc)
                .and_then(|ws| ws.worksite),
            None,
            "arrival should clear stale worksite ownership after delivery"
        );
        assert!(
            app.world()
                .resource::<crate::resources::ReturningSet>()
                .0
                .is_empty(),
            "arrival should remove delivered NPCs from ReturningSet immediately"
        );
    }

    #[test]
    fn arrival_system_prunes_despawned_returning_entities() {
        let (mut app, _town_entity) = setup_arrival_app();
        let npc = app
            .world_mut()
            .spawn((
                GpuSlot(0),
                Job::Farmer,
                TownId(0),
                Activity {
                    kind: ActivityKind::ReturnLoot,
                    phase: ActivityPhase::Transit,
                    target: ActivityTarget::Dropoff,
                    ..Default::default()
                },
                Home(Vec2::new(500.0, 500.0)),
                NpcWorkState::default(),
                CarriedLoot {
                    food: 1,
                    ..Default::default()
                },
            ))
            .id();
        app.world_mut()
            .resource_mut::<crate::resources::GpuReadState>()
            .positions = vec![0.0, 0.0];
        app.world_mut()
            .resource_mut::<crate::resources::ReturningSet>()
            .0
            .insert(npc);

        let _ = app.world_mut().run_system_once(arrival_system);
        assert_eq!(
            app.world()
                .resource::<crate::resources::ReturningSet>()
                .0
                .len(),
            1,
            "live undelivered NPCs should remain tracked"
        );

        app.world_mut().entity_mut(npc).despawn();
        let _ = app.world_mut().run_system_once(arrival_system);

        assert!(
            app.world()
                .resource::<crate::resources::ReturningSet>()
                .0
                .is_empty(),
            "despawned NPCs should be pruned from ReturningSet"
        );
    }
}
