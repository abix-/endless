//! AI decision system -- scores and executes building + upgrade actions per AI tick.

use bevy::prelude::*;

use crate::components::Job;
use crate::constants::*;
use crate::resources::*;
use crate::world::BuildingKind;

use super::build_actions::*;
use super::mine_analysis::TownContext;
use super::slot_selection::*;
use super::*;

// ============================================================================
// AI DECISION SYSTEM
// ============================================================================

/// One decision per AI per interval tick. Scores all eligible actions, picks via weighted random.
/// Per-player timers stagger towns across ticks so all 18 AI factions never run simultaneously.
pub fn ai_decision_system(
    time: Res<Time>,
    config: Res<AiPlayerConfig>,
    mut ai_state: ResMut<AiPlayerState>,
    mut res: AiBuildRes,
    mut town_access: crate::systemparams::TownAccess,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    game_time: Res<GameTime>,
    difficulty: Res<Difficulty>,
    gpu_state: Res<GpuReadState>,
    pop_stats: Res<PopulationStats>,
    mut snapshots: Local<AiTownSnapshotCache>,
    settings: Res<crate::settings::UserSettings>,
    mut snapshot_dirty: ResMut<AiSnapshotDirty>,
) {
    // Use real-time delta (not game-time-scaled) so AI decision cadence stays
    // constant regardless of game speed. At 16x, strategic building/upgrade
    // decisions do not benefit from running 16x more often.
    let delta = if game_time.is_paused() {
        0.0
    } else {
        time.delta_secs()
    };

    // Advance every player's individual timer.
    for player in ai_state.players.iter_mut() {
        player.decision_timer += delta;
    }

    // Early exit when no player is due this frame.
    let any_due = ai_state
        .players
        .iter()
        .any(|p| p.active && p.decision_timer >= config.decision_interval);
    if !any_due {
        return;
    }

    let dirty = snapshot_dirty.0;
    snapshot_dirty.0 = false;
    if dirty {
        snapshots.towns.clear();
        // Recompute spawner counts per town from EntityMap
        snapshots.spawner_counts.clear();
        for inst in res.world.entity_map.iter_instances() {
            if crate::constants::building_def(inst.kind).spawner.is_some() {
                *snapshots
                    .spawner_counts
                    .entry(inst.town_idx as usize)
                    .or_default() += 1;
            }
        }
    }

    for pi in 0..ai_state.players.len() {
        // Two-step style common in Rust ECS:
        // 1) gather immutable state and score actions
        // 2) perform one mutating action
        if !ai_state.players[pi].active
            || ai_state.players[pi].decision_timer < config.decision_interval
        {
            continue;
        }
        // Reset per-player timer. Each town fires independently at its own cadence.
        ai_state.players[pi].decision_timer = 0.0;
        let player = &ai_state.players[pi];
        let tdi = player.town_data_idx;
        let personality = player.personality;
        let road_style = player.road_style;
        let build_enabled = player.build_enabled;
        let upgrade_enabled = player.upgrade_enabled;
        let kind = player.kind;
        let _ = player; // end immutable borrow -- mutable access needed later
        if let std::collections::hash_map::Entry::Vacant(e) = snapshots.towns.entry(tdi) {
            if let Some(snap) = build_town_snapshot(
                &res.world.world_data,
                &res.world.entity_map,
                &res.world.grid,
                tdi,
                town_access.area_level(tdi as i32),
                personality,
                road_style,
            ) {
                e.insert(snap);
            }
        }

        let mut food_val = town_access.food(tdi as i32);
        let food = food_val;
        let spawner_count = snapshots.spawner_counts.get(&tdi).copied().unwrap_or(0);
        let town_policy = town_access.policy(tdi as i32);
        let policy_reserves = town_policy
            .as_ref()
            .map(|p| (p.reserve_food, p.reserve_gold))
            .unwrap_or((0, 0));
        let reserve = personality.food_reserve_per_spawner() * spawner_count + policy_reserves.0;
        // Desire signals are computed once below and reused by action + upgrade scoring.
        let mining_radius = town_policy
            .as_ref()
            .map(|p| p.mining_radius)
            .unwrap_or(crate::constants::DEFAULT_MINING_RADIUS);
        let Some(ctx) = TownContext::build(
            tdi,
            food,
            snapshots.towns.get(&tdi),
            &res,
            kind,
            mining_radius,
            town_access.area_level(tdi as i32),
        ) else {
            continue;
        };

        let town_name = res
            .world
            .world_data
            .towns
            .get(tdi)
            .map(|t| t.name.clone())
            .unwrap_or_default();
        let pname = personality.name();

        // Pre-compute mine_shafts before bc closure to allow mutable borrow for bootstrap.
        let mine_shafts = res
            .world
            .entity_map
            .count_for_town(BuildingKind::MinerHome, ctx.ti);

        // Deterministic miner bootstrap: bypasses food reserve gate.
        // Ensures min miner homes are built before the town can starve its gold economy.
        if matches!(kind, AiKind::Builder)
            && ctx.has_slots
            && mine_shafts < personality.min_miner_homes()
            && food >= building_cost(BuildingKind::MinerHome)
        {
            if let Some(mines) = ctx
                .mines
                .as_ref()
                .filter(|m| m.in_radius + m.outside_radius > 0)
            {
                if let Some(what) = try_build_miner_home(
                    &ctx,
                    mines,
                    &mut res,
                    &mut food_val,
                    snapshots.towns.get(&tdi),
                    personality,
                    road_style,
                ) {
                    snapshots.towns.remove(&tdi);
                    let faction = res
                        .world
                        .world_data
                        .towns
                        .get(tdi)
                        .map(|t| t.faction)
                        .unwrap_or(0);
                    log_ai(
                        &mut combat_log,
                        &game_time,
                        faction,
                        &town_name,
                        pname,
                        &what,
                    );
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY {
                        actions.pop_front();
                    }
                    actions.push_back((what, game_time.day(), game_time.hour()));
                    continue;
                }
            }
        }

        // Hoard food for miner home: if bootstrap didn't fire (food < 4), block all spending.
        if matches!(kind, AiKind::Builder)
            && ctx.has_slots
            && mine_shafts < personality.min_miner_homes()
            && food < building_cost(BuildingKind::MinerHome)
        {
            if ctx
                .mines
                .as_ref()
                .is_some_and(|m| m.in_radius + m.outside_radius > 0)
            {
                continue;
            }
        }

        // Food reserve rule: if town is at/below reserve, skip spending this tick.
        if food <= reserve {
            continue;
        }

        let bc = |k: BuildingKind| res.world.entity_map.count_for_town(k, ctx.ti);
        let farms = bc(BuildingKind::Farm);
        let houses = bc(BuildingKind::FarmerHome);
        let barracks = bc(BuildingKind::ArcherHome);
        let xbow_homes = bc(BuildingKind::CrossbowHome);
        let waypoints = bc(BuildingKind::Waypoint);
        let total_military_homes = barracks + xbow_homes;
        let faction = res
            .world
            .world_data
            .towns
            .get(tdi)
            .map(|t| t.faction)
            .unwrap_or(0);
        // Threat signal from GPU spatial grid: fountain's enemy count from readback.
        let threat = res
            .world
            .entity_map
            .iter_kind_for_town(BuildingKind::Fountain, tdi as u32)
            .next()
            .map(|inst| inst.slot)
            .and_then(|slot| gpu_state.threat_counts.get(slot).copied())
            .map(|packed| {
                let enemies = (packed >> 16) as f32;
                (enemies / 10.0).min(1.0)
            })
            .unwrap_or(0.0);
        // Count alive civilians vs military for this town from PopulationStats.
        let town_key = tdi as i32;
        let pop_alive = |job: Job| {
            pop_stats
                .0
                .get(&(job as i32, town_key))
                .map(|p| p.alive)
                .unwrap_or(0)
                .max(0) as usize
        };
        let civilians = pop_alive(Job::Farmer) + pop_alive(Job::Miner);
        let military = pop_alive(Job::Archer) + pop_alive(Job::Fighter) + pop_alive(Job::Crossbow);
        let mut desires = desire_state(
            personality,
            food,
            reserve,
            houses + mine_shafts,
            total_military_homes,
            waypoints,
            threat,
            civilians,
            military,
        );

        // Gold desire: driven by cheapest gold-costing upgrade the AI wants but can't afford.
        let uw = personality.upgrade_weights(kind);
        let levels = town_access.upgrade_levels(tdi as i32);
        let gold = town_access.gold(tdi as i32);
        let cheapest_gold = cheapest_gold_upgrade_cost(&uw, &levels, gold);
        desires.gold_desire = if cheapest_gold > 0 {
            ((1.0 - gold as f32 / cheapest_gold as f32) * personality.gold_desire_mult())
                .clamp(0.0, 1.0)
        } else {
            personality.base_mining_desire()
        };

        // Economy desire: how much the town needs to fill its buildable area.
        // Floors other desires so building scores never collapse to zero while slots remain.
        desires.economy_desire = 1.0 - ctx.slot_fullness;
        desires.food_desire = desires.food_desire.max(desires.economy_desire);
        desires.military_desire = desires.military_desire.max(desires.economy_desire);
        desires.gold_desire = desires.gold_desire.max(desires.economy_desire);

        // --- Policy: eat_food toggle based on food desire ---
        if let Some(mut policy) = town_access.policy_mut(tdi as i32) {
            let (off_threshold, on_threshold) = personality.eat_food_desire_thresholds();
            let should_eat = if policy.0.eat_food {
                desires.food_desire < off_threshold
            } else {
                desires.food_desire < on_threshold
            };
            if should_eat != policy.0.eat_food {
                policy.0.eat_food = should_eat;
                let state = if should_eat { "on" } else { "off" };
                log_ai(
                    &mut combat_log,
                    &game_time,
                    faction,
                    &town_name,
                    pname,
                    &format!(
                        "eat_food -> {state} (food_desire={:.2})",
                        desires.food_desire
                    ),
                );
            }
        }

        if !ai_state.players[pi].policy_defaults_logged {
            if let Some(mut policy) = town_access.policy_mut(tdi as i32) {
                policy.0.prioritize_healing = personality.default_prioritize_healing();
                policy.0.recovery_hp = personality.default_recovery_hp();
                policy.0.archer_aggressive = personality.default_archer_aggressive();
                policy.0.archer_flee_hp = personality.default_archer_flee_hp();
                policy.0.farmer_flee_hp = personality.default_farmer_flee_hp();
                policy.0.farmer_fight_back = personality.default_farmer_fight_back();

                log_ai(
                    &mut combat_log,
                    &game_time,
                    faction,
                    &town_name,
                    pname,
                    &format!(
                        "policy defaults: heal={}, recovery={:.2}, aggro={}, archer_flee={:.2}, farmer_flee={:.2}, fight_back={}",
                        policy.0.prioritize_healing,
                        policy.0.recovery_hp,
                        policy.0.archer_aggressive,
                        policy.0.archer_flee_hp,
                        policy.0.farmer_flee_hp,
                        policy.0.farmer_fight_back
                    ),
                );
                ai_state.players[pi].policy_defaults_logged = true;
            }
        }

        let debug = settings.debug_ai_decisions;

        // ================================================================
        // Phase 1: Score and execute a BUILDING action
        // ================================================================
        if build_enabled {
            let mut build_scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);

            match kind {
                AiKind::Raider => {
                    // Raider AI has a smaller economy action set.
                    if ctx.has_slots && ctx.food >= building_cost(BuildingKind::Tent) {
                        build_scores.push((AiAction::BuildTent, 30.0));
                    }
                }
                AiKind::Builder => {
                    // Builder AI scores economic + military + mining expansion actions.
                    let (fw, hw, bw, gw) = personality.building_weights();
                    let total_civilians = houses + mine_shafts;
                    let bt = personality.archer_home_target(total_civilians);
                    let ht = personality.farmer_home_target(farms);
                    let Some(mines) = ctx.mines.as_ref() else {
                        continue;
                    };
                    let ms_target = ((total_civilians as f32 * personality.mining_ratio())
                        as usize)
                        .max(mines.in_radius) // at least 1 miner per in-radius mine
                        .min(mines.in_radius * MAX_MINERS_PER_MINE);
                    let house_deficit = ht.saturating_sub(houses);
                    let barracks_deficit = bt.saturating_sub(barracks);
                    let miner_deficit = ms_target.saturating_sub(mine_shafts);

                    if ctx.has_slots {
                        // Desire-driven need model:
                        // food_desire gates farm/house construction,
                        // military_desire gates barracks/crossbow/waypoint construction.
                        // Base personality weights set ratios within each category.
                        let farm_need =
                            desires.food_desire * (houses as f32 - farms as f32).max(0.0);
                        let house_need = if house_deficit > 0 {
                            desires.food_desire * (house_deficit as f32).min(10.0)
                        } else {
                            desires.food_desire * 0.5 // baseline to match military's 0.5 floor
                        };
                        let barracks_need = if barracks_deficit > 0 {
                            desires.military_desire * barracks_deficit as f32
                        } else {
                            desires.military_desire * 0.5
                        };

                        if ctx.food >= building_cost(BuildingKind::Farm) {
                            build_scores.push((AiAction::BuildFarm, fw * farm_need));
                        }
                        if ctx.food >= building_cost(BuildingKind::FarmerHome) {
                            build_scores.push((AiAction::BuildFarmerHome, hw * house_need));
                        }
                        if ctx.food >= building_cost(BuildingKind::ArcherHome) {
                            build_scores.push((AiAction::BuildArcherHome, bw * barracks_need));
                        }
                        // Crossbow homes: AI builds them once it has some archer homes established
                        if barracks >= 2 && ctx.food >= building_cost(BuildingKind::CrossbowHome) {
                            let xbow_need = if xbow_homes < barracks / 2 {
                                desires.military_desire
                                    * barracks.saturating_sub(xbow_homes * 2) as f32
                            } else {
                                desires.military_desire * 0.5
                            };
                            build_scores.push((AiAction::BuildCrossbowHome, bw * 0.6 * xbow_need));
                        }
                        if miner_deficit > 0 && ctx.food >= building_cost(BuildingKind::MinerHome) {
                            let ms_need = desires.gold_desire * miner_deficit as f32;
                            // Bootstrap boost: guarantee min miner homes per personality
                            let bootstrap = if mine_shafts < personality.min_miner_homes() {
                                5.0
                            } else {
                                1.0
                            };
                            build_scores.push((AiAction::BuildMinerHome, hw * ms_need * bootstrap));
                        } else if miner_deficit == 0
                            && mines.outside_radius > 0
                            && mine_shafts >= mines.in_radius
                        {
                            let expand_need = desires.gold_desire * mines.outside_radius as f32;
                            build_scores.push((
                                AiAction::ExpandMiningRadius,
                                personality.expand_mining_weight() * expand_need,
                            ));
                        }
                    }

                    let perimeter_target = snapshots
                        .towns
                        .get(&tdi)
                        .map(|s| s.waypoint_ring.len())
                        .unwrap_or(total_military_homes);
                    let waypoint_target = total_military_homes.max(perimeter_target);
                    if ctx.food >= building_cost(BuildingKind::Waypoint)
                        && waypoints < waypoint_target
                    {
                        let gp_need =
                            desires.military_desire * (waypoint_target - waypoints) as f32;
                        build_scores.push((AiAction::BuildWaypoint, gw * gp_need));
                    }

                    // Roads: build roads using the town's road style
                    let rw = personality.road_weight();
                    if road_style != RoadStyle::None
                        && rw > 0.0
                        && ctx.food >= building_cost(BuildingKind::Road) * 4
                    {
                        let road_candidates = count_road_candidates(
                            &res.world.entity_map,
                            ctx.area_level,
                            ctx.center,
                            &res.world.grid,
                            ctx.ti,
                            road_style,
                        );
                        if road_candidates > 0 {
                            let roads = bc(BuildingKind::Road)
                                + bc(BuildingKind::StoneRoad)
                                + bc(BuildingKind::MetalRoad);
                            let economy_buildings = farms + houses + mine_shafts;
                            let road_need =
                                road_candidates.min(economy_buildings.saturating_sub(roads / 2));
                            if road_need > 0 {
                                build_scores.push((AiAction::BuildRoads, rw * road_need as f32));
                            }
                        }
                    }
                }
            }

            // Retry loop: if picked action fails, remove it and re-pick from remaining.
            let mut build_succeeded = false;
            loop {
                let Some(action) = weighted_pick(&build_scores) else {
                    break;
                };
                let mut new_mr = None;
                let label = execute_action(
                    action,
                    &ctx,
                    &mut res,
                    &mut food_val,
                    mining_radius,
                    &mut new_mr,
                    snapshots.towns.get(&tdi),
                    personality,
                    road_style,
                    *difficulty,
                );
                if let Some(mr) = new_mr {
                    if let Some(mut p) = town_access.policy_mut(tdi as i32) {
                        p.0.mining_radius = mr;
                    }
                }
                if let Some(what) = label {
                    snapshots.towns.remove(&tdi);
                    log_ai(
                        &mut combat_log,
                        &game_time,
                        faction,
                        &town_name,
                        pname,
                        &what,
                    );
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY {
                        actions.pop_front();
                    }
                    actions.push_back((what, game_time.day(), game_time.hour()));
                    build_succeeded = true;
                    break;
                }
                // Action failed -- log and remove this variant from candidates
                if debug {
                    let msg = format!(
                        "[dbg] {} FAILED ({})",
                        action.label(),
                        format_top_scores(&build_scores, 4)
                    );
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY {
                        actions.pop_front();
                    }
                    actions.push_back((msg, game_time.day(), game_time.hour()));
                }
                let failed = std::mem::discriminant(&action);
                build_scores.retain(|(a, _)| std::mem::discriminant(a) != failed);
            }
            if !build_succeeded && debug {
                if build_scores.is_empty() {
                    let actions = &mut ai_state.players[pi].last_actions;
                    if actions.len() >= MAX_ACTION_HISTORY {
                        actions.pop_front();
                    }
                    actions.push_back((
                        "[dbg] no build candidates".into(),
                        game_time.day(),
                        game_time.hour(),
                    ));
                }
            }
        } // build_enabled

        // ================================================================
        // Phase 2: Score and execute an UPGRADE action (if food/gold remain)
        // ================================================================
        if upgrade_enabled {
            let food_after = town_access.food(tdi as i32);
            let gold_after = town_access.gold(tdi as i32);
            // Gold reservation: policy reserve + expansion upgrade hoard.
            let expansion_gold_reserve = policy_reserves.1
                + if !ctx.has_slots {
                    uw.iter()
                        .enumerate()
                        .filter(|&(_, &w)| w > 0.0)
                        .filter(|&(idx, _)| UPGRADES.nodes[idx].triggers_expansion)
                        .filter(|&(idx, _)| upgrade_unlocked(&levels, idx))
                        .map(|(idx, _)| {
                            let lv = levels.get(idx).copied().unwrap_or(0);
                            let node = &UPGRADES.nodes[idx];
                            if node.custom_cost {
                                expansion_cost(lv).1
                            } else {
                                let scale = upgrade_cost(lv);
                                node.cost
                                    .iter()
                                    .filter(|&&(kind, _)| kind == ResourceKind::Gold)
                                    .map(|&(_, base)| base * scale)
                                    .sum::<i32>()
                            }
                        })
                        .min()
                        .unwrap_or(0)
                } else {
                    0
                };
            if food_after > reserve {
                let mut upgrade_scores: Vec<(AiAction, f32)> = Vec::with_capacity(8);
                for (idx, &weight) in uw.iter().enumerate() {
                    if weight <= 0.0 {
                        continue;
                    }
                    if !upgrade_available(&levels, idx, food_after, gold_after) {
                        continue;
                    }
                    // Fill slots first: only expansion upgrades allowed while town has empty slots
                    let is_expansion = UPGRADES.nodes[idx].triggers_expansion;
                    if ctx.has_slots && !is_expansion {
                        continue;
                    }
                    // Hoard gold for expansion: skip non-expansion gold-costing upgrades
                    // unless we have surplus gold beyond what expansion needs.
                    if !ctx.has_slots && !is_expansion && expansion_gold_reserve > 0 {
                        let lv = levels.get(idx).copied().unwrap_or(0);
                        let node = &UPGRADES.nodes[idx];
                        let gold_cost: i32 = if node.custom_cost {
                            expansion_cost(lv).1
                        } else {
                            let scale = upgrade_cost(lv);
                            node.cost
                                .iter()
                                .filter(|&&(kind, _)| kind == ResourceKind::Gold)
                                .map(|&(_, base)| base * scale)
                                .sum()
                        };
                        if gold_cost > 0 && gold_after - gold_cost < expansion_gold_reserve {
                            continue;
                        }
                    }
                    let mut w = weight;
                    if is_military_upgrade(idx) {
                        w *= 1.0 + desires.military_desire * 2.0;
                    }
                    if UPGRADES.nodes[idx].triggers_expansion {
                        // Delay expansion while town still has empty slots and can afford buildings.
                        // Previous check only looked at home targets -- missed farms, waypoints, roads.
                        if matches!(kind, AiKind::Builder) && ctx.has_slots {
                            let cheapest = building_cost(BuildingKind::Farm)
                                .min(building_cost(BuildingKind::FarmerHome))
                                .min(building_cost(BuildingKind::ArcherHome))
                                .min(building_cost(BuildingKind::MinerHome));
                            if food_after >= cheapest {
                                continue;
                            }
                        }
                        if ctx.slot_fullness > 0.7 {
                            w *= 2.0 + 4.0 * (ctx.slot_fullness - 0.7) / 0.3;
                        }
                        if !ctx.has_slots {
                            w *= 10.0;
                        }
                    }
                    upgrade_scores.push((AiAction::Upgrade(idx), w));
                }

                if let Some(action) = weighted_pick(&upgrade_scores) {
                    let mut new_mr = None;
                    let label = execute_action(
                        action,
                        &ctx,
                        &mut res,
                        &mut food_val,
                        mining_radius,
                        &mut new_mr,
                        snapshots.towns.get(&tdi),
                        personality,
                        road_style,
                        *difficulty,
                    );
                    if let Some(mr) = new_mr {
                        if let Some(mut p) = town_access.policy_mut(tdi as i32) {
                            p.0.mining_radius = mr;
                        }
                    }
                    if label.is_some() {
                        snapshots.towns.remove(&tdi);
                    }
                    if let Some(what) = label {
                        log_ai(
                            &mut combat_log,
                            &game_time,
                            faction,
                            &town_name,
                            pname,
                            &what,
                        );
                        let actions = &mut ai_state.players[pi].last_actions;
                        if actions.len() >= MAX_ACTION_HISTORY {
                            actions.pop_front();
                        }
                        actions.push_back((what, game_time.day(), game_time.hour()));
                    } else if debug {
                        let name = if let AiAction::Upgrade(idx) = action {
                            upgrade_node(idx).label
                        } else {
                            action.label()
                        };
                        let msg = format!("[dbg] upgrade {} FAILED", name);
                        let actions = &mut ai_state.players[pi].last_actions;
                        if actions.len() >= MAX_ACTION_HISTORY {
                            actions.pop_front();
                        }
                        actions.push_back((msg, game_time.day(), game_time.hour()));
                    }
                }
            }
        } // upgrade_enabled

        // Write back food changes from building actions to ECS
        if food_val != food {
            if let Some(mut f) = town_access.food_mut(tdi as i32) {
                f.0 = food_val;
            }
        }
    }
}
