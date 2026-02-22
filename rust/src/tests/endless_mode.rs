//! Endless Mode Test (16 phases)
//! Validates full pipeline for both builder and raider town destruction:
//! world gen → fountain destroyed → pending spawn queued → respawn delay →
//! boat spawns → sails to land → NPCs disembark → walk → settle → buildings placed.
//! Phases 1-8: Builder AI town. Phases 9-16: Raider AI town (1 game-hour gap).

use bevy::prelude::*;
use crate::components::{Faction, Migrating};
use crate::messages::{BuildingDamageMsg, SpawnNpcMsg};
use crate::resources::*;
use crate::systems::{AiPlayerState, AiKind};
use crate::world::{self, BuildingKind, WorldGenStyle};

use super::{TestState, BuildingInitParams};

pub fn setup(
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut config: ResMut<world::WorldGenConfig>,
    mut food_storage: ResMut<FoodStorage>,
    mut gold_storage: ResMut<GoldStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut farm_states: ResMut<GrowthStates>,
    mut town_grids: ResMut<world::TownGrids>,
    mut bld: BuildingInitParams,
    mut spawn_writer: MessageWriter<SpawnNpcMsg>,
    mut endless: ResMut<EndlessMode>,
    mut ai_state: ResMut<AiPlayerState>,
    mut raider_state: ResMut<RaiderState>,
    mut test_state: ResMut<TestState>,
    mut game_time: ResMut<GameTime>,
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

    let (npc_msgs, ai_players) = world::setup_world(
        &config,
        &mut world_grid, &mut world_data,
        &mut farm_states, &mut town_grids,
        &mut bld.spawner_state, &mut bld.building_hp,
        &mut bld.slot_alloc, &mut bld.building_slots,
        &mut food_storage, &mut gold_storage,
        &mut faction_stats, &mut raider_state,
    );
    for msg in npc_msgs { spawn_writer.write(msg); }
    ai_state.players = ai_players;

    let total_towns = world_data.towns.len();

    endless.enabled = true;
    endless.strength_fraction = 0.75;
    endless.pending_spawns.clear();

    game_time.time_scale = 1.0;

    test_state.counters.insert("initial_towns".into(), total_towns as u32);
    test_state.counters.insert("initial_fountain_hp".into(), bld.building_hp.towns.len() as u32);

    test_state.phase_name = "Checking AI towns...".into();
    info!("endless-mode: setup — {} towns, 6000x6000 world, endless enabled", total_towns);
}

