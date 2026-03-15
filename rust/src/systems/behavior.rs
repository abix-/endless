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
use crate::resources::{GameTime, GpuReadState, NpcLogCache, SelectedNpc, SquadState};
use crate::settings::UserSettings;
use crate::systemparams::EconomyState;
use bevy::prelude::*;

// ============================================================================
// SYSTEM PARAM BUNDLES - Logical groupings for scalability
// ============================================================================

use crate::messages::{CombatLogMsg, WorkIntentMsg};
use bevy::ecs::system::SystemParam;

/// NPC gameplay data queries (bundled to stay under 16 params)
#[derive(SystemParam)]
pub struct NpcDataQueries<'w, 's> {
    pub home_q: Query<'w, 's, &'static Home>,
    pub personality_q: Query<'w, 's, &'static Personality>,
    pub leash_range_q: Query<'w, 's, &'static LeashRange>,
    pub work_state_q: Query<'w, 's, &'static mut NpcWorkState>,
    pub patrol_route_q: Query<'w, 's, &'static mut PatrolRoute>,
    pub carried_loot_q: Query<'w, 's, &'static mut CarriedLoot>,
    pub stealer_q: Query<'w, 's, &'static Stealer>,
    pub has_energy_q: Query<'w, 's, &'static HasEnergy>,
}

/// NPC combat/state queries for decision_system (bundled to stay under 16 params)
#[derive(SystemParam)]
pub struct DecisionNpcState<'w, 's> {
    pub npc_flags_q: Query<'w, 's, &'static mut NpcFlags>,
    pub squad_id_q: Query<'w, 's, &'static SquadId>,
    pub manual_target_q: Query<'w, 's, &'static ManualTarget>,
    pub energy_q: Query<'w, 's, &'static mut Energy>,
    pub combat_state_q: Query<'w, 's, &'static mut CombatState>,
    pub health_q: Query<'w, 's, &'static Health, Without<Building>>,
    pub cached_stats_q: Query<'w, 's, &'static CachedStats>,
    pub activity_q: Query<'w, 's, &'static mut Activity>,
}

/// Extra resources for decision_system (bundled to stay under 16 params)
#[derive(SystemParam)]
pub struct DecisionExtras<'w> {
    pub npc_logs: ResMut<'w, NpcLogCache>,
    pub combat_log: MessageWriter<'w, CombatLogMsg>,
    pub gpu_updates: MessageWriter<'w, GpuUpdateMsg>,
    pub work_intents: MessageWriter<'w, WorkIntentMsg>,
    pub damage: MessageWriter<'w, crate::messages::DamageMsg>,
    pub squad_state: Res<'w, SquadState>,
    pub selected_npc: Res<'w, SelectedNpc>,
    pub settings: Res<'w, UserSettings>,
}

/// Incrementally maintain `ReturningSet` from `Changed<Activity>`.
/// O(changed) per frame instead of O(all_npcs).
pub fn sync_returning_set(
    mut returning: ResMut<crate::resources::ReturningSet>,
    changed_q: Query<(Entity, &Activity), Changed<Activity>>,
) {
    for (entity, activity) in &changed_q {
        if activity.kind == ActivityKind::ReturnLoot {
            if !returning.0.contains(&entity) {
                returning.0.push(entity);
            }
        } else {
            returning.0.retain(|&e| e != entity);
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
    returning: Res<crate::resources::ReturningSet>,
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
    for &entity in &returning.0 {
        let Ok((_, slot, _job, town_id, activity, home, _work_state)) = npc_q.get(entity) else {
            continue;
        };
        if activity.kind != ActivityKind::ReturnLoot {
            continue;
        }
        let idx = slot.0;
        if idx * 2 + 1 >= positions.len() {
            continue;
        }
        let x = positions[idx * 2];
        let y = positions[idx * 2 + 1];
        let dx = x - home.0.x;
        let dy = y - home.0.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist <= DELIVERY_RADIUS && town_id.0 >= 0 {
            deliveries.push((idx, entity, town_id.0 as usize));
        }
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
