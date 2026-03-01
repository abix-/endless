//! Fountain Shot Readback Test (3 phases)
//! Validates assumption: repeated fountain shots can alternate between fresh and stale readback positions.

use bevy::prelude::*;

use crate::messages::{GpuUpdate, GpuUpdateMsg, SpawnNpcMsg};
use crate::resources::*;
use crate::world;

use super::TestState;

const ENEMY_START_X: f32 = 540.0;
const ENEMY_START_Y: f32 = 400.0;
const MISMATCH_EPSILON: f32 = 8.0;

pub fn setup(
    mut slot_alloc: ResMut<GpuSlotPool>,
    mut entity_map: ResMut<EntityMap>,
    mut spawn_events: MessageWriter<SpawnNpcMsg>,
    mut world_data: ResMut<world::WorldData>,
    mut food_storage: ResMut<FoodStorage>,
    mut faction_stats: ResMut<FactionStats>,
    mut test_state: ResMut<TestState>,
    mut camera_query: Query<&mut Transform, With<crate::render::MainCamera>>,
    mut uid_alloc: ResMut<crate::resources::NextEntityUid>,
    mut commands: Commands,
    mut gpu_updates: MessageWriter<crate::messages::GpuUpdateMsg>,
) {
    // One town with a fountain tower.
    world_data.towns.push(world::Town {
        name: "FountainTown".into(),
        center: Vec2::new(400.0, 400.0),
        faction: 0,
        sprite_type: 0,
    });
    food_storage.init(1);
    faction_stats.init(2);

    // Create fountain instance directly
    let _ = world::place_building(
        &mut slot_alloc, &mut entity_map, &mut uid_alloc, &mut commands, &mut gpu_updates,
        world::BuildingKind::Fountain, Vec2::new(400.0, 400.0), 0, 0, 0, 0,
        None, None, None, None,
    );

    // One enemy NPC in fountain range; keep this NPC pinned in tick so tower fires repeatedly.
    let building_gpu_slot = slot_alloc.alloc_reset().expect("slot alloc for target");
    spawn_events.write(SpawnNpcMsg {
        slot_idx: building_gpu_slot,
        x: ENEMY_START_X,
        y: ENEMY_START_Y,
        job: 0,
        faction: 1,
        town_idx: 0,
        home_x: ENEMY_START_X,
        home_y: ENEMY_START_Y,
        work_x: -1.0,
        work_y: -1.0,
        starting_post: -1,
        attack_type: 0,
        uid_override: None,
    });

    if let Ok(mut cam) = camera_query.single_mut() {
        cam.translation.x = 400.0;
        cam.translation.y = 400.0;
    }
    test_state.phase_name = "Waiting for first tower projectile...".into();
    test_state
        .counters
        .insert("building_gpu_slot".into(), building_gpu_slot as u32);
    test_state.counters.insert("tower_spawns".into(), 0);
    test_state.counters.insert("mismatch_total".into(), 0);
    test_state.counters.insert("odd_mismatch".into(), 0);
    test_state.counters.insert("even_mismatch".into(), 0);
    test_state.counters.insert("mismatch_transitions".into(), 0);
    test_state.set_flag("has_last_mismatch", false);
    test_state.set_flag("last_mismatch", false);

    info!(
        "fountain-shot-stale: setup complete (building_gpu_slot={}, enemy=({:.1},{:.1}))",
        building_gpu_slot, ENEMY_START_X, ENEMY_START_Y
    );
}

