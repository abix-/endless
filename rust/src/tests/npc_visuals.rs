//! NPC Visuals Test Scene
//! Spawns all NPC types in a labeled grid showing each render layer individually.
//! Stays on screen until user clicks Back.

use bevy::prelude::*;
use bevy_egui::{EguiContexts, egui};

use crate::components::*;
use crate::constants::*;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::render::MainCamera;
use crate::resources::NpcEntityMap;

use super::{TestState, TestSetupParams};

// Grid layout
const GRID_X_START: f32 = 200.0;
const GRID_Y_START: f32 = 480.0;
const COL_SPACING: f32 = 80.0;
const ROW_SPACING: f32 = 80.0;

// Column indices
const COL_BODY: usize = 0;
const COL_WEAPON: usize = 1;
const COL_HELMET: usize = 2;
const COL_ITEM: usize = 3;
const COL_SLEEP: usize = 4;
const COL_HEAL: usize = 5;
const COL_FULL: usize = 6;
const NUM_COLS: usize = 7;

// Row indices
const ROW_ARCHER: usize = 0;
const ROW_FARMER: usize = 1;
const ROW_RAIDER: usize = 2;
const ROW_FIGHTER: usize = 3;
const NUM_ROWS: usize = 4;

const ROW_LABELS: [&str; NUM_ROWS] = ["Archer", "Farmer", "Raider", "Fighter"];
const COL_LABELS: [&str; NUM_COLS] = ["Body", "+Weapon", "+Helmet", "Item", "Sleep", "Heal", "Full"];
const ROW_JOBS: [i32; NUM_ROWS] = [1, 0, 2, 3]; // Archer, Farmer, Raider, Fighter

fn grid_pos(row: usize, col: usize) -> Vec2 {
    Vec2::new(
        GRID_X_START + col as f32 * COL_SPACING,
        GRID_Y_START - row as f32 * ROW_SPACING,
    )
}

pub fn setup(mut params: TestSetupParams, mut farm_states: ResMut<crate::resources::GrowthStates>) {
    params.add_town("VisualTown");
    params.add_bed(380.0, 420.0);
    params.world_data.farms_mut().push(crate::world::PlacedBuilding::new(Vec2::new(450.0, 400.0), 0));
    farm_states.kinds.push(crate::resources::GrowthKind::Farm);
    farm_states.states.push(crate::resources::FarmGrowthState::Growing);
    farm_states.progress.push(0.0);
    farm_states.positions.push(Vec2::new(450.0, 400.0));
    farm_states.town_indices.push(Some(0));
    params.init_economy(1);
    params.game_time.time_scale = 0.0;

    // Spawn NPC grid: NUM_ROWS * NUM_COLS = 28 NPCs
    let mut first_slot = usize::MAX;
    for row in 0..NUM_ROWS {
        for col in 0..NUM_COLS {
            let pos = grid_pos(row, col);
            let job = ROW_JOBS[row];
            let slot = params.spawn_npc(job, pos.x, pos.y, pos.x, pos.y);
            if first_slot == usize::MAX {
                first_slot = slot;
            }
        }
    }

    params.test_state.counters.insert("first_slot".into(), first_slot as u32);
    params.test_state.phase_name = "Waiting for NPCs...".into();
    info!("npc-visuals: setup — {} NPCs in {}x{} grid", NUM_ROWS * NUM_COLS, NUM_ROWS, NUM_COLS);
}

