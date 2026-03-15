//! Mine analysis -- single-pass over gold_mines per AI tick.

use bevy::prelude::*;

use crate::resources::EntityMap;
use crate::world::{self, BuildingKind};

use super::{AiBuildRes, AiKind, AiTownSnapshot};

/// Pre-computed mine stats for one AI town. Built once per tick, used by scoring + execution.
pub(super) struct MineAnalysis {
    pub in_radius: usize,
    pub outside_radius: usize,
    /// All alive mine positions on the map (for miner home slot scoring).
    pub all_positions: Vec<Vec2>,
}

pub(super) fn analyze_mines(
    entity_map: &EntityMap,
    center: Vec2,
    mining_radius: f32,
) -> MineAnalysis {
    // Single-pass analysis over alive gold mines.
    let radius_sq = mining_radius * mining_radius;
    let mut in_radius = 0usize;
    let mut outside_radius = 0usize;
    let mut all_positions = Vec::new();

    for m in entity_map.iter_kind(BuildingKind::GoldMine) {
        all_positions.push(m.position);
        if (m.position - center).length_squared() <= radius_sq {
            in_radius += 1;
        } else {
            outside_radius += 1;
        }
    }

    MineAnalysis {
        in_radius,
        outside_radius,
        all_positions,
    }
}

/// Per-tick derived context for one AI town. Built once before scoring/execution.
pub(super) struct TownContext {
    pub center: Vec2,
    pub ti: u32,
    pub tdi: usize,
    pub area_level: i32,
    pub food: i32,
    pub has_slots: bool,
    pub slot_fullness: f32,
    pub mines: Option<MineAnalysis>,
}

impl TownContext {
    pub fn build(
        tdi: usize,
        food: i32,
        snapshot: Option<&AiTownSnapshot>,
        res: &AiBuildRes,
        kind: AiKind,
        mining_radius: f32,
        town_area_level: i32,
    ) -> Option<Self> {
        let town = res.world.world_data.towns.get(tdi)?;
        let center = snapshot.map(|s| s.center).unwrap_or(town.center);
        let area_level = town_area_level;
        let ti = tdi as u32;
        let empty_count = snapshot.map(|s| s.empty_slots.len()).unwrap_or_else(|| {
            world::empty_slots(tdi, center, &res.world.grid, &res.world.entity_map).len()
        });
        let (min_c, max_c, min_r, max_r) = world::build_bounds(area_level, center, &res.world.grid);
        let total = ((max_c - min_c + 1) * (max_r - min_r + 1) - 1) as f32;
        let slot_fullness = 1.0 - empty_count as f32 / total.max(1.0);
        let mines = match kind {
            AiKind::Builder => Some(analyze_mines(&res.world.entity_map, center, mining_radius)),
            AiKind::Raider => None,
        };
        Some(Self {
            center,
            ti,
            tdi,
            area_level,
            food,
            has_slots: empty_count > 0,
            slot_fullness,
            mines,
        })
    }
}
