//! Slot Reuse Wave Test (5 phases)
//! Validates: AI wave targets building → building destroyed → slot reused by new building
//!   → wave should end but doesn't (ABA bug in SlotPool).
//!
//! Uses setup_world for proper 2-town setup, then manually places a player farm
//! near the AI town as a target. AI archers spawn from 5 archer homes.
//! After wave dispatches, we destroy the target and reuse its slot to prove the bug.

use bevy::prelude::*;

use crate::components::Health;
use crate::resources::*;
use crate::systems::{AiPersonality, AiPlayerState};
use crate::world::{self, BuildingKind, WorldGenStyle};

use super::{BuildingInitParams, TestScenarioSetup, TestState};

/// Persistent state across phases — tracks the target slot for verification.
#[derive(Resource, Default)]
pub struct SlotReuseTestState {
    /// UID of the player farm that the AI wave targets.
    pub target_entity: Option<Entity>,
    /// Slot index of the target (for direct EntityMap lookups in test).
    pub building_gpu_slot: Option<usize>,
    /// Position of the original target building.
    pub target_pos: Option<Vec2>,
    /// Set true once we've destroyed the target.
    pub target_destroyed: bool,
    /// Set true once we've placed a new building that reused the slot.
    pub slot_reused: bool,
    /// Position of the new building that reused the slot.
    pub reuse_pos: Option<Vec2>,
    /// Seconds waited in phase 4 for heartbeat.
    pub heartbeat_wait: f32,
    /// Index of the AI attack squad we're tracking.
    pub attack_squad_idx: Option<usize>,
}

pub fn setup(
    mut commands: Commands,
    mut world_data: ResMut<world::WorldData>,
    mut world_grid: ResMut<world::WorldGrid>,
    mut config: ResMut<world::WorldGenConfig>,
    mut faction_stats: ResMut<FactionStats>,

    mut slot_alloc: ResMut<GpuSlotPool>,
    mut bld: BuildingInitParams,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
    _spawn_writer: MessageWriter<crate::messages::SpawnNpcMsg>,
    mut state: TestScenarioSetup,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut town_index: ResMut<crate::resources::TownIndex>,
) {
    // 1 player town + 1 AI builder town, no raiders
    config.gen_style = WorldGenStyle::Continents;
    config.num_towns = 1;
    config.ai_towns = 1;
    config.raider_towns = 0;
    config.world_width = 10000.0;
    config.world_height = 10000.0;
    config.world_margin = 300.0;
    config.min_town_distance = 2000.0;

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
        &mut town_index,
        &mut commands,
        &mut gpu_updates,
    );
    state.ai_state.players = ai_players;

    // Give AI town plenty of food for spawning archers
    for player in &state.ai_state.players {
        let ti = player.town_data_idx as i32;
        if let Some(&e) = town_index.0.get(&ti) {
            commands.entity(e).insert(crate::components::FoodStore(500));
        }
    }

    // Place 5 extra archer homes for the AI town so military spawns quickly
    if let Some(player) = state.ai_state.players.first() {
        let ti = player.town_data_idx;
        if let Some(town) = world_data.towns.get(ti) {
            let center = town.center;
            let faction = town.faction;
            for i in 0..5 {
                let offset = Vec2::new(32.0 * (i as f32 + 1.0), 64.0);
                let _ = world::place_building(
                    &mut slot_alloc,
                    &mut bld.entity_map,
                    &mut commands,
                    &mut gpu_updates,
                    BuildingKind::ArcherHome,
                    center + offset,
                    ti as u32,
                    faction,
                    &Default::default(),
                    None,
                    None,
                );
            }
        }
    }

    // Place a player farm near the AI town as the attack target
    if let Some(ai_player) = state.ai_state.players.first() {
        let ai_ti = ai_player.town_data_idx;
        if let Some(ai_town) = world_data.towns.get(ai_ti) {
            let farm_pos = ai_town.center + Vec2::new(-200.0, 0.0);
            let player_ti = world_data
                .towns
                .iter()
                .position(|t| t.faction == crate::constants::FACTION_PLAYER)
                .unwrap_or(0);
            let _ = world::place_building(
                &mut slot_alloc,
                &mut bld.entity_map,
                &mut commands,
                &mut gpu_updates,
                BuildingKind::Farm,
                farm_pos,
                player_ti as u32,
                0,
                &Default::default(),
                None,
                None,
            );
        }
    }

    // Aggressive personality → low wave_min_start (3), attacks everything
    for player in &mut state.ai_state.players {
        player.personality = AiPersonality::Aggressive;
    }

    state.ai_config.decision_interval = 1.0;
    state.endless.enabled = true;
    state.game_time.time_scale = 4.0; // speed up spawning

    // Init test-local resource
    commands.insert_resource(SlotReuseTestState::default());

    // Focus camera on AI town
    if let Some(ai_player) = state.ai_state.players.first() {
        let ai_ti = ai_player.town_data_idx;
        if let Some(town) = world_data.towns.get(ai_ti) {
            if let Ok(mut cam) = camera_query.single_mut() {
                cam.translation.x = town.center.x;
                cam.translation.y = town.center.y;
            }
        }
    }

    state.test_state.phase_name = "Waiting for AI wave...".into();
    info!(
        "slot-reuse-wave: setup — 1 player town, 1 AI builder (Aggressive), 5 extra archer homes"
    );
}