pub fn tick(
    mut commands: Commands,
    npc_map: Res<NpcEntityMap>,
    mut test: ResMut<TestState>,
    time: Res<Time>,
    mut modified: Local<bool>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut camera_query: Query<(&mut Transform, &mut Projection), With<MainCamera>>,
    mut contexts: EguiContexts,
    windows: Query<&Window>,
    equip_query: Query<(
        Option<&EquippedWeapon>,
        Option<&EquippedHelmet>,
        Option<&EquippedArmor>,
    )>,
) {
    let Some(elapsed) = test.tick_elapsed(&time) else { return; };

    let first_slot = test.count("first_slot") as usize;
    let total = NUM_ROWS * NUM_COLS;

    // Phase 1: Wait for all NPCs to spawn, then modify components
    if test.phase == 1 && !*modified {
        // Check all NPCs exist
        let found = (0..total).filter(|i| npc_map.0.contains_key(&(first_slot + i))).count();
        if found < total {
            test.phase_name = format!("Spawning {}/{}...", found, total);
            if elapsed > 5.0 {
                test.fail_phase(elapsed, format!("Only {}/{} NPCs spawned", found, total));
            }
            return;
        }

        // Position camera at grid center
        if let Ok((mut transform, mut projection)) = camera_query.single_mut() {
            let center = Vec2::new(
                GRID_X_START + (NUM_COLS as f32 - 1.0) * COL_SPACING / 2.0,
                GRID_Y_START - (NUM_ROWS as f32 - 1.0) * ROW_SPACING / 2.0,
            );
            transform.translation.x = center.x;
            transform.translation.y = center.y;
            if let Projection::Orthographic(ref mut ortho) = *projection {
                ortho.scale = 0.25; // 4x zoom
            }
        }

        // Modify components per column
        for row in 0..NUM_ROWS {
            for col in 0..NUM_COLS {
                let slot = first_slot + row * NUM_COLS + col;
                let Some(&entity) = npc_map.0.get(&slot) else { continue };

                // Stop movement
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx: slot, speed: 0.0 }));

                match col {
                    COL_BODY => {
                        // Body only: remove all equipment
                        commands.entity(entity).remove::<EquippedWeapon>();
                        commands.entity(entity).remove::<EquippedHelmet>();
                        commands.entity(entity).remove::<EquippedArmor>();
                    }
                    COL_WEAPON => {
                        // Body + weapon only
                        commands.entity(entity).remove::<EquippedHelmet>();
                        commands.entity(entity).remove::<EquippedArmor>();
                        // Insert weapon if not already present (farmers/fighters don't have one)
                        if equip_query.get(entity).map(|(w, _, _)| w.is_none()).unwrap_or(true) {
                            commands.entity(entity).insert(EquippedWeapon(EQUIP_SWORD.0, EQUIP_SWORD.1));
                        }
                    }
                    COL_HELMET => {
                        // Body + helmet only
                        commands.entity(entity).remove::<EquippedWeapon>();
                        commands.entity(entity).remove::<EquippedArmor>();
                        // Insert helmet if not already present
                        if equip_query.get(entity).map(|(_, h, _)| h.is_none()).unwrap_or(true) {
                            commands.entity(entity).insert(EquippedHelmet(EQUIP_HELMET.0, EQUIP_HELMET.1));
                        }
                    }
                    COL_ITEM => {
                        // Show food sprite as body (world atlas)
                        commands.entity(entity).remove::<EquippedWeapon>();
                        commands.entity(entity).remove::<EquippedHelmet>();
                        commands.entity(entity).remove::<EquippedArmor>();
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx: slot, col: FOOD_SPRITE.0, row: FOOD_SPRITE.1, atlas: 1.0 }));
                    }
                    COL_SLEEP => {
                        // Show sleep sprite as body (character sheet)
                        commands.entity(entity).remove::<EquippedWeapon>();
                        commands.entity(entity).remove::<EquippedHelmet>();
                        commands.entity(entity).remove::<EquippedArmor>();
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx: slot, col: SLEEP_SPRITE.0, row: SLEEP_SPRITE.1, atlas: 0.0 }));
                    }
                    COL_HEAL => {
                        // Show heal sprite as body (character sheet)
                        commands.entity(entity).remove::<EquippedWeapon>();
                        commands.entity(entity).remove::<EquippedHelmet>();
                        commands.entity(entity).remove::<EquippedArmor>();
                        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx: slot, col: HEAL_SPRITE.0, row: HEAL_SPRITE.1, atlas: 0.0 }));
                    }
                    COL_FULL => {
                        // Full: keep all equipment as spawned (guard has weapon+helmet, raider has weapon)
                    }
                    _ => {}
                }
            }
        }

        *modified = true;
        test.pass_phase(elapsed, format!("All {} NPCs configured", total));
        return;
    }

    // Egui overlay: labels at world positions
    let Ok(ctx) = contexts.ctx_mut() else { return };
    let Ok(window) = windows.single() else { return };
    let Ok((cam_transform, cam_projection)) = camera_query.single() else { return };
    let Projection::Orthographic(ref ortho) = *cam_projection else { return };

    let zoom = 1.0 / ortho.scale;
    let cam_pos = cam_transform.translation.truncate();
    let viewport = Vec2::new(window.width(), window.height());

    let world_to_screen = |world_pos: Vec2| -> egui::Pos2 {
        let offset = (world_pos - cam_pos) * zoom;
        egui::Pos2::new(
            offset.x + viewport.x / 2.0,
            viewport.y / 2.0 - offset.y,
        )
    };

    // Column headers (above top row)
    for col in 0..NUM_COLS {
        let header_pos = grid_pos(0, col) + Vec2::new(0.0, 40.0);
        let screen = world_to_screen(header_pos);
        egui::Area::new(egui::Id::new(format!("col_header_{}", col)))
            .fixed_pos(screen)
            .pivot(egui::Align2::CENTER_BOTTOM)
            .interactable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(COL_LABELS[col]).strong().size(12.0).color(egui::Color32::WHITE));
            });
    }

    // Row labels (left of first column)
    for row in 0..NUM_ROWS {
        let label_pos = grid_pos(row, 0) - Vec2::new(60.0, 0.0);
        let screen = world_to_screen(label_pos);
        egui::Area::new(egui::Id::new(format!("row_label_{}", row)))
            .fixed_pos(screen)
            .pivot(egui::Align2::CENTER_CENTER)
            .interactable(false)
            .show(ctx, |ui| {
                let color = match row {
                    ROW_ARCHER => egui::Color32::from_rgb(77, 102, 255),
                    ROW_FARMER => egui::Color32::from_rgb(102, 255, 102),
                    ROW_RAIDER => egui::Color32::from_rgb(255, 102, 102),
                    ROW_FIGHTER => egui::Color32::from_rgb(255, 255, 102),
                    _ => egui::Color32::WHITE,
                };
                ui.label(egui::RichText::new(ROW_LABELS[row]).strong().size(14.0).color(color));
            });
    }

    // Sprite coordinate labels below each NPC
    for row in 0..NUM_ROWS {
        for col in 0..NUM_COLS {
            let pos = grid_pos(row, col) - Vec2::new(0.0, 20.0);
            let screen = world_to_screen(pos);

            // Determine what sprite info to show
            let info = match col {
                COL_BODY => {
                    let (sc, sr) = match row {
                        ROW_ARCHER => npc_def(Job::Archer).sprite,
                        ROW_FARMER => npc_def(Job::Farmer).sprite,
                        ROW_RAIDER => npc_def(Job::Raider).sprite,
                        ROW_FIGHTER => npc_def(Job::Fighter).sprite,
                        _ => (0.0, 0.0),
                    };
                    format!("({:.0},{:.0})", sc, sr)
                }
                COL_WEAPON => format!("wep({:.0},{:.0})", EQUIP_SWORD.0, EQUIP_SWORD.1),
                COL_HELMET => format!("hlm({:.0},{:.0})", EQUIP_HELMET.0, EQUIP_HELMET.1),
                COL_ITEM => format!("food({:.0},{:.0})", FOOD_SPRITE.0, FOOD_SPRITE.1),
                COL_SLEEP => format!("zzz({:.0},{:.0})", SLEEP_SPRITE.0, SLEEP_SPRITE.1),
                COL_HEAL => format!("heal({:.0},{:.0})", HEAL_SPRITE.0, HEAL_SPRITE.1),
                COL_FULL => "all".into(),
                _ => String::new(),
            };

            egui::Area::new(egui::Id::new(format!("sprite_{}_{}", row, col)))
                .fixed_pos(screen)
                .pivot(egui::Align2::CENTER_TOP)
                .interactable(false)
                .show(ctx, |ui| {
                    ui.label(egui::RichText::new(info).size(10.0).color(egui::Color32::GRAY));
                });
        }
    }
}
