//! Endless Mode Test (14 phases)
//! Validates full pipeline for both builder and raider town destruction:
//! world gen → fountain destroyed → pending spawn queued → respawn delay →
//! boat spawns → sails to land → settle → buildings placed.
//! Phases 1-7: Builder AI town. Phases 8-14: Raider AI town (1 game-hour gap).

use crate::messages::DamageMsg;
use crate::resources::*;
use crate::systems::{AiKind, AiPlayerState};
use crate::world::{self, BuildingKind, WorldGenStyle};
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use super::{BuildingInitParams, TestState};

#[derive(SystemParam)]
pub(super) struct EndlessModeSetupState<'w> {
    endless: ResMut<'w, EndlessMode>,
    ai_state: ResMut<'w, AiPlayerState>,
    raider_state: ResMut<'w, RaiderState>,
    test_state: ResMut<'w, TestState>,
    game_time: ResMut<'w, GameTime>,
}

pub(super) fn setup(
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut config: ResMut<world::WorldGenConfig>,
    mut faction_stats: ResMut<FactionStats>,

    mut slot_alloc: ResMut<GpuSlotPool>,
    mut bld: BuildingInitParams,
    mut state: EndlessModeSetupState,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut uid_alloc: ResMut<crate::resources::NextEntityUid>,
    mut commands: Commands,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    mut town_index: ResMut<crate::resources::TownIndex>,
) {
    config.gen_style = WorldGenStyle::Continents;
    config.num_towns = 1;
    config.ai_towns = 2;
    config.raider_towns = 2;
    config.world_width = 6000.0;
    config.world_height = 6000.0;
    config.world_margin = 400.0;
    config.raider_distance = 2000.0;
    config.min_town_distance = 1000.0;

    let ai_players = world::setup_world(
        &config,
        &mut world_grid,
        &mut world_data,
        &mut crate::resources::FactionList::default(),
        &mut slot_alloc,
        &mut bld.entity_map,
        &mut faction_stats,
        &mut crate::resources::Reputation::default(),
        &mut state.raider_state,
        &mut uid_alloc,
        &mut town_index,
        &mut commands,
        &mut gpu_updates,
    );
    state.ai_state.players = ai_players;

    let total_towns = world_data.towns.len();

    state.endless.enabled = true;
    state.endless.strength_fraction = 0.75;
    state.endless.pending_spawns.clear();

    state.game_time.time_scale = 1.0;

    state
        .test_state
        .counters
        .insert("initial_towns".into(), total_towns as u32);
    state
        .test_state
        .counters
        .insert("initial_fountain_hp".into(), total_towns as u32);

    if let Some(town) = world_data.towns.first() {
        if let Ok(mut cam) = camera_query.single_mut() {
            cam.translation.x = town.center.x;
            cam.translation.y = town.center.y;
        }
    }
    state.test_state.phase_name = "Checking AI towns...".into();
    info!(
        "endless-mode: setup — {} towns, 6000x6000 world, endless enabled",
        total_towns
    );
}