pub fn tick(
    time: Res<Time>,
    gpu_state: Res<GpuReadState>,
    combat_debug: Res<CombatDebug>,
    writes: Res<crate::gpu::ProjBufferWrites>,
    proj_pos_state: Res<ProjPositionState>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut test: ResMut<TestState>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else {
        return;
    };

    // Pin target so the tower keeps shooting in a stable lane.
    let building_gpu_slot = test.count("building_gpu_slot") as usize;
    let (tx, ty) = if building_gpu_slot * 2 + 1 < gpu_state.positions.len() {
        (
            gpu_state.positions[building_gpu_slot * 2],
            gpu_state.positions[building_gpu_slot * 2 + 1],
        )
    } else {
        (ENEMY_START_X, ENEMY_START_Y)
    };
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
        idx: building_gpu_slot,
        x: tx,
        y: ty,
    }));

    // Analyze only new tower projectile spawns this frame (tower shots have shooter == building slot).
    for &idx in &writes.spawn_dirty_indices {
        let spawn_order = test.count("tower_spawns") + 1;
        test.inc("tower_spawns");

        let i2 = idx * 2;
        if i2 + 1 >= writes.positions.len() {
            continue;
        }

        let spawn_x = writes.positions[i2];
        let spawn_y = writes.positions[i2 + 1];
        let (read_x, read_y) = if i2 + 1 < proj_pos_state.0.len() {
            (proj_pos_state.0[i2], proj_pos_state.0[i2 + 1])
        } else {
            (-9999.0, -9999.0)
        };

        let dx = read_x - spawn_x;
        let dy = read_y - spawn_y;
        let mismatch = (dx * dx + dy * dy).sqrt() > MISMATCH_EPSILON;

        if mismatch {
            test.inc("mismatch_total");
            if spawn_order % 2 == 1 {
                test.inc("odd_mismatch");
            } else {
                test.inc("even_mismatch");
            }
        }

        if test.get_flag("has_last_mismatch") {
            let last = test.get_flag("last_mismatch");
            if last != mismatch {
                test.inc("mismatch_transitions");
            }
        }
        test.set_flag("last_mismatch", mismatch);
        test.set_flag("has_last_mismatch", true);
    }

    let spawns = test.count("tower_spawns");
    let mismatch_total = test.count("mismatch_total");
    let odd = test.count("odd_mismatch");
    let even = test.count("even_mismatch");
    let transitions = test.count("mismatch_transitions");

    match test.phase {
        // Phase 1: confirm tower has started firing.
        1 => {
            test.phase_name = format!(
                "attacks={} tower_spawns={}",
                combat_debug.attacks_made, spawns
            );
            if spawns >= 1 {
                test.pass_phase(
                    elapsed,
                    format!("first tower spawn seen (spawns={})", spawns),
                );
            } else if elapsed > 20.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "no tower projectile spawn (attacks={})",
                        combat_debug.attacks_made
                    ),
                );
            }
        }
        // Phase 2: collect enough samples to detect parity behavior.
        2 => {
            test.phase_name = format!(
                "spawns={} mismatch={} odd={} even={}",
                spawns, mismatch_total, odd, even
            );
            if spawns >= 6 {
                test.pass_phase(
                    elapsed,
                    format!("collected {} tower projectile spawns", spawns),
                );
            } else if elapsed > 60.0 {
                test.fail_phase(
                    elapsed,
                    format!("insufficient spawns={} (need >=6)", spawns),
                );
            }
        }
        // Phase 3: validate alternating mismatch signature.
        3 => {
            test.phase_name = format!(
                "mismatch={} odd={} even={} transitions={}",
                mismatch_total, odd, even, transitions
            );
            let alternating_bias = (odd > 0 && even == 0) || (even > 0 && odd == 0);
            if alternating_bias {
                test.pass_phase(
                    elapsed,
                    format!(
                        "alternating stale pattern observed (odd={}, even={}, transitions={})",
                        odd, even, transitions
                    ),
                );
                test.complete(elapsed);
            } else if elapsed > 80.0 {
                test.fail_phase(
                    elapsed,
                    format!(
                        "no alternating stale pattern (mismatch={}, odd={}, even={}, transitions={})",
                        mismatch_total, odd, even, transitions
                    ),
                );
            }
        }
        _ => {}
    }
}
