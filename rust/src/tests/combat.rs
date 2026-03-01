//! Combat Pipeline Test (6 phases)
//! Validates: GPU targeting → Fighting → projectile/damage → health decreases → death → slot freed.

use bevy::prelude::*;

use crate::messages::SpawnNpcMsg;
use crate::resources::*;
use crate::world;

use super::TestState;

pub fn setup(
    mut slot_alloc: ResMut<GpuSlotPool>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut policies: ResMut<TownPolicies>,
    mut squad_state: ResMut<SquadState>,
    mut test_state: ResMut<TestState>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
) {
    // Keep this test deterministic even after user/debug squad interactions.
    for squad in squad_state.squads.iter_mut() {
        if !squad.is_player() {
            continue;
        }
        squad.members.clear();
        squad.target = None;
        squad.target_size = 0;
        squad.patrol_enabled = true;
        squad.rest_when_tired = true;
        squad.hold_fire = false;
    }
    squad_state.selected = 0;
    squad_state.placing_target = false;
    squad_state.drag_start = None;
    squad_state.box_selecting = false;
    squad_state.dc_no_return = false;

    // Two opposing towns
    world_data.towns.push(world::Town {
        name: "Town".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    world_data.towns.push(world::Town {
        name: "Raider Town".into(),
        center: Vec2::new(400.0, 200.0),
        faction: 1,
        sprite_type: 1,
    });
    food_storage.init(2);
    faction_stats.init(2);
    policies.policies.resize(2, PolicySet::default());
    // Test-scene override: archers flee at 5% HP.
    policies.policies[0].archer_flee_hp = 0.05;
    // Keep combat test focused on fighting (avoid early heal breakoff).
    policies.policies[0].recovery_hp = 0.05;

    // Spawn 1 guard (faction 0) and 1 raider (faction 1) close together.
    let slot0 = slot_alloc.alloc_reset().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot0,
        x: 400.0,
        y: 310.0,
        job: 1,
        faction: 0,
        town_idx: 0,
        home_x: 400.0,
        home_y: 400.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1, // no patrol — just stands there
        attack_type: 0,
        uid_override: None,
    });
    let slot1 = slot_alloc.alloc_reset().expect("slot alloc");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: slot1,
        x: 400.0,
        y: 290.0,
        job: 2,
        faction: 1,
        town_idx: 1,
        home_x: 400.0,
        home_y: 200.0,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        attack_type: 0,
        uid_override: None,
    });

    // Focus the camera on the combat setup so this test is immediately visible.
    if let Ok(mut cam) = camera_query.single_mut() {
        cam.translation.x = 400.0;
        cam.translation.y = 300.0;
    }

    test_state.phase_name = "Waiting for spawns...".into();
    info!("combat: setup — 1 guard vs 1 raider, 20px apart");
}

pub fn tick(
    entity_map: Res<EntityMap>,
    combat_debug: Res<CombatDebug>,
    health_debug: Res<HealthDebug>,
    slot_alloc: Res<GpuSlotPool>,
    time: Res<Time>,
    mut test: ResMut<TestState>,
    combat_state_q: Query<&crate::components::CombatState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    let alive = entity_map.iter_npcs().filter(|n| !n.dead).count();
    let in_combat = entity_map
        .iter_npcs()
        .filter(|n| {
            !n.dead
                && combat_state_q
                    .get(n.entity)
                    .is_ok_and(|cs| cs.is_fighting())
        })
        .count();

    match test.phase {
        // Phase 1: GPU targeting finds enemy
        1 => {
            test.phase_name = format!(
                "targets_found={} alive={}",
                combat_debug.targets_found, alive
            );
            if combat_debug.targets_found > 0 {
                test.pass_phase(
                    elapsed,
                    format!("targets_found={}", combat_debug.targets_found),
                );
            } else if elapsed > 10.0 {
                test.fail_phase(elapsed, format!("targets_found=0 alive={}", alive));
            }
        }
        // Phase 2: CombatState::Fighting entered
        2 => {
            test.phase_name = format!("in_combat={}", in_combat);
            if in_combat > 0 {
                test.pass_phase(elapsed, format!("in_combat={}", in_combat));
            } else if elapsed > 15.0 {
                test.fail_phase(
                    elapsed,
                    format!("in_combat=0 targets={}", combat_debug.targets_found),
                );
            }
        }
        // Phase 3: Damage dealt
        3 => {
            test.phase_name = format!(
                "damage={} attacks={}",
                health_debug.damage_processed, combat_debug.attacks_made
            );
            if health_debug.damage_processed > 0 || combat_debug.attacks_made > 0 {
                test.pass_phase(
                    elapsed,
                    format!(
                        "damage={} attacks={}",
                        health_debug.damage_processed, combat_debug.attacks_made
                    ),
                );
            } else if elapsed > 20.0 {
                test.fail_phase(elapsed, format!("damage=0 attacks=0"));
            }
        }
        // Phase 4: Health decreases (tracked via debug)
        4 => {
            // damage_processed already confirmed in phase 3, just confirm cumulative
            let total_damage = health_debug.damage_processed;
            test.phase_name = format!("total_damage={}", total_damage);
            if total_damage > 0 {
                test.pass_phase(elapsed, format!("total_damage={}", total_damage));
            } else if elapsed > 25.0 {
                test.fail_phase(elapsed, "no damage recorded");
            }
        }
        // Phase 5: NPC dies
        5 => {
            test.phase_name = format!(
                "alive={}/2 deaths={}",
                alive, health_debug.deaths_this_frame
            );
            if alive < 2 || health_debug.deaths_this_frame > 0 {
                test.pass_phase(
                    elapsed,
                    format!("alive={} deaths={}", alive, health_debug.deaths_this_frame),
                );
            } else if elapsed > 45.0 {
                test.fail_phase(elapsed, format!("alive={} (no deaths)", alive));
            }
        }
        // Phase 6: Slot freed, entity despawned
        6 => {
            let free = slot_alloc.free_list().len();
            test.phase_name = format!("free_slots={} alive={}", free, alive);
            if free > 0 {
                test.pass_phase(elapsed, format!("slot freed (free={})", free));
                test.complete(elapsed);
            } else if elapsed > 50.0 {
                test.fail_phase(elapsed, format!("free_slots=0 alive={}", alive));
            }
        }
        _ => {}
    }
}