pub fn tick(
    world_data: Res<world::WorldData>,
    _world_grid: Res<world::WorldGrid>,
    building_query: Query<
        (
            &crate::components::Building,
            &crate::components::Health,
            &crate::components::TownId,
            &crate::components::EntityUid,
        ),
        Without<crate::components::Dead>,
    >,
    endless: Res<EndlessMode>,
    ai_state: Res<AiPlayerState>,
    migration_state: Res<MigrationState>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut test: ResMut<TestState>,
    mut damage_writer: MessageWriter<DamageMsg>,
    entity_map: Res<EntityMap>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    match test.phase {
        // ====================================================================
        // BUILDER AI TOWN DESTRUCTION (phases 1-7)
        // ====================================================================

        // Phase 1: World has AI towns with Fountains
        1 => {
            let ai_towns: Vec<(usize, &world::Town)> = world_data
                .towns
                .iter()
                .enumerate()
                .filter(|(_, t)| t.faction > crate::constants::FACTION_PLAYER)
                .collect();
            let ai_count = ai_towns.len();
            let fountain_count = world_data.towns.len();

            test.phase_name = format!("ai_towns={} fountains={}", ai_count, fountain_count);

            if ai_count > 0 && fountain_count > 1 {
                // Find first builder AI town
                let builder_idx = ai_state
                    .players
                    .iter()
                    .find(|p| p.active && matches!(p.kind, AiKind::Builder))
                    .map(|p| p.town_data_idx);
                if let Some(idx) = builder_idx {
                    test.counters.insert("target_town".into(), idx as u32);
                    test.pass_phase(
                        elapsed,
                        format!(
                            "{} AI towns, {} fountains, builder target={}",
                            ai_count, fountain_count, idx
                        ),
                    );
                } else {
                    // Fallback: first non-player town
                    let first_ai_idx = ai_towns[0].0;
                    test.counters
                        .insert("target_town".into(), first_ai_idx as u32);
                    test.pass_phase(
                        elapsed,
                        format!(
                            "{} AI towns, {} fountains (no builder found, using {})",
                            ai_count, fountain_count, first_ai_idx
                        ),
                    );
                }
            } else if elapsed > 3.0 {
                test.fail_phase(
                    elapsed,
                    format!("ai_towns={} fountains={}", ai_count, fountain_count),
                );
            }
        }

        // Phase 2: Destroy builder AI Fountain
        2 => {
            let target = test.count("target_town") as usize;
            let fountain = building_query
                .iter()
                .find(|(b, _, t, _)| b.kind == BuildingKind::Fountain && t.0 as usize == target);

            if !test.get_flag("damage_sent") {
                let max_hp = crate::constants::building_def(BuildingKind::Fountain).hp;
                if let Some((_, _, _, uid)) = fountain {
                    damage_writer.write(DamageMsg {
                        target: *uid,
                        amount: max_hp + 100.0,
                        attacker_faction: 2,
                        attacker: -1,
                    });
                }
                test.set_flag("damage_sent", true);
            }

            let hp = fountain.map(|(_, h, _, _)| h.0).unwrap_or(-1.0);
            test.phase_name = format!("fountain[{}] hp={}", target, hp);

            if fountain.is_none() {
                test.pass_phase(elapsed, format!("fountain[{}] destroyed", target));
            } else if hp <= 0.0 {
                test.pass_phase(elapsed, format!("fountain[{}] hp={}", target, hp));
            } else if elapsed > 5.0 {
                test.fail_phase(
                    elapsed,
                    format!("fountain[{}] hp={} (still alive)", target, hp),
                );
            }
        }

        // Phase 3: Pending spawn queued
        3 => {
            let pending = endless.pending_spawns.len();
            test.phase_name = format!("pending_spawns={}", pending);
            if pending > 0 {
                let spawn = &endless.pending_spawns[0];
                test.pass_phase(
                    elapsed,
                    format!(
                        "queued: is_raider={}, delay={:.1}h, strength={:.0}%",
                        spawn.is_raider,
                        spawn.delay_remaining,
                        endless.strength_fraction * 100.0
                    ),
                );
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, "no pending spawns after fountain destruction");
            }
        }

        // Phase 4: Respawn delay ticks down
        4 => {
            if endless.pending_spawns.is_empty() {
                test.pass_phase(elapsed, "spawn already consumed (delay elapsed)");
            } else {
                let delay = endless.pending_spawns[0].delay_remaining;
                test.phase_name = format!("delay={:.2}h", delay);
                if delay <= 0.0 {
                    test.pass_phase(elapsed, format!("delay reached 0 ({:.2}h)", delay));
                } else if elapsed > 45.0 {
                    test.fail_phase(
                        elapsed,
                        format!("delay={:.2}h (still positive after 45s)", delay),
                    );
                }
            }
        }

        // Phase 5: Boat spawned → migration active
        5 => {
            let initial = test.count("initial_towns") as usize;
            let current = world_data.towns.len();
            let initial_fountains = test.count("initial_fountain_hp") as usize;

            if let Some(mg) = &migration_state.active {
                test.pass_phase(
                    elapsed,
                    format!(
                        "migration active: on_boat={}, members={}, is_raider={}",
                        mg.boat_slot.is_some(),
                        mg.member_slots.len(),
                        mg.is_raider
                    ),
                );
            } else if current > initial && world_data.towns.len() > initial_fountains {
                test.pass_phase(
                    elapsed,
                    format!("migration already settled (towns {}->{})", initial, current),
                );
                test.set_flag("already_settled", true);
            } else if elapsed > 60.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "no migration active, towns={}, pending={}",
                        current,
                        endless.pending_spawns.len()
                    ),
                );
            } else {
                test.phase_name = format!("waiting for migration... towns={}/{}", current, initial);
            }
        }

        // Phase 6: Migration settles
        6 => {
            if test.get_flag("already_settled") {
                test.pass_phase(elapsed, "already settled in phase 5");
            } else if migration_state.active.is_none() {
                let initial = test.count("initial_towns") as usize;
                let current = world_data.towns.len();
                if current > initial {
                    test.counters
                        .insert("migration_town_idx".into(), (current - 1) as u32);
                }
                test.pass_phase(elapsed, format!("migration settled ({:.1}s)", elapsed));
            } else {
                let mg = migration_state.active.as_ref().unwrap();
                test.phase_name = format!(
                    "waiting for settle... boat={} members={}",
                    mg.boat_slot.is_some(),
                    mg.member_slots.len()
                );
                if elapsed > 120.0 {
                    test.fail_phase(elapsed, "migration did not settle within 120s");
                }
            }
        }

        // Phase 7: New town has buildings + AI player active
        7 => {
            let initial_fountains = test.count("initial_fountain_hp") as usize;
            let current_fountains = world_data.towns.len();
            let new_town_idx = test.count("migration_town_idx") as usize;

            let has_new_fountain = current_fountains > initial_fountains;
            let new_town_buildings: usize = entity_map
                .iter_instances()
                .filter(|inst| inst.town_idx as usize == new_town_idx)
                .count();
            let ai_active = ai_state
                .players
                .iter()
                .find(|p| p.town_data_idx == new_town_idx)
                .map(|p| p.active)
                .unwrap_or(false);

            test.phase_name = format!(
                "fountains={}/{} buildings={} ai_active={}",
                current_fountains, initial_fountains, new_town_buildings, ai_active
            );

            if has_new_fountain && new_town_buildings > 0 && ai_active {
                // Record game hour + snapshot for raider phase
                test.counters
                    .insert("phase7_hour".into(), game_time.total_hours() as u32);
                test.counters
                    .insert("towns_after_builder".into(), world_data.towns.len() as u32);
                test.counters.insert(
                    "fountains_after_builder".into(),
                    world_data.towns.len() as u32,
                );
                test.pass_phase(
                    elapsed,
                    format!("settled: {} new buildings, AI active", new_town_buildings),
                );
            } else if elapsed > 30.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "new_fountain={} buildings={} ai_active={}",
                        has_new_fountain, new_town_buildings, ai_active
                    ),
                );
            }
        }

        // ====================================================================
        // RAIDER AI TOWN DESTRUCTION (phases 8-14)
        // ====================================================================

        // Phase 8: Wait 1 game hour, then find raider target
        8 => {
            let phase7_hour = test.count("phase7_hour") as i32;
            let current_hour = game_time.total_hours();
            test.phase_name = format!("waiting 1h... hour={}/{}", current_hour, phase7_hour + 1);

            if current_hour > phase7_hour {
                // Find a raider AI town that's still active
                let raider = ai_state
                    .players
                    .iter()
                    .find(|p| p.active && matches!(p.kind, AiKind::Raider));
                if let Some(r) = raider {
                    test.counters
                        .insert("raider_target_town".into(), r.town_data_idx as u32);
                    test.pass_phase(
                        elapsed,
                        format!("1h elapsed, raider target={}", r.town_data_idx),
                    );
                } else {
                    test.fail_phase(elapsed, "no active raider AI towns found");
                }
            } else if elapsed > 75.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "game hour stuck at {} (need {})",
                        current_hour,
                        phase7_hour + 1
                    ),
                );
            }
        }

        // Phase 9: Destroy raider fountain
        9 => {
            let target = test.count("raider_target_town") as usize;
            let fountain = building_query
                .iter()
                .find(|(b, _, t, _)| b.kind == BuildingKind::Fountain && t.0 as usize == target);

            if !test.get_flag("raider_damage_sent") {
                let max_hp = crate::constants::building_def(BuildingKind::Fountain).hp;
                if let Some((_, _, _, uid)) = fountain {
                    damage_writer.write(DamageMsg {
                        target: *uid,
                        amount: max_hp + 100.0,
                        attacker_faction: 2,
                        attacker: -1,
                    });
                }
                test.set_flag("raider_damage_sent", true);
            }

            let hp = fountain.map(|(_, h, _, _)| h.0).unwrap_or(-1.0);
            test.phase_name = format!("raider fountain[{}] hp={}", target, hp);

            if fountain.is_none() {
                test.pass_phase(
                    elapsed,
                    format!("raider fountain[{}] destroyed", target),
                );
            } else if hp <= 0.0 {
                test.pass_phase(
                    elapsed,
                    format!("raider fountain[{}] hp={}", target, hp),
                );
            } else if elapsed > 80.0 {
                test.fail_phase(
                    elapsed,
                    format!("raider fountain[{}] hp={} (still alive)", target, hp),
                );
            }
        }

        // Phase 10: Raider pending spawn queued
        10 => {
            let pending = endless.pending_spawns.len();
            test.phase_name = format!("raider pending_spawns={}", pending);
            if pending > 0 {
                let spawn = &endless.pending_spawns[0];
                test.pass_phase(
                    elapsed,
                    format!(
                        "raider queued: is_raider={}, delay={:.1}h",
                        spawn.is_raider, spawn.delay_remaining
                    ),
                );
            } else if elapsed > 85.0 {
                test.fail_phase(
                    elapsed,
                    "no pending spawns after raider fountain destruction",
                );
            }
        }

        // Phase 11: Raider respawn delay ticks down
        11 => {
            if endless.pending_spawns.is_empty() {
                test.pass_phase(elapsed, "raider spawn already consumed");
            } else {
                let delay = endless.pending_spawns[0].delay_remaining;
                test.phase_name = format!("raider delay={:.2}h", delay);
                if delay <= 0.0 {
                    test.pass_phase(elapsed, format!("raider delay reached 0 ({:.2}h)", delay));
                } else if elapsed > 140.0 {
                    test.fail_phase(
                        elapsed,
                        format!("raider delay={:.2}h (still positive after 140s)", delay),
                    );
                }
            }
        }

        // Phase 12: Raider boat/migration active
        12 => {
            let initial = test.count("towns_after_builder") as usize;
            let current = world_data.towns.len();
            let initial_fountains = test.count("fountains_after_builder") as usize;

            if let Some(mg) = &migration_state.active {
                test.pass_phase(
                    elapsed,
                    format!(
                        "raider migration active: on_boat={}, members={}, is_raider={}",
                        mg.boat_slot.is_some(),
                        mg.member_slots.len(),
                        mg.is_raider
                    ),
                );
            } else if current > initial && world_data.towns.len() > initial_fountains {
                test.pass_phase(
                    elapsed,
                    format!(
                        "raider migration already settled (towns {}->{})",
                        initial, current
                    ),
                );
                test.set_flag("raider_already_settled", true);
            } else if elapsed > 200.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "no raider migration, towns={}, pending={}",
                        current,
                        endless.pending_spawns.len()
                    ),
                );
            } else {
                test.phase_name = format!(
                    "waiting for raider migration... towns={}/{}",
                    current, initial
                );
            }
        }

        // Phase 13: Raider migration settles
        13 => {
            if test.get_flag("raider_already_settled") {
                test.pass_phase(elapsed, "raider already settled in phase 12");
            } else if migration_state.active.is_none() {
                let initial = test.count("towns_after_builder") as usize;
                let current = world_data.towns.len();
                if current > initial {
                    test.counters
                        .insert("raider_migration_town_idx".into(), (current - 1) as u32);
                }
                test.pass_phase(
                    elapsed,
                    format!("raider migration settled ({:.1}s)", elapsed),
                );
            } else {
                let mg = migration_state.active.as_ref().unwrap();
                test.phase_name = format!(
                    "waiting for raider settle... boat={} members={}",
                    mg.boat_slot.is_some(),
                    mg.member_slots.len()
                );
                if elapsed > 300.0 {
                    test.fail_phase(elapsed, "raider migration did not settle within 300s");
                }
            }
        }

        // Phase 14: Raider new town has buildings + AI active → COMPLETE
        14 => {
            let initial_fountains = test.count("fountains_after_builder") as usize;
            let current_fountains = world_data.towns.len();
            let new_town_idx = test.count("raider_migration_town_idx") as usize;

            let has_new_fountain = current_fountains > initial_fountains;
            let new_town_buildings: usize = entity_map
                .iter_instances()
                .filter(|inst| inst.town_idx as usize == new_town_idx)
                .count();
            let ai_active = ai_state
                .players
                .iter()
                .find(|p| p.town_data_idx == new_town_idx)
                .map(|p| p.active)
                .unwrap_or(false);

            test.phase_name = format!(
                "raider fountains={}/{} buildings={} ai_active={}",
                current_fountains, initial_fountains, new_town_buildings, ai_active
            );

            if has_new_fountain && new_town_buildings > 0 && ai_active {
                test.pass_phase(
                    elapsed,
                    format!(
                        "raider settled: {} new buildings, AI active",
                        new_town_buildings
                    ),
                );
                test.complete(elapsed);
            } else if elapsed > 330.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "raider new_fountain={} buildings={} ai_active={}",
                        has_new_fountain, new_town_buildings, ai_active
                    ),
                );
            }
        }

        _ => {}
    }
}