pub fn tick(
    mut entity_map: ResMut<EntityMap>,
    ai_state: Res<AiPlayerState>,
    squad_state: Res<SquadState>,
    mut slot_alloc: ResMut<GpuSlotPool>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    mut local: ResMut<SlotReuseTestState>,
    mut health_q: Query<&mut Health>,
    world_data: Res<world::WorldData>,
    mut commands: Commands,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    match test.phase {
        // Phase 1: Wait for an AI attack squad wave to dispatch
        1 => {
            test.phase_name = "Waiting for AI wave dispatch...".into();

            // Find the AI attack squad with wave_active
            for player in &ai_state.players {
                for &si in &player.squad_indices {
                    if let Some(squad) = squad_state.squads.get(si) {
                        if squad.wave_active {
                            // Found active wave — record target UID + slot
                            if let Some(cmd) = player.squad_cmd.get(&si) {
                                if let Some(entity) = cmd.building_uid {
                                    let slot = entity_map.slot_for_entity(entity);
                                    local.target_entity = Some(entity);
                                    local.building_gpu_slot = slot;
                                    local.target_pos = squad.target;
                                    local.attack_squad_idx = Some(si);
                                    let pos_str = squad
                                        .target
                                        .map(|p| format!("({:.0}, {:.0})", p.x, p.y))
                                        .unwrap_or("None".into());
                                    test.pass_phase(
                                        elapsed,
                                        format!(
                                            "wave active: squad {} → entity {:?} (slot {:?}) at {}",
                                            si, entity, slot, pos_str,
                                        ),
                                    );
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            if elapsed > 30.0 {
                // Debug: show squad state
                let squad_count: usize =
                    ai_state.players.iter().map(|p| p.squad_indices.len()).sum();
                let total_members: usize = ai_state
                    .players
                    .iter()
                    .flat_map(|p| p.squad_indices.iter())
                    .filter_map(|&si| squad_state.squads.get(si))
                    .map(|s| s.members.len())
                    .sum();
                test.fail_phase(
                    elapsed,
                    format!(
                        "no active wave after 30s (squads={}, total_members={})",
                        squad_count, total_members,
                    ),
                );
            }
        }

        // Phase 2: Destroy the target building
        2 => {
            let Some(slot) = local.building_gpu_slot else {
                test.fail_phase(elapsed, "no target slot recorded");
                return;
            };

            if !local.target_destroyed {
                test.phase_name = format!("Destroying building at slot {}...", slot);

                // Set HP to 0 — death_system will process next frame
                if let Some(&entity) = entity_map.entities.get(&slot) {
                    if let Ok(mut hp) = health_q.get_mut(entity) {
                        hp.0 = 0.0;
                        local.target_destroyed = true;
                        info!("slot-reuse-wave: set HP=0 on slot {}", slot);
                    }
                } else {
                    // Building might already be gone (death_system processed it)
                    local.target_destroyed = true;
                }
                return;
            }

            // Wait for death_system to remove it from EntityMap
            test.phase_name = format!("Waiting for slot {} removal from EntityMap...", slot);
            if entity_map.get_instance(slot).is_none() {
                test.pass_phase(elapsed, format!("slot {} removed from EntityMap", slot));
            } else if elapsed > 5.0 {
                test.fail_phase(
                    elapsed,
                    format!("slot {} still in EntityMap after 5s", slot),
                );
            }
        }

        // Phase 3: Trigger slot reuse by placing a new building
        3 => {
            let Some(slot) = local.building_gpu_slot else {
                test.fail_phase(elapsed, "no target slot recorded");
                return;
            };

            if !local.slot_reused {
                test.phase_name = "Placing new building to trigger slot reuse...".into();

                // Place a new building — the freed slot should be reused (LIFO)
                let ai_ti = ai_state
                    .players
                    .first()
                    .map(|p| p.town_data_idx)
                    .unwrap_or(1);
                let ai_faction = world_data.towns.get(ai_ti).map(|t| t.faction).unwrap_or(1);
                let new_pos = Vec2::new(200.0, 200.0); // far from original target
                let new_slot = world::place_building(
                    &mut slot_alloc,
                    &mut entity_map,
                    &mut commands,
                    &mut gpu_updates,
                    BuildingKind::Farm,
                    new_pos,
                    ai_ti as u32,
                    ai_faction,
                    &Default::default(),
                    None,
                    None,
                );

                if let Ok(ns) = new_slot {
                    local.slot_reused = true;
                    local.reuse_pos = Some(new_pos);
                    if ns == slot {
                        test.pass_phase(elapsed, format!(
                            "LIFO reuse confirmed: new building got same slot {} at ({:.0}, {:.0})",
                            ns, new_pos.x, new_pos.y,
                        ));
                    } else {
                        // Slot wasn't reused — someone else allocated first. Still useful data.
                        test.pass_phase(elapsed, format!(
                            "slot NOT reused (got {} instead of {}). ABA won't trigger this run.",
                            ns, slot,
                        ));
                    }
                } else {
                    test.fail_phase(elapsed, "place_building returned Err");
                }
            }
        }

        // Phase 4: Wait for squad commander heartbeat — check if wave ended
        4 => {
            let Some(si) = local.attack_squad_idx else {
                test.fail_phase(elapsed, "no attack squad index recorded");
                return;
            };

            local.heartbeat_wait += time.delta_secs();
            test.phase_name = format!(
                "Waiting for heartbeat ({:.1}s / 3.0s)...",
                local.heartbeat_wait,
            );

            // Wait at least 3 real seconds (> heartbeat interval) for commander to process
            if local.heartbeat_wait < 3.0 {
                return;
            }

            if let Some(squad) = squad_state.squads.get(si) {
                if !squad.wave_active {
                    // FIX CONFIRMED: wave ended correctly after target destroyed (UID-based identity)
                    test.pass_phase(elapsed, "fix confirmed: wave ends after target destroyed");
                } else {
                    // Bug still present — UID lookup should have resolved this
                    let target_str = squad
                        .target
                        .map(|p| format!("({:.0}, {:.0})", p.x, p.y))
                        .unwrap_or("None".into());
                    test.fail_phase(
                        elapsed,
                        format!(
                            "wave still active after target destroyed (UID bug?). squad.target={}",
                            target_str,
                        ),
                    );
                }
            } else {
                test.fail_phase(elapsed, format!("squad {} no longer exists", si));
            }
        }

        // Phase 5: Report
        5 => {
            let slot = local.building_gpu_slot.unwrap_or(0);
            let orig_pos = local
                .target_pos
                .map(|p| format!("({:.0}, {:.0})", p.x, p.y))
                .unwrap_or("?".into());
            let reuse_pos = local
                .reuse_pos
                .map(|p| format!("({:.0}, {:.0})", p.x, p.y))
                .unwrap_or("?".into());
            let si = local.attack_squad_idx.unwrap_or(0);
            let wave_active = squad_state
                .squads
                .get(si)
                .map(|s| s.wave_active)
                .unwrap_or(false);
            let resolve_result = entity_map
                .get_instance(slot)
                .map(|inst| {
                    format!(
                        "{:?} at ({:.0}, {:.0})",
                        inst.kind, inst.position.x, inst.position.y
                    )
                })
                .unwrap_or("None".into());

            info!("========================================");
            info!("SLOT REUSE WAVE BUG REPORT");
            info!("  building_gpu_slot: {}", slot);
            info!("  original_pos: {}", orig_pos);
            info!("  reuse_pos: {}", reuse_pos);
            info!("  resolve_building_pos(slot): {}", resolve_result);
            info!("  wave_active: {}", wave_active);
            info!("========================================");

            test.pass_phase(
                elapsed,
                format!(
                    "slot={} orig={} reuse={} resolve={} wave_active={}",
                    slot, orig_pos, reuse_pos, resolve_result, wave_active,
                ),
            );
            test.complete(elapsed);
        }

        _ => {}
    }
}