pub fn tick(
    world_data: Res<world::WorldData>,
    world_grid: Res<world::WorldGrid>,
    building_hp: Res<BuildingHpState>,
    endless: Res<EndlessMode>,
    ai_state: Res<AiPlayerState>,
    migration_state: Res<MigrationState>,
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut test: ResMut<TestState>,
    mut damage_writer: MessageWriter<BuildingDamageMsg>,
    migrating_query: Query<&Faction, With<Migrating>>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    match test.phase {
        // ====================================================================
        // BUILDER AI TOWN DESTRUCTION (phases 1-8)
        // ====================================================================

        // Phase 1: World has AI towns with Fountains
        1 => {
            let ai_towns: Vec<(usize, &world::Town)> = world_data.towns.iter().enumerate()
                .filter(|(_, t)| t.faction > 0)
                .collect();
            let ai_count = ai_towns.len();
            let fountain_count = building_hp.towns.len();

            test.phase_name = format!("ai_towns={} fountains={}", ai_count, fountain_count);

            if ai_count > 0 && fountain_count > 1 {
                // Find first builder AI town
                let builder_idx = ai_state.players.iter()
                    .find(|p| p.active && matches!(p.kind, AiKind::Builder))
                    .map(|p| p.town_data_idx);
                if let Some(idx) = builder_idx {
                    test.counters.insert("target_town".into(), idx as u32);
                    test.pass_phase(elapsed, format!("{} AI towns, {} fountains, builder target={}", ai_count, fountain_count, idx));
                } else {
                    // Fallback: first non-player town
                    let first_ai_idx = ai_towns[0].0;
                    test.counters.insert("target_town".into(), first_ai_idx as u32);
                    test.pass_phase(elapsed, format!("{} AI towns, {} fountains (no builder found, using {})", ai_count, fountain_count, first_ai_idx));
                }
            } else if elapsed > 3.0 {
                test.fail_phase(elapsed, format!("ai_towns={} fountains={}", ai_count, fountain_count));
            }
        }

        // Phase 2: Destroy builder AI Fountain
        2 => {
            let target = test.count("target_town") as usize;
            let hp = building_hp.towns.get(target).copied().unwrap_or(0.0);
            test.phase_name = format!("fountain[{}] hp={:.0}", target, hp);

            if !test.get_flag("damage_sent") {
                let max_hp = BuildingHpState::max_hp(BuildingKind::Fountain);
                damage_writer.write(BuildingDamageMsg {
                    kind: BuildingKind::Fountain, index: target,
                    amount: max_hp + 100.0, attacker_faction: 0, attacker: -1,
                });
                test.set_flag("damage_sent", true);
            } else if hp <= 0.0 {
                let ai_active = ai_state.players.iter()
                    .find(|p| p.town_data_idx == target).map(|p| p.active).unwrap_or(true);
                if !ai_active {
                    test.pass_phase(elapsed, format!("fountain[{}] destroyed, AI deactivated", target));
                } else if elapsed > 5.0 {
                    test.fail_phase(elapsed, "AI player not deactivated after fountain death");
                }
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("fountain[{}] hp={:.0} (still alive)", target, hp));
            }
        }

        // Phase 3: Pending spawn queued
        3 => {
            let pending = endless.pending_spawns.len();
            test.phase_name = format!("pending_spawns={}", pending);
            if pending > 0 {
                let spawn = &endless.pending_spawns[0];
                test.pass_phase(elapsed, format!(
                    "queued: is_raider={}, delay={:.1}h, strength={:.0}%",
                    spawn.is_raider, spawn.delay_remaining, endless.strength_fraction * 100.0
                ));
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
                    test.fail_phase(elapsed, format!("delay={:.2}h (still positive after 45s)", delay));
                }
            }
        }

        // Phase 5: Boat spawned → migration active
        5 => {
            let initial = test.count("initial_towns") as usize;
            let current = world_data.towns.len();
            let initial_fountains = test.count("initial_fountain_hp") as usize;

            if let Some(mg) = &migration_state.active {
                test.pass_phase(elapsed, format!(
                    "migration active: on_boat={}, members={}, is_raider={}",
                    mg.boat_slot.is_some(), mg.member_slots.len(), mg.is_raider
                ));
            } else if current > initial && building_hp.towns.len() > initial_fountains {
                test.pass_phase(elapsed, format!("migration already settled (towns {}->{})", initial, current));
                test.set_flag("already_settled", true);
            } else if elapsed > 60.0 {
                test.fail_phase(elapsed, format!("no migration active, towns={}, pending={}", current, endless.pending_spawns.len()));
            } else {
                test.phase_name = format!("waiting for migration... towns={}/{}", current, initial);
            }
        }

        // Phase 6: NPCs disembarked with Migrating component
        6 => {
            if test.get_flag("already_settled") {
                test.pass_phase(elapsed, "skipped (already settled)");
            } else {
                let migrating_npcs: Vec<&Faction> = migrating_query.iter().collect();
                let count = migrating_npcs.len();
                let non_player = migrating_npcs.iter().filter(|f| f.0 > 0).count();

                if let Some(mg) = &migration_state.active {
                    if mg.boat_slot.is_some() {
                        test.phase_name = format!("boat sailing... pos=({:.0},{:.0})", mg.boat_pos.x, mg.boat_pos.y);
                        if elapsed > 60.0 { test.fail_phase(elapsed, "boat never reached land"); }
                        return;
                    }
                }

                test.phase_name = format!("migrating={} non_player={}", count, non_player);
                if count > 0 && non_player == count {
                    test.pass_phase(elapsed, format!("{} migrating NPCs, all non-player faction", count));
                } else if count > 0 && non_player < count {
                    test.fail_phase(elapsed, format!("{} migrating NPCs but {} are player faction", count, count - non_player));
                } else if elapsed > 60.0 {
                    test.fail_phase(elapsed, "no migrating NPCs found");
                }
            }
        }

        // Phase 7: Migration settles
        7 => {
            if test.get_flag("already_settled") {
                test.pass_phase(elapsed, "already settled in phase 5");
            } else if migration_state.active.is_none() {
                let initial = test.count("initial_towns") as usize;
                let current = world_data.towns.len();
                if current > initial {
                    test.counters.insert("migration_town_idx".into(), (current - 1) as u32);
                }
                test.pass_phase(elapsed, format!("migration settled ({:.1}s)", elapsed));
            } else {
                let mg = migration_state.active.as_ref().unwrap();
                test.phase_name = format!("waiting for settle... boat={} members={}", mg.boat_slot.is_some(), mg.member_slots.len());
                if elapsed > 120.0 { test.fail_phase(elapsed, "migration did not settle within 120s"); }
            }
        }

        // Phase 8: New town has buildings + AI player active
        8 => {
            let initial_fountains = test.count("initial_fountain_hp") as usize;
            let current_fountains = building_hp.towns.len();
            let new_town_idx = test.count("migration_town_idx") as usize;

            let has_new_fountain = current_fountains > initial_fountains;
            let new_town_buildings: usize = world_grid.cells.iter()
                .filter(|c| c.building.map(|(_, ti)| ti as usize == new_town_idx).unwrap_or(false))
                .count();
            let ai_active = ai_state.players.iter()
                .find(|p| p.town_data_idx == new_town_idx).map(|p| p.active).unwrap_or(false);

            test.phase_name = format!("fountains={}/{} buildings={} ai_active={}", current_fountains, initial_fountains, new_town_buildings, ai_active);

            if has_new_fountain && new_town_buildings > 0 && ai_active {
                // Record game hour + snapshot for raider phase
                test.counters.insert("phase8_hour".into(), game_time.total_hours() as u32);
                test.counters.insert("towns_after_builder".into(), world_data.towns.len() as u32);
                test.counters.insert("fountains_after_builder".into(), building_hp.towns.len() as u32);
                test.pass_phase(elapsed, format!("settled: {} new buildings, AI active", new_town_buildings));
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("new_fountain={} buildings={} ai_active={}", has_new_fountain, new_town_buildings, ai_active));
            }
        }

        // ====================================================================
        // RAIDER AI TOWN DESTRUCTION (phases 9-16)
        // ====================================================================

        // Phase 9: Wait 1 game hour, then find raider target
        9 => {
            let phase8_hour = test.count("phase8_hour") as i32;
            let current_hour = game_time.total_hours();
            test.phase_name = format!("waiting 1h... hour={}/{}", current_hour, phase8_hour + 1);

            if current_hour >= phase8_hour + 1 {
                // Find a raider AI town that's still active
                let raider = ai_state.players.iter()
                    .find(|p| p.active && matches!(p.kind, AiKind::Raider));
                if let Some(r) = raider {
                    test.counters.insert("raider_target_town".into(), r.town_data_idx as u32);
                    test.pass_phase(elapsed, format!("1h elapsed, raider target={}", r.town_data_idx));
                } else {
                    test.fail_phase(elapsed, "no active raider AI towns found");
                }
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("game hour stuck at {} (need {})", current_hour, phase8_hour + 1));
            }
        }

        // Phase 10: Destroy raider fountain
        10 => {
            let target = test.count("raider_target_town") as usize;
            let hp = building_hp.towns.get(target).copied().unwrap_or(0.0);
            test.phase_name = format!("raider fountain[{}] hp={:.0}", target, hp);

            if !test.get_flag("raider_damage_sent") {
                let max_hp = BuildingHpState::max_hp(BuildingKind::Fountain);
                damage_writer.write(BuildingDamageMsg {
                    kind: BuildingKind::Fountain, index: target,
                    amount: max_hp + 100.0, attacker_faction: 0, attacker: -1,
                });
                test.set_flag("raider_damage_sent", true);
            } else if hp <= 0.0 {
                let ai_active = ai_state.players.iter()
                    .find(|p| p.town_data_idx == target).map(|p| p.active).unwrap_or(true);
                if !ai_active {
                    test.pass_phase(elapsed, format!("raider fountain[{}] destroyed, AI deactivated", target));
                } else if elapsed > 5.0 {
                    test.fail_phase(elapsed, "raider AI not deactivated after fountain death");
                }
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("raider fountain[{}] hp={:.0} (still alive)", target, hp));
            }
        }

        // Phase 11: Raider pending spawn queued
        11 => {
            let pending = endless.pending_spawns.len();
            test.phase_name = format!("raider pending_spawns={}", pending);
            if pending > 0 {
                let spawn = &endless.pending_spawns[0];
                test.pass_phase(elapsed, format!(
                    "raider queued: is_raider={}, delay={:.1}h",
                    spawn.is_raider, spawn.delay_remaining
                ));
            } else if elapsed > 5.0 {
                test.fail_phase(elapsed, "no pending spawns after raider fountain destruction");
            }
        }

        // Phase 12: Raider respawn delay ticks down
        12 => {
            if endless.pending_spawns.is_empty() {
                test.pass_phase(elapsed, "raider spawn already consumed");
            } else {
                let delay = endless.pending_spawns[0].delay_remaining;
                test.phase_name = format!("raider delay={:.2}h", delay);
                if delay <= 0.0 {
                    test.pass_phase(elapsed, format!("raider delay reached 0 ({:.2}h)", delay));
                } else if elapsed > 45.0 {
                    test.fail_phase(elapsed, format!("raider delay={:.2}h (still positive after 45s)", delay));
                }
            }
        }

        // Phase 13: Raider boat/migration active
        13 => {
            let initial = test.count("towns_after_builder") as usize;
            let current = world_data.towns.len();
            let initial_fountains = test.count("fountains_after_builder") as usize;

            if let Some(mg) = &migration_state.active {
                test.pass_phase(elapsed, format!(
                    "raider migration active: on_boat={}, members={}, is_raider={}",
                    mg.boat_slot.is_some(), mg.member_slots.len(), mg.is_raider
                ));
            } else if current > initial && building_hp.towns.len() > initial_fountains {
                test.pass_phase(elapsed, format!("raider migration already settled (towns {}->{})", initial, current));
                test.set_flag("raider_already_settled", true);
            } else if elapsed > 60.0 {
                test.fail_phase(elapsed, format!("no raider migration, towns={}, pending={}", current, endless.pending_spawns.len()));
            } else {
                test.phase_name = format!("waiting for raider migration... towns={}/{}", current, initial);
            }
        }

        // Phase 14: Raider NPCs disembarked with Migrating
        14 => {
            if test.get_flag("raider_already_settled") {
                test.pass_phase(elapsed, "skipped (raider already settled)");
            } else {
                let migrating_npcs: Vec<&Faction> = migrating_query.iter().collect();
                let count = migrating_npcs.len();
                let non_player = migrating_npcs.iter().filter(|f| f.0 > 0).count();

                if let Some(mg) = &migration_state.active {
                    if mg.boat_slot.is_some() {
                        test.phase_name = format!("raider boat sailing... pos=({:.0},{:.0})", mg.boat_pos.x, mg.boat_pos.y);
                        if elapsed > 60.0 { test.fail_phase(elapsed, "raider boat never reached land"); }
                        return;
                    }
                }

                test.phase_name = format!("raider migrating={} non_player={}", count, non_player);
                if count > 0 && non_player == count {
                    test.pass_phase(elapsed, format!("{} raider migrating NPCs", count));
                } else if count > 0 && non_player < count {
                    test.fail_phase(elapsed, format!("{} migrating but {} are player faction", count, count - non_player));
                } else if elapsed > 60.0 {
                    test.fail_phase(elapsed, "no raider migrating NPCs found");
                }
            }
        }

        // Phase 15: Raider migration settles
        15 => {
            if test.get_flag("raider_already_settled") {
                test.pass_phase(elapsed, "raider already settled in phase 13");
            } else if migration_state.active.is_none() {
                let initial = test.count("towns_after_builder") as usize;
                let current = world_data.towns.len();
                if current > initial {
                    test.counters.insert("raider_migration_town_idx".into(), (current - 1) as u32);
                }
                test.pass_phase(elapsed, format!("raider migration settled ({:.1}s)", elapsed));
            } else {
                let mg = migration_state.active.as_ref().unwrap();
                test.phase_name = format!("waiting for raider settle... boat={} members={}", mg.boat_slot.is_some(), mg.member_slots.len());
                if elapsed > 120.0 { test.fail_phase(elapsed, "raider migration did not settle within 120s"); }
            }
        }

        // Phase 16: Raider new town has buildings + AI active → COMPLETE
        16 => {
            let initial_fountains = test.count("fountains_after_builder") as usize;
            let current_fountains = building_hp.towns.len();
            let new_town_idx = test.count("raider_migration_town_idx") as usize;

            let has_new_fountain = current_fountains > initial_fountains;
            let new_town_buildings: usize = world_grid.cells.iter()
                .filter(|c| c.building.map(|(_, ti)| ti as usize == new_town_idx).unwrap_or(false))
                .count();
            let ai_active = ai_state.players.iter()
                .find(|p| p.town_data_idx == new_town_idx).map(|p| p.active).unwrap_or(false);

            test.phase_name = format!("raider fountains={}/{} buildings={} ai_active={}", current_fountains, initial_fountains, new_town_buildings, ai_active);

            if has_new_fountain && new_town_buildings > 0 && ai_active {
                test.pass_phase(elapsed, format!("raider settled: {} new buildings, AI active", new_town_buildings));
                test.complete(elapsed);
            } else if elapsed > 30.0 {
                test.fail_phase(elapsed, format!("raider new_fountain={} buildings={} ai_active={}", has_new_fountain, new_town_buildings, ai_active));
            }
        }

        _ => {}
    }
}
