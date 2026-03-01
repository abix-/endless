//! Save/Load system with quicksave/quickload shortcuts and JSON serialization.
//! Save format is self-contained: dedicated serde structs decouple from ECS types.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::*;
use crate::constants::{ItemKind, MAX_SQUADS};
use crate::messages::GpuUpdateMsg;
use crate::resources::*;
use crate::settings::{ControlAction, UserSettings};
use crate::systems::AiPlayerState;
use crate::systems::spawn::{NpcSpawnOverrides, materialize_npc};
use crate::systems::stats::{
    CombatConfig, TownUpgrades, decode_auto_upgrade_flags, decode_upgrade_levels,
};
use crate::world::{self, TownGrids, WorldCell, WorldData, WorldGrid};

// ============================================================================
// VEC2 HELPERS
// ============================================================================

fn v2(v: Vec2) -> [f32; 2] {
    [v.x, v.y]
}
fn to_vec2(a: [f32; 2]) -> Vec2 {
    Vec2::new(a[0], a[1])
}

// ============================================================================
// GRID BUILDING SAVE COMPAT
// ============================================================================

/// Legacy Building enum format from old saves. Deserializes old `{"Farm": {"town_idx": 0}}` format
/// and converts to `(BuildingKind, u32)` tuples.
#[derive(Deserialize)]
#[allow(dead_code)]
enum LegacyBuilding {
    Fountain {
        town_idx: u32,
    },
    Farm {
        town_idx: u32,
    },
    Bed {
        town_idx: u32,
    },
    #[serde(alias = "GuardPost")]
    Waypoint {
        town_idx: u32,
        #[serde(default)]
        patrol_order: u32,
    },
    Camp {
        town_idx: u32,
    },
    GoldMine,
    MinerHome {
        town_idx: u32,
    },
    FarmerHome {
        town_idx: u32,
    },
    ArcherHome {
        town_idx: u32,
    },
    CrossbowHome {
        town_idx: u32,
    },
    FighterHome {
        town_idx: u32,
    },
    Tent {
        town_idx: u32,
    },
    Home {
        kind: world::BuildingKind,
        town_idx: u32,
    },
}

impl LegacyBuilding {
    fn to_grid_building(self) -> (world::BuildingKind, u32) {
        use world::BuildingKind::*;
        match self {
            Self::Fountain { town_idx } => (Fountain, town_idx),
            Self::Farm { town_idx } => (Farm, town_idx),
            Self::Bed { town_idx } => (Bed, town_idx),
            Self::Waypoint { town_idx, .. } => (Waypoint, town_idx),
            Self::Camp { town_idx } => (Fountain, town_idx),
            Self::GoldMine => (GoldMine, 0),
            Self::MinerHome { town_idx } => (MinerHome, town_idx),
            Self::FarmerHome { town_idx } => (FarmerHome, town_idx),
            Self::ArcherHome { town_idx } => (ArcherHome, town_idx),
            Self::CrossbowHome { town_idx } => (CrossbowHome, town_idx),
            Self::FighterHome { town_idx } => (FighterHome, town_idx),
            Self::Tent { town_idx } => (Tent, town_idx),
            Self::Home { kind, town_idx } => (kind, town_idx),
        }
    }
}

/// Deserialize grid buildings: accepts both new tuple format and legacy enum format.
fn deserialize_grid_buildings<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<Option<(world::BuildingKind, u32)>>, D::Error> {
    // Try new format first (Vec<Option<(BuildingKind, u32)>>), fall back to legacy
    let raw: Vec<Option<serde_json::Value>> = Deserialize::deserialize(deserializer)?;
    Ok(raw
        .into_iter()
        .map(|opt| {
            opt.and_then(|v| {
                // Try new tuple format: [kind, town_idx]
                if let Ok(tuple) = serde_json::from_value::<(world::BuildingKind, u32)>(v.clone()) {
                    return Some(tuple);
                }
                // Fall back to legacy enum format: {"Farm": {"town_idx": 0}}
                serde_json::from_value::<LegacyBuilding>(v)
                    .ok()
                    .map(|lb| lb.to_grid_building())
            })
        })
        .collect())
}

// ============================================================================
// SAVE FORMAT STRUCTS
// ============================================================================

// Save format changelog:
// v1: initial format
// v2: farm_growth contains only farm entries (mines moved to mine_growth)
const SAVE_VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
pub struct SaveData {
    pub version: u32,

    // World grid
    pub grid_width: usize,
    pub grid_height: usize,
    pub grid_cell_size: f32,
    pub terrain: Vec<u8>, // Biome as u8
    #[serde(deserialize_with = "deserialize_grid_buildings")]
    pub buildings: Vec<Option<(world::BuildingKind, u32)>>, // parallel to terrain

    // Town grids (area_level + town_data_idx)
    pub town_grids: Vec<TownGridSave>,

    // Time + economy
    pub total_seconds: f32,
    pub seconds_per_hour: f32,
    pub time_scale: f32,
    #[serde(default)]
    pub food: Vec<i32>,
    #[serde(default)]
    pub gold: Vec<i32>,

    // Farm states
    pub farm_growth: Vec<FarmGrowthSave>,

    // Mine growth states
    #[serde(default)]
    pub mine_growth: Vec<FarmGrowthSave>,

    // Spawners
    pub spawners: Vec<SpawnerSave>,

    // Building HP (keyed by registry save_key + "towns" for fountains)
    #[serde(default)]
    pub building_hp: std::collections::HashMap<String, Vec<f32>>,

    // Upgrades + policies
    pub upgrades: Vec<Vec<u8>>,
    #[serde(default)]
    pub policies: Vec<PolicySet>,
    #[serde(default)]
    pub auto_upgrades: Vec<Vec<bool>>,

    // Squads
    pub squads: Vec<SquadSave>,

    // Legacy waypoint turret state (no longer used, kept for backward compat)
    #[serde(default, alias = "guard_post_attack")]
    pub waypoint_attack: Vec<bool>,

    // Raider state
    #[serde(alias = "camp_respawn_timers")]
    pub raider_respawn_timers: Vec<f32>,
    #[serde(alias = "camp_forage_timers")]
    pub raider_forage_timers: Vec<f32>,
    #[serde(alias = "camp_max_pop")]
    pub raider_max_pop: Vec<i32>,

    // Faction stats
    pub faction_stats: Vec<FactionStatSave>,
    pub kill_stats: [i32; 2], // [archer_kills, villager_kills]

    // NPCs
    pub npcs: Vec<NpcSaveData>,

    // AI players
    pub ai_players: Vec<AiPlayerSave>,

    // Migration state
    #[serde(default)]
    pub migration: Option<MigrationSave>,

    // UID counter
    #[serde(default)]
    pub next_entity_uid: Option<u64>,

    // Endless mode
    #[serde(default)]
    pub endless_mode: bool,
    #[serde(default = "default_endless_strength")]
    pub endless_strength: f32,
    #[serde(default)]
    pub endless_pending: Vec<PendingAiSpawn>,

    // Building vecs + towns — registry-driven via #[serde(flatten)]
    // Captures: towns, farms, beds, waypoints, farmer_homes, archer_homes,
    // crossbow_homes, fighter_homes, tents, miner_homes, gold_mines
    #[serde(flatten)]
    pub building_data: std::collections::HashMap<String, serde_json::Value>,
}

// Sub-structs

#[derive(Serialize, Deserialize, Clone)]
pub struct TownGridSave {
    pub town_data_idx: usize,
    pub area_level: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FarmGrowthSave {
    pub state: u8, // 0=Growing, 1=Ready
    pub progress: f32,
    #[serde(default)]
    pub under_construction: f32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SpawnerSave {
    pub building_kind: i32,
    pub town_idx: i32,
    pub position: [f32; 2],
    #[serde(alias = "npc_slot")]
    pub npc_gpu_slot: i32, // Legacy: kept for old save compat, derived from npc_uid
    pub respawn_timer: f32,
    #[serde(default)]
    pub npc_uid: Option<u64>,
    #[serde(default)]
    pub under_construction: f32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SquadSave {
    pub members: Vec<usize>, // Legacy: kept for old save compat, derived from member_uids
    pub target: Option<[f32; 2]>,
    pub target_size: usize,
    pub patrol_enabled: bool,
    pub rest_when_tired: bool,
    #[serde(default)]
    pub wave_active: bool,
    #[serde(default)]
    pub wave_start_count: usize,
    #[serde(default)]
    pub wave_min_start: usize,
    #[serde(default = "default_wave_retreat_below_pct")]
    pub wave_retreat_below_pct: usize,
    #[serde(default)]
    pub owner: SquadOwner,
    #[serde(default)]
    pub member_uids: Option<Vec<u64>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FactionStatSave {
    pub alive: i32,
    pub dead: i32,
    pub kills: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct AiPlayerSave {
    pub town_data_idx: usize,
    pub grid_idx: usize,
    pub kind: u8,        // 0=Raider, 1=Builder
    pub personality: u8, // 0=Aggressive, 1=Balanced, 2=Economic
    #[serde(default = "default_road_style")]
    pub road_style: u8, // 0=None, 1=Cardinal, 2=Grid4, 3=Grid5
    #[serde(default = "default_true")]
    pub active: bool,
}

fn default_road_style() -> u8 {
    2
} // Grid4 for old saves

#[derive(Serialize, Deserialize, Clone)]
pub struct MigrationSave {
    #[serde(default)]
    pub town_data_idx: Option<usize>,
    #[serde(default)]
    pub grid_idx: usize,
    pub member_slots: Vec<usize>,
    pub check_timer: f32,
    #[serde(default)]
    pub is_raider: bool,
    #[serde(default)]
    pub faction: i32,
    #[serde(default)]
    pub upgrade_levels: Vec<u8>,
    #[serde(default)]
    pub starting_food: i32,
    #[serde(default)]
    pub starting_gold: i32,
}

fn default_true() -> bool {
    true
}
fn default_wave_retreat_below_pct() -> usize {
    50
}
fn default_endless_strength() -> f32 {
    0.75
}

// Building save (mirrors world::Building)

// ============================================================================
// NPC SAVE DATA
// ============================================================================

#[derive(Serialize, Deserialize, Clone)]
pub struct NpcSaveData {
    pub slot: usize,
    pub position: [f32; 2],
    pub job: u8,
    pub faction: i32,
    pub town_id: i32,
    pub health: f32,
    pub energy: f32,
    #[serde(default)]
    pub uid: Option<u64>,
    pub activity: ActivitySave,
    pub combat_state: CombatStateSave,
    pub personality: PersonalitySave,
    pub name: String,
    pub level: i32,
    pub xp: i32,
    pub attack_type: u8, // 0=Melee, 1=Ranged
    pub home: [f32; 2],
    pub work_position: Option<[f32; 2]>,
    pub squad_id: Option<i32>,
    pub carried_gold: Option<i32>,
    pub weapon: Option<[f32; 2]>,
    pub helmet: Option<[f32; 2]>,
    pub armor: Option<[f32; 2]>,
}

#[derive(Serialize, Deserialize, Clone)]
pub enum ActivitySave {
    Idle,
    Working,
    OnDuty { ticks_waiting: u32 },
    Patrolling,
    GoingToWork,
    GoingToRest,
    Resting,
    GoingToHeal,
    HealingAtFountain { recover_until: f32 },
    Wandering,
    Raiding { target: [f32; 2] },
    Returning { loot: Vec<(ItemKind, i32)> },
    Mining { mine_pos: [f32; 2] },
    MiningAtMine,
}

impl ActivitySave {
    fn from_activity(a: &Activity) -> Self {
        match a {
            Activity::Idle => Self::Idle,
            Activity::Working => Self::Working,
            Activity::OnDuty { ticks_waiting } => Self::OnDuty {
                ticks_waiting: *ticks_waiting,
            },
            Activity::Patrolling => Self::Patrolling,
            Activity::GoingToWork => Self::GoingToWork,
            Activity::GoingToRest => Self::GoingToRest,
            Activity::Resting => Self::Resting,
            Activity::GoingToHeal => Self::GoingToHeal,
            Activity::HealingAtFountain { recover_until } => Self::HealingAtFountain {
                recover_until: *recover_until,
            },
            Activity::Wandering => Self::Wandering,
            Activity::Raiding { target } => Self::Raiding {
                target: v2(*target),
            },
            Activity::Returning { loot } => Self::Returning { loot: loot.clone() },
            Activity::Mining { mine_pos } => Self::Mining {
                mine_pos: v2(*mine_pos),
            },
            Activity::MiningAtMine => Self::MiningAtMine,
        }
    }

    fn to_activity(&self) -> Activity {
        match self {
            Self::Idle => Activity::Idle,
            Self::Working => Activity::Working,
            Self::OnDuty { ticks_waiting } => Activity::OnDuty {
                ticks_waiting: *ticks_waiting,
            },
            Self::Patrolling => Activity::Patrolling,
            Self::GoingToWork => Activity::GoingToWork,
            Self::GoingToRest => Activity::GoingToRest,
            Self::Resting => Activity::Resting,
            Self::GoingToHeal => Activity::GoingToHeal,
            Self::HealingAtFountain { recover_until } => Activity::HealingAtFountain {
                recover_until: *recover_until,
            },
            Self::Wandering => Activity::Wandering,
            Self::Raiding { target } => Activity::Raiding {
                target: to_vec2(*target),
            },
            Self::Returning { loot } => Activity::Returning { loot: loot.clone() },
            Self::Mining { mine_pos } => Activity::Mining {
                mine_pos: to_vec2(*mine_pos),
            },
            Self::MiningAtMine => Activity::MiningAtMine,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub enum CombatStateSave {
    None,
    Fighting { origin: [f32; 2] },
    Fleeing,
}

impl CombatStateSave {
    fn from_combat_state(cs: &CombatState) -> Self {
        match cs {
            CombatState::None => Self::None,
            CombatState::Fighting { origin } => Self::Fighting {
                origin: v2(*origin),
            },
            CombatState::Fleeing => Self::Fleeing,
        }
    }

    fn to_combat_state(&self) -> CombatState {
        match self {
            Self::None => CombatState::None,
            Self::Fighting { origin } => CombatState::Fighting {
                origin: to_vec2(*origin),
            },
            Self::Fleeing => CombatState::Fleeing,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TraitSave {
    pub kind: u8, // 0=Brave, 1=Tough, 2=Swift, 3=Focused
    pub magnitude: f32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct PersonalitySave {
    pub trait1: Option<TraitSave>,
    pub trait2: Option<TraitSave>,
}

impl PersonalitySave {
    fn from_personality(p: &Personality) -> Self {
        let map_trait = |t: &TraitInstance| -> TraitSave {
            let kind = t.kind.to_id();
            TraitSave {
                kind: kind as u8,
                magnitude: t.magnitude,
            }
        };
        Self {
            trait1: p.trait1.as_ref().map(map_trait),
            trait2: p.trait2.as_ref().map(map_trait),
        }
    }

    fn to_personality(&self) -> Personality {
        let map_trait = |t: &TraitSave| -> TraitInstance {
            let kind = TraitKind::from_id(t.kind as i32).unwrap_or(TraitKind::Focused);
            TraitInstance {
                kind,
                magnitude: t.magnitude,
            }
        };
        Personality {
            trait1: self.trait1.as_ref().map(map_trait),
            trait2: self.trait2.as_ref().map(map_trait),
        }
    }
}

// ============================================================================
// BIOME ENCODING
// ============================================================================

fn biome_to_u8(b: world::Biome) -> u8 {
    match b {
        world::Biome::Grass => 0,
        world::Biome::Forest => 1,
        world::Biome::Water => 2,
        world::Biome::Rock => 3,
        world::Biome::Dirt => 4,
    }
}

fn u8_to_biome(v: u8) -> world::Biome {
    match v {
        1 => world::Biome::Forest,
        2 => world::Biome::Water,
        3 => world::Biome::Rock,
        4 => world::Biome::Dirt,
        _ => world::Biome::Grass,
    }
}

// ============================================================================
// SAVE PATH
// ============================================================================

fn save_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .ok()?;
    let dir = std::path::PathBuf::from(home)
        .join("Documents")
        .join("Endless")
        .join("saves");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn quicksave_path() -> Option<std::path::PathBuf> {
    save_dir().map(|d| d.join("quicksave.json"))
}

// ============================================================================
// SAVE FUNCTION
// ============================================================================

/// Collect full game state into SaveData. NPC data passed as pre-collected vec.
pub fn collect_save_data(
    grid: &WorldGrid,
    world_data: &WorldData,
    entity_map: &EntityMap,
    town_grids: &TownGrids,
    game_time: &GameTime,
    food_storage: &FoodStorage,
    gold_storage: &GoldStorage,
    building_hp: std::collections::HashMap<String, Vec<f32>>,
    upgrades: &TownUpgrades,
    policies: &TownPolicies,
    auto_upgrade: &AutoUpgrade,
    squad_state: &SquadState,
    raider_state: &RaiderState,
    faction_stats: &FactionStats,
    kill_stats: &KillStats,
    ai_state: &AiPlayerState,
    migration_state: &MigrationState,
    endless: &EndlessMode,
    npcs: Vec<NpcSaveData>,
    uid_alloc: &NextEntityUid,
) -> SaveData {
    // Terrain + buildings
    let terrain: Vec<u8> = grid.cells.iter().map(|c| biome_to_u8(c.terrain)).collect();
    let mut buildings: Vec<Option<(world::BuildingKind, u32)>> = vec![None; grid.cells.len()];
    for inst in entity_map.iter_instances() {
        let (gc, gr) = grid.world_to_grid(inst.position);
        let idx = gr * grid.width + gc;
        if idx < buildings.len() {
            buildings[idx] = Some((inst.kind, inst.town_idx));
        }
    }

    // Building vecs — serialized from EntityMap instances
    let mut building_data: std::collections::HashMap<String, serde_json::Value> =
        std::collections::HashMap::new();
    building_data.insert(
        "towns".to_string(),
        serde_json::to_value(&world_data.towns).unwrap(),
    );
    for def in crate::constants::BUILDING_REGISTRY.iter() {
        let Some(key) = def.save_key else { continue };
        let mut insts: Vec<_> = entity_map.iter_kind(def.kind).collect();
        insts.sort_by_key(|i| i.slot);
        let placed: Vec<world::PlacedBuilding> = insts
            .iter()
            .map(|inst| world::PlacedBuilding {
                position: inst.position,
                town_idx: inst.town_idx,
                patrol_order: inst.patrol_order,
                assigned_mine: inst.assigned_mine,
                manual_mine: inst.manual_mine,
                wall_level: inst.wall_level,
            })
            .collect();
        building_data.insert(key.to_string(), serde_json::to_value(&placed).unwrap());
    }

    // Town grids
    let town_grids_save: Vec<TownGridSave> = town_grids
        .grids
        .iter()
        .map(|g| TownGridSave {
            town_data_idx: g.town_data_idx,
            area_level: g.area_level,
        })
        .collect();

    // Farm growth (serialized from BuildingInstance growth fields)
    let farm_growth: Vec<FarmGrowthSave> = entity_map
        .iter_kind(crate::world::BuildingKind::Farm)
        .map(|i| FarmGrowthSave {
            state: if i.growth_ready { 1 } else { 0 },
            progress: i.growth_progress,
            under_construction: i.under_construction,
        })
        .collect();

    // Spawners (serialized from EntityMap spawner instances)
    let spawners: Vec<SpawnerSave> = entity_map
        .iter_instances()
        .filter(|i| crate::constants::building_def(i.kind).spawner.is_some())
        .map(|i| SpawnerSave {
            building_kind: crate::constants::tileset_index(i.kind) as i32,
            town_idx: i.town_idx as i32,
            position: v2(i.position),
            npc_gpu_slot: i
                .npc_uid
                .and_then(|uid| entity_map.slot_for_uid(uid))
                .map(|s| s as i32)
                .unwrap_or(-1),
            respawn_timer: i.respawn_timer,
            npc_uid: i.npc_uid.map(|uid| uid.0),
            under_construction: i.under_construction,
        })
        .collect();

    let building_hp_save = building_hp;

    // Upgrades (already Vec<Vec<u8>>)
    let upgrades_save: Vec<Vec<u8>> = upgrades.levels.clone();

    // Auto-upgrades (already Vec<Vec<bool>>)
    let auto_upgrades_save: Vec<Vec<bool>> = auto_upgrade.flags.clone();

    // Squads
    let squads: Vec<SquadSave> = squad_state
        .squads
        .iter()
        .map(|s| SquadSave {
            members: s
                .members
                .iter()
                .filter_map(|uid| entity_map.slot_for_uid(*uid))
                .collect(),
            target: s.target.map(v2),
            target_size: s.target_size,
            patrol_enabled: s.patrol_enabled,
            rest_when_tired: s.rest_when_tired,
            wave_active: s.wave_active,
            wave_start_count: s.wave_start_count,
            wave_min_start: s.wave_min_start,
            wave_retreat_below_pct: s.wave_retreat_below_pct,
            owner: s.owner,
            member_uids: Some(s.members.iter().map(|uid| uid.0).collect()),
        })
        .collect();

    // Faction stats
    let faction_stats_save: Vec<FactionStatSave> = faction_stats
        .stats
        .iter()
        .map(|s| FactionStatSave {
            alive: s.alive,
            dead: s.dead,
            kills: s.kills,
        })
        .collect();

    // AI players
    let ai_players: Vec<AiPlayerSave> = ai_state
        .players
        .iter()
        .map(|p| {
            use crate::systems::ai_player::*;
            AiPlayerSave {
                town_data_idx: p.town_data_idx,
                grid_idx: p.grid_idx,
                kind: match p.kind {
                    AiKind::Raider => 0,
                    AiKind::Builder => 1,
                },
                personality: match p.personality {
                    AiPersonality::Aggressive => 0,
                    AiPersonality::Balanced => 1,
                    AiPersonality::Economic => 2,
                },
                road_style: match p.road_style {
                    RoadStyle::None => 0,
                    RoadStyle::Cardinal => 1,
                    RoadStyle::Grid4 => 2,
                    RoadStyle::Grid5 => 3,
                },
                active: p.active,
            }
        })
        .collect();

    SaveData {
        version: SAVE_VERSION,
        grid_width: grid.width,
        grid_height: grid.height,
        grid_cell_size: grid.cell_size,
        terrain,
        buildings,
        building_data,
        town_grids: town_grids_save,
        total_seconds: game_time.total_seconds,
        seconds_per_hour: game_time.seconds_per_hour,
        time_scale: game_time.time_scale,
        food: food_storage.food.clone(),
        gold: gold_storage.gold.clone(),
        farm_growth,
        mine_growth: entity_map
            .iter_kind(crate::world::BuildingKind::GoldMine)
            .map(|i| FarmGrowthSave {
                state: if i.growth_ready { 1 } else { 0 },
                progress: i.growth_progress,
                under_construction: i.under_construction,
            })
            .collect(),
        spawners,
        building_hp: building_hp_save,
        upgrades: upgrades_save,
        policies: policies.policies.clone(),
        auto_upgrades: auto_upgrades_save,
        squads,
        waypoint_attack: vec![],
        raider_respawn_timers: raider_state.respawn_timers.clone(),
        raider_forage_timers: raider_state.forage_timers.clone(),
        raider_max_pop: raider_state.max_pop.clone(),
        faction_stats: faction_stats_save,
        kill_stats: [kill_stats.archer_kills, kill_stats.villager_kills],
        npcs,
        ai_players,
        migration: migration_state
            .active
            .as_ref()
            .filter(|g| g.boat_slot.is_none()) // don't save boat phase — transient
            .map(|g| MigrationSave {
                town_data_idx: g.town_data_idx,
                grid_idx: g.grid_idx,
                member_slots: g.member_slots.clone(),
                check_timer: migration_state.check_timer,
                is_raider: g.is_raider,
                faction: g.faction,
                upgrade_levels: g.upgrade_levels.clone(),
                starting_food: g.starting_food,
                starting_gold: g.starting_gold,
            }),
        next_entity_uid: Some(uid_alloc.0),
        endless_mode: endless.enabled,
        endless_strength: endless.strength_fraction,
        endless_pending: endless.pending_spawns.clone(),
    }
}

/// Write SaveData to the quicksave file.
pub fn write_save(data: &SaveData) -> Result<(), String> {
    let path = quicksave_path().ok_or("cannot determine save directory")?;
    write_save_to(data, &path)
}

/// Write save data to a specific path.
pub fn write_save_to(data: &SaveData, path: &std::path::Path) -> Result<(), String> {
    let json = serde_json::to_string(data).map_err(|e| format!("serialize: {e}"))?;
    std::fs::write(path, json).map_err(|e| format!("write {}: {e}", path.display()))?;
    info!("Game saved to {}", path.display());
    Ok(())
}

/// Return the path for a rotating autosave slot (0, 1, 2).
fn autosave_path(slot: u8) -> Option<std::path::PathBuf> {
    save_dir().map(|d| d.join(format!("autosave_{}.json", slot + 1)))
}

/// Info about a save file on disk.
pub struct SaveFileInfo {
    pub filename: String,
    pub path: std::path::PathBuf,
    pub modified: std::time::SystemTime,
}

/// List all save files in the save directory, sorted newest first.
pub fn list_saves() -> Vec<SaveFileInfo> {
    let Some(dir) = save_dir() else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut saves: Vec<SaveFileInfo> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            Some(SaveFileInfo {
                filename: e.file_name().to_string_lossy().into_owned(),
                path: e.path(),
                modified: meta.modified().ok()?,
            })
        })
        .collect();
    saves.sort_by(|a, b| b.modified.cmp(&a.modified));
    saves
}

/// Read SaveData from an arbitrary path.
pub fn read_save_from(path: &std::path::Path) -> Result<SaveData, String> {
    let json =
        std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let data: SaveData = serde_json::from_str(&json).map_err(|e| format!("deserialize: {e}"))?;
    if data.version > SAVE_VERSION {
        return Err(format!(
            "save version {} > supported {}",
            data.version, SAVE_VERSION
        ));
    }
    if data.version < SAVE_VERSION {
        info!("Migrating save from v{} to v{}", data.version, SAVE_VERSION);
    }
    Ok(data)
}

/// Read SaveData from the quicksave file.
pub fn read_save() -> Result<SaveData, String> {
    let path = quicksave_path().ok_or("cannot determine save directory")?;
    read_save_from(&path)
}

// ============================================================================
// APPLY SAVE (restore game state from SaveData)
// ============================================================================

/// Apply save data to all game resources. Call after despawning all NPC entities.
pub fn apply_save(
    save: &SaveData,
    grid: &mut WorldGrid,
    world_data: &mut WorldData,
    town_grids: &mut TownGrids,
    game_time: &mut GameTime,
    food_storage: &mut FoodStorage,
    gold_storage: &mut GoldStorage,
    upgrades: &mut TownUpgrades,
    policies: &mut TownPolicies,
    auto_upgrade: &mut AutoUpgrade,
    squad_state: &mut SquadState,
    raider_state: &mut RaiderState,
    faction_stats: &mut FactionStats,
    kill_stats: &mut KillStats,
    ai_state: &mut AiPlayerState,
    migration_state: &mut MigrationState,
    endless: &mut EndlessMode,
    npcs_by_town: &mut NpcsByTownCache,
    slots: &mut GpuSlotPool,
) {
    info!("Applying save version {}", save.version);

    // World grid
    grid.width = save.grid_width;
    grid.height = save.grid_height;
    grid.cell_size = save.grid_cell_size;
    grid.cells = save
        .terrain
        .iter()
        .map(|&t| WorldCell {
            terrain: u8_to_biome(t),
        })
        .collect();

    // Towns
    if let Some(val) = save.building_data.get("towns") {
        world_data.towns = serde_json::from_value(val.clone()).unwrap_or_default();
    }

    // Town grids
    town_grids.grids = save
        .town_grids
        .iter()
        .map(|g| {
            let mut tg = world::TownGrid::new_base(g.town_data_idx);
            tg.area_level = g.area_level;
            tg
        })
        .collect();
    world::sync_town_grid_world_caps(grid, &world_data.towns, town_grids);

    // Game time
    game_time.total_seconds = save.total_seconds;
    game_time.seconds_per_hour = save.seconds_per_hour;
    game_time.time_scale = save.time_scale.max(0.0);
    game_time.start_hour = 6;
    game_time.last_hour = game_time.total_hours();
    game_time.hour_ticked = false;
    game_time.paused = game_time.time_scale <= 0.0;

    // Economy
    food_storage.food = save.food.clone();
    gold_storage.gold = save.gold.clone();

    // Growth states + spawner state are rebuilt in load_building_instances_from_save

    // Upgrades
    upgrades.levels = save
        .upgrades
        .iter()
        .map(|v| decode_upgrade_levels(v))
        .collect();

    // Policies
    let num_towns = world_data.towns.len();
    policies.policies = save.policies.clone();
    policies
        .policies
        .resize(num_towns.max(16), PolicySet::default());

    // Auto-upgrades
    auto_upgrade.flags = save
        .auto_upgrades
        .iter()
        .map(|v| decode_auto_upgrade_flags(v))
        .collect();
    auto_upgrade.ensure_towns(num_towns.max(16));

    // Squads — load all saved squads (player + AI).
    // First MAX_SQUADS are player-reserved; extras are AI squads.
    squad_state.squads.clear();
    for ss in save.squads.iter() {
        let members = ss
            .member_uids
            .as_ref()
            .map(|uids| {
                uids.iter()
                    .map(|&u| crate::components::EntityUid(u))
                    .collect()
            })
            .unwrap_or_default(); // old saves: empty until post-spawn fixup
        squad_state.squads.push(Squad {
            members,
            target: ss.target.map(to_vec2),
            target_size: ss.target_size,
            patrol_enabled: ss.patrol_enabled,
            rest_when_tired: ss.rest_when_tired,
            wave_active: ss.wave_active,
            wave_start_count: ss.wave_start_count,
            wave_min_start: ss.wave_min_start,
            wave_retreat_below_pct: ss.wave_retreat_below_pct.clamp(1, 100),
            owner: ss.owner,
            hold_fire: false,
        });
    }
    // Ensure at least MAX_SQUADS player squads exist.
    while squad_state.squads.len() < MAX_SQUADS {
        squad_state.squads.push(Squad::default());
    }
    squad_state.selected = 0;
    squad_state.placing_target = false;

    // Raider state
    raider_state.max_pop = save.raider_max_pop.clone();
    raider_state.respawn_timers = save.raider_respawn_timers.clone();
    raider_state.forage_timers = save.raider_forage_timers.clone();

    // Faction stats
    faction_stats.stats = save
        .faction_stats
        .iter()
        .map(|s| FactionStat {
            alive: s.alive,
            dead: s.dead,
            kills: s.kills,
        })
        .collect();

    // Kill stats
    kill_stats.archer_kills = save.kill_stats[0];
    kill_stats.villager_kills = save.kill_stats[1];

    // AI players
    {
        use crate::systems::ai_player::*;
        use std::collections::VecDeque;
        ai_state.players = save
            .ai_players
            .iter()
            .map(|p| AiPlayer {
                town_data_idx: p.town_data_idx,
                grid_idx: p.grid_idx,
                kind: if p.kind == 0 {
                    AiKind::Raider
                } else {
                    AiKind::Builder
                },
                personality: match p.personality {
                    0 => AiPersonality::Aggressive,
                    2 => AiPersonality::Economic,
                    _ => AiPersonality::Balanced,
                },
                road_style: match p.road_style {
                    0 => RoadStyle::None,
                    1 => RoadStyle::Cardinal,
                    3 => RoadStyle::Grid5,
                    _ => RoadStyle::Grid4,
                },
                last_actions: VecDeque::new(),
                active: p.active,
                squad_indices: Vec::new(),
                squad_cmd: std::collections::HashMap::new(),
            })
            .collect();
        // Rebuild AI squad indices by scanning SquadState ownership (authoritative).
        for player in ai_state.players.iter_mut() {
            rebuild_squad_indices(player, &squad_state.squads);
        }
    }

    // Migration state (boat phase is not saved — only walk/settle phase)
    if let Some(ms) = &save.migration {
        migration_state.active = Some(MigrationGroup {
            boat_slot: None,
            boat_pos: Vec2::ZERO,
            settle_target: Vec2::ZERO,
            is_raider: ms.is_raider,
            upgrade_levels: ms.upgrade_levels.clone(),
            starting_food: ms.starting_food,
            starting_gold: ms.starting_gold,
            member_slots: ms.member_slots.clone(),
            faction: ms.faction,
            town_data_idx: ms.town_data_idx,
            grid_idx: ms.grid_idx,
        });
        migration_state.check_timer = ms.check_timer;
    } else {
        *migration_state = MigrationState::default();
    }

    // Endless mode
    endless.enabled = true; // always enabled
    endless.strength_fraction = save.endless_strength;
    endless.pending_spawns = save.endless_pending.clone();

    // NPC tracking
    npcs_by_town.0 = vec![Vec::new(); num_towns];

    // Slot allocator: rebuild from saved NPC slots
    slots.reset();
    let mut max_slot = 0usize;
    let mut used_slots = std::collections::HashSet::new();
    for npc in &save.npcs {
        used_slots.insert(npc.slot);
        max_slot = max_slot.max(npc.slot + 1);
    }
    slots.set_next(max_slot);
    // Free list = all slots below max_slot that aren't used
    for i in 0..max_slot {
        if !used_slots.contains(&i) {
            slots.free_list_mut().push(i);
        }
    }
}

// ============================================================================
// SAVE/LOAD TRIGGER RESOURCE
// ============================================================================

/// Request an immediate save (quicksave or configured path).
#[derive(Message, Clone, Copy)]
pub struct SaveGameMsg;

/// Request an immediate load (quicksave or configured path).
#[derive(Message, Clone, Copy)]
pub struct LoadGameMsg;

/// Trigger resource for save/load operations.
#[derive(Resource, Default)]
pub struct SaveLoadRequest {
    /// Set by main menu "Load Game" — tells game_startup_system to load instead of world gen.
    pub load_on_enter: bool,
    /// When set, save to this path instead of quicksave.
    pub save_path: Option<std::path::PathBuf>,
    /// When set, load from this path instead of quicksave.
    pub load_path: Option<std::path::PathBuf>,
    /// Autosave interval in game-hours (0 = disabled). Set from settings on game start.
    pub autosave_hours: i32,
    /// Last game-hour when autosave triggered (to detect interval crossing).
    pub autosave_last_hour: i32,
    /// Rotating slot index (0, 1, 2) for the next autosave.
    pub autosave_slot: u8,
}

/// Check if a quicksave file exists.
pub fn has_quicksave() -> bool {
    quicksave_path().map(|p| p.exists()).unwrap_or(false)
}

/// Build a named save path in Documents/Endless/saves.
pub fn named_save_path(name: &str) -> Option<std::path::PathBuf> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return None;
    }
    let safe: String = trimmed
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                ch
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        return None;
    }
    save_dir().map(|d| d.join(format!("{safe}.json")))
}

/// Toast notification state for save/load feedback.
#[derive(Resource, Default)]
pub struct SaveToast {
    pub message: String,
    pub timer: f32,
}

// ============================================================================
// NPC QUERY — uses nested tuples to stay under Bevy's 16-element limit
// ============================================================================

/// Collect NPC save data from EntityMap NpcInstances + ECS queries.
pub fn collect_npc_data(
    entity_map: &EntityMap,
    npc_meta: &NpcMetaCache,
    squad_id_q: &Query<&SquadId>,
    activity_q: &Query<&Activity>,
    position_q: &Query<&Position>,
    health_q: &Query<&Health, Without<Building>>,
    energy_q: &Query<&Energy>,
    combat_state_q: &Query<&CombatState>,
    attack_type_q: &Query<&BaseAttackType>,
    personality_q: &Query<&Personality>,
    home_q: &Query<&Home>,
    work_state_q: &Query<&NpcWorkState>,
    carried_gold_q: &Query<&CarriedGold>,
    weapon_q: &Query<&EquippedWeapon>,
    helmet_q: &Query<&EquippedHelmet>,
    armor_q: &Query<&EquippedArmor>,
    has_energy_q: &Query<&HasEnergy>,
) -> Vec<NpcSaveData> {
    let mut npcs = Vec::new();
    for npc in entity_map.iter_npcs() {
        if npc.dead {
            continue;
        }
        let idx = npc.slot;
        let meta = &npc_meta.0[idx];

        npcs.push(NpcSaveData {
            slot: idx,
            position: position_q
                .get(npc.entity)
                .map(|p| [p.x, p.y])
                .unwrap_or([0.0, 0.0]),
            job: match npc.job {
                Job::Farmer => 0,
                Job::Archer => 1,
                Job::Raider => 2,
                Job::Fighter => 3,
                Job::Miner => 4,
                Job::Crossbow => 5,
            },
            faction: npc.faction,
            town_id: npc.town_idx,
            health: health_q.get(npc.entity).map(|h| h.0).unwrap_or(100.0),
            uid: entity_map.uid_for_slot(idx).map(|u| u.0),
            energy: if has_energy_q.get(npc.entity).is_ok() {
                energy_q.get(npc.entity).map(|e| e.0).unwrap_or(100.0)
            } else {
                100.0
            },
            activity: activity_q
                .get(npc.entity)
                .map(|a| ActivitySave::from_activity(a))
                .unwrap_or(ActivitySave::Idle),
            combat_state: combat_state_q
                .get(npc.entity)
                .map(|cs| CombatStateSave::from_combat_state(cs))
                .unwrap_or(CombatStateSave::None),
            personality: personality_q
                .get(npc.entity)
                .map(|p| PersonalitySave::from_personality(p))
                .unwrap_or_default(),
            name: meta.name.clone(),
            level: meta.level,
            xp: meta.xp,
            attack_type: match attack_type_q
                .get(npc.entity)
                .copied()
                .unwrap_or(BaseAttackType::Melee)
            {
                BaseAttackType::Melee => 0,
                BaseAttackType::Ranged => 1,
            },
            home: home_q
                .get(npc.entity)
                .map(|h| [h.0.x, h.0.y])
                .unwrap_or([0.0, 0.0]),
            work_position: work_state_q
                .get(npc.entity)
                .ok()
                .and_then(|ws| ws.work_target_building)
                .and_then(|uid| entity_map.instance_by_uid(uid).map(|i| v2(i.position))),
            squad_id: squad_id_q.get(npc.entity).ok().map(|s| s.0),
            carried_gold: carried_gold_q
                .get(npc.entity)
                .ok()
                .and_then(|g| if g.0 > 0 { Some(g.0) } else { None }),
            weapon: weapon_q.get(npc.entity).ok().map(|w| [w.0, w.1]),
            helmet: helmet_q.get(npc.entity).ok().map(|h| [h.0, h.1]),
            armor: armor_q.get(npc.entity).ok().map(|a| [a.0, a.1]),
        });
    }
    npcs
}

// ============================================================================
// SYSTEM PARAM BUNDLES (keep system params under Bevy's 16-element limit)
// ============================================================================

use bevy::ecs::system::SystemParam;

/// World state resources for save/load.
#[derive(SystemParam)]
pub struct SaveWorldState<'w> {
    pub grid: ResMut<'w, WorldGrid>,
    pub world_data: ResMut<'w, WorldData>,
    pub town_grids: ResMut<'w, TownGrids>,
    pub game_time: ResMut<'w, GameTime>,
    pub food_storage: ResMut<'w, FoodStorage>,
    pub gold_storage: ResMut<'w, GoldStorage>,
    pub upgrades: ResMut<'w, TownUpgrades>,
    pub policies: ResMut<'w, TownPolicies>,
    pub auto_upgrade: ResMut<'w, AutoUpgrade>,
    pub squad_state: ResMut<'w, SquadState>,
    pub tower_state: ResMut<'w, TowerState>,
}

/// More world state + faction/AI resources.
#[derive(SystemParam)]
pub struct SaveFactionState<'w> {
    pub raider_state: ResMut<'w, RaiderState>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub kill_stats: ResMut<'w, KillStats>,
    pub ai_state: ResMut<'w, AiPlayerState>,
    pub migration_state: ResMut<'w, MigrationState>,
    pub endless: ResMut<'w, EndlessMode>,
}

/// NPC queries for save (collect_npc_data).
#[derive(SystemParam)]
pub struct SaveNpcQueries<'w, 's> {
    pub squad_id_q: Query<'w, 's, &'static SquadId>,
    pub activity_q: Query<'w, 's, &'static Activity>,
    pub position_q: Query<'w, 's, &'static Position>,
    pub health_q: Query<'w, 's, &'static Health, Without<Building>>,
    pub energy_q: Query<'w, 's, &'static Energy>,
    pub combat_state_q: Query<'w, 's, &'static CombatState>,
    pub attack_type_q: Query<'w, 's, &'static BaseAttackType>,
    pub personality_q: Query<'w, 's, &'static Personality>,
    pub home_q: Query<'w, 's, &'static Home>,
    pub work_state_q: Query<'w, 's, &'static NpcWorkState>,
    pub carried_gold_q: Query<'w, 's, &'static CarriedGold>,
    pub weapon_q: Query<'w, 's, &'static EquippedWeapon>,
    pub helmet_q: Query<'w, 's, &'static EquippedHelmet>,
    pub armor_q: Query<'w, 's, &'static EquippedArmor>,
    pub has_energy_q: Query<'w, 's, &'static HasEnergy>,
}

/// NPC tracking resources for load.
#[derive(SystemParam)]
pub struct LoadNpcTracking<'w> {
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub npc_meta: ResMut<'w, NpcMetaCache>,
    pub npcs_by_town: ResMut<'w, NpcsByTownCache>,
    pub slots: ResMut<'w, GpuSlotPool>,
    pub combat_log: ResMut<'w, CombatLog>,
    pub gpu_state: ResMut<'w, GpuReadState>,
    pub dirty_writers: crate::messages::DirtyWriters<'w>,
    pub tilemap_spawned: ResMut<'w, crate::render::TilemapSpawned>,
    pub building_hp_render: ResMut<'w, BuildingHpRender>,
    pub healing_cache: ResMut<'w, HealingZoneCache>,
    pub active_healing: ResMut<'w, ActiveHealingSlots>,
    pub npc_gpu_state: ResMut<'w, crate::gpu::EntityGpuState>,
}

// ============================================================================
// BUILDING HP — entity-based save/load bridge
// ============================================================================

/// Build building HP hashmap from entity queries for save format.
/// Produces the same JSON as the old BuildingHpState serde.
fn collect_building_hp(
    building_query: &Query<(&Building, &GpuSlot, &Health), Without<Dead>>,
    entity_map: &EntityMap,
) -> std::collections::HashMap<String, Vec<f32>> {
    use std::collections::HashMap;
    let mut slot_hp: HashMap<usize, f32> = HashMap::new();
    for (_, npc_idx, health) in building_query.iter() {
        slot_hp.insert(npc_idx.0, health.0);
    }
    let mut map: HashMap<String, Vec<f32>> = HashMap::new();
    for def in crate::constants::BUILDING_REGISTRY {
        let key = if def.kind == world::BuildingKind::Fountain {
            "towns"
        } else {
            match def.save_key {
                Some(k) => k,
                None => continue,
            }
        };
        let mut insts: Vec<_> = entity_map.iter_kind(def.kind).collect();
        insts.sort_by_key(|i| i.slot);
        let hps: Vec<f32> = insts
            .iter()
            .map(|inst| slot_hp.get(&inst.slot).copied().unwrap_or(0.0))
            .collect();
        map.insert(key.into(), hps);
    }
    map
}

/// Convert old HP format (save_key → Vec<f32>) to slot-keyed HashMap.
pub fn convert_building_hp_to_slots(
    old_hp: &std::collections::HashMap<String, Vec<f32>>,
    entity_map: &EntityMap,
    _world_data: &world::WorldData,
) -> std::collections::HashMap<usize, f32> {
    let mut result = std::collections::HashMap::new();
    for def in crate::constants::BUILDING_REGISTRY {
        let key = if def.kind == world::BuildingKind::Fountain {
            Some("towns")
        } else {
            def.save_key
        };
        let Some(key) = key else { continue };
        let Some(hps) = old_hp.get(key) else { continue };
        // by_kind preserves insertion order, which matches save-file ordinal order
        // (load_building_instances_from_save appends in save-file order)
        let slots: Vec<usize> = entity_map.iter_kind(def.kind).map(|i| i.slot).collect();
        for (i, &hp) in hps.iter().enumerate() {
            if let Some(&slot) = slots.get(i) {
                result.insert(slot, hp);
            }
        }
    }
    result
}

/// Load building instances from save data directly into EntityMap.
/// Handles backward compat: deserializes PlacedBuilding vecs, skips tombstoned (is_alive check).
pub fn load_building_instances_from_save(
    save: &SaveData,
    slot_alloc: &mut crate::resources::GpuSlotPool,
    entity_map: &mut EntityMap,
    world_data: &WorldData,
    world_size_px: f32,
    uid_alloc: &mut NextEntityUid,
    commands: &mut Commands,
    gpu_updates: &mut MessageWriter<crate::messages::GpuUpdateMsg>,
) {
    use crate::constants::{BUILDING_REGISTRY, FACTION_NEUTRAL};
    entity_map.clear_buildings();
    entity_map.entities.clear();
    entity_map.init_spatial(world_size_px);

    // HP lookup: per save_key (or "towns" for Fountain), indexed by ordinal
    let fountain_hps = save.building_hp.get("towns");

    // Fountain instances from towns
    for (i, town) in world_data.towns.iter().enumerate() {
        if !world::is_alive(town.center) {
            continue;
        }
        let hp = fountain_hps.and_then(|v| v.get(i).copied());
        let _ = world::place_building(
            slot_alloc, entity_map, uid_alloc, commands, gpu_updates,
            world::BuildingKind::Fountain, town.center, i as u32, town.faction,
            0, 0, None, hp, None, None,
        );
    }

    // All other kinds from building_data
    for def in BUILDING_REGISTRY {
        let Some(key) = def.save_key else { continue };
        let Some(val) = save.building_data.get(key) else {
            continue;
        };
        let buildings: Vec<world::PlacedBuilding> =
            serde_json::from_value(val.clone()).unwrap_or_default();
        let kind_hps = save.building_hp.get(key);
        for (i, b) in buildings.iter().enumerate() {
            if !world::is_alive(b.position) {
                continue;
            }
            let faction = if def.kind == world::BuildingKind::GoldMine {
                FACTION_NEUTRAL
            } else {
                world_data
                    .towns
                    .get(b.town_idx as usize)
                    .map(|t| t.faction)
                    .unwrap_or(0)
            };
            let hp = kind_hps.and_then(|v| v.get(i).copied());
            let _ = world::place_building(
                slot_alloc, entity_map, uid_alloc, commands, gpu_updates,
                def.kind, b.position, b.town_idx, faction,
                b.patrol_order, b.wall_level, None, hp, None, None,
            );
            // Restore miner home fields
            if def.kind == world::BuildingKind::MinerHome {
                if let Some(inst) = entity_map.find_by_position_mut(b.position) {
                    inst.assigned_mine = b.assigned_mine;
                    inst.manual_mine = b.manual_mine;
                }
            }
        }
    }

    // Restore spawner state (npc_uid, respawn_timer) from save data
    for s in &save.spawners {
        let pos = to_vec2(s.position);
        if let Some(inst) = entity_map.find_by_position_mut(pos) {
            inst.npc_uid = s.npc_uid.map(crate::components::EntityUid);
            inst.respawn_timer = s.respawn_timer;
            inst.under_construction = s.under_construction;
        }
    }

    info!(
        "Loaded {} building instances from save",
        entity_map.building_count()
    );
}

/// Rebuild growth states from EntityMap instances + save data.
pub fn restore_growth_from_save(save: &SaveData, entity_map: &mut EntityMap) {
    // Farms — sort by slot to match save order
    let mut farm_slots: Vec<usize> = entity_map
        .iter_kind(world::BuildingKind::Farm)
        .map(|i| i.slot)
        .collect();
    farm_slots.sort();
    let farm_growth = if save.version < 2 {
        &save.farm_growth[..farm_slots.len().min(save.farm_growth.len())]
    } else {
        &save.farm_growth[..]
    };
    for (i, &slot) in farm_slots.iter().enumerate() {
        if let Some(inst) = entity_map.get_instance_mut(slot) {
            if let Some(fg) = farm_growth.get(i) {
                inst.growth_ready = fg.state == 1;
                inst.growth_progress = fg.progress;
                inst.under_construction = fg.under_construction;
            }
        }
    }

    // Mines — sort by slot to match save order
    let mut mine_slots: Vec<usize> = entity_map
        .iter_kind(world::BuildingKind::GoldMine)
        .map(|i| i.slot)
        .collect();
    mine_slots.sort();
    for (i, &slot) in mine_slots.iter().enumerate() {
        if let Some(inst) = entity_map.get_instance_mut(slot) {
            if let Some(mg) = save.mine_growth.get(i) {
                inst.growth_ready = mg.state == 1;
                inst.growth_progress = mg.progress;
                inst.under_construction = mg.under_construction;
            }
        }
    }
}

// ============================================================================
// BEVY SYSTEMS
// ============================================================================

/// Keyboard quick save/load shortcuts. Emits save/load messages.
pub fn save_load_input_system(
    keys: Res<ButtonInput<KeyCode>>,
    ui_state: Res<UiState>,
    settings: Res<UserSettings>,
    mut save_msgs: MessageWriter<SaveGameMsg>,
    mut load_msgs: MessageWriter<LoadGameMsg>,
) {
    if ui_state.pause_menu_open {
        return;
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::QuickSave)) {
        save_msgs.write(SaveGameMsg);
    }
    if keys.just_pressed(settings.key_for_action(ControlAction::QuickLoad)) {
        load_msgs.write(LoadGameMsg);
    }
}

/// Execute save when requested.
pub fn save_game_system(
    mut save_msgs: MessageReader<SaveGameMsg>,
    mut request: ResMut<SaveLoadRequest>,
    mut toast: ResMut<SaveToast>,
    ws: SaveWorldState,
    fs: SaveFactionState,
    entity_map: Res<EntityMap>,
    npc_meta: Res<NpcMetaCache>,
    building_query: Query<(&Building, &GpuSlot, &Health), Without<Dead>>,
    nq: SaveNpcQueries,
    uid_alloc: Res<NextEntityUid>,
) {
    if save_msgs.read().next().is_none() {
        return;
    }

    let npcs = collect_npc_data(
        &entity_map,
        &npc_meta,
        &nq.squad_id_q,
        &nq.activity_q,
        &nq.position_q,
        &nq.health_q,
        &nq.energy_q,
        &nq.combat_state_q,
        &nq.attack_type_q,
        &nq.personality_q,
        &nq.home_q,
        &nq.work_state_q,
        &nq.carried_gold_q,
        &nq.weapon_q,
        &nq.helmet_q,
        &nq.armor_q,
        &nq.has_energy_q,
    );
    let building_hp = collect_building_hp(&building_query, &entity_map);
    let data = collect_save_data(
        &ws.grid,
        &ws.world_data,
        &entity_map,
        &ws.town_grids,
        &ws.game_time,
        &ws.food_storage,
        &ws.gold_storage,
        building_hp,
        &ws.upgrades,
        &ws.policies,
        &ws.auto_upgrade,
        &ws.squad_state,
        &fs.raider_state,
        &fs.faction_stats,
        &fs.kill_stats,
        &fs.ai_state,
        &fs.migration_state,
        &fs.endless,
        npcs,
        &uid_alloc,
    );

    let result = if let Some(path) = request.save_path.take() {
        write_save_to(&data, &path)
    } else {
        write_save(&data)
    };

    match result {
        Ok(()) => {
            toast.message = format!("Game Saved ({} NPCs)", data.npcs.len());
            toast.timer = 2.0;
        }
        Err(e) => {
            error!("Save failed: {e}");
            toast.message = format!("Save failed: {e}");
            toast.timer = 3.0;
        }
    }
}

/// Autosave system — triggers on hour_ticked, writes to rotating autosave_N.json files.
pub fn autosave_system(
    mut request: ResMut<SaveLoadRequest>,
    mut toast: ResMut<SaveToast>,
    ws: SaveWorldState,
    fs: SaveFactionState,
    entity_map: Res<EntityMap>,
    npc_meta: Res<NpcMetaCache>,
    building_query: Query<(&Building, &GpuSlot, &Health), Without<Dead>>,
    nq: SaveNpcQueries,
    uid_alloc: Res<NextEntityUid>,
) {
    if request.autosave_hours <= 0 || !ws.game_time.hour_ticked {
        return;
    }

    let current_hour = ws.game_time.total_hours();
    if current_hour - request.autosave_last_hour < request.autosave_hours {
        return;
    }
    request.autosave_last_hour = current_hour;

    let slot = request.autosave_slot;
    request.autosave_slot = (slot + 1) % 3;

    let Some(path) = autosave_path(slot) else {
        return;
    };

    let npcs = collect_npc_data(
        &entity_map,
        &npc_meta,
        &nq.squad_id_q,
        &nq.activity_q,
        &nq.position_q,
        &nq.health_q,
        &nq.energy_q,
        &nq.combat_state_q,
        &nq.attack_type_q,
        &nq.personality_q,
        &nq.home_q,
        &nq.work_state_q,
        &nq.carried_gold_q,
        &nq.weapon_q,
        &nq.helmet_q,
        &nq.armor_q,
        &nq.has_energy_q,
    );
    let building_hp = collect_building_hp(&building_query, &entity_map);
    let data = collect_save_data(
        &ws.grid,
        &ws.world_data,
        &entity_map,
        &ws.town_grids,
        &ws.game_time,
        &ws.food_storage,
        &ws.gold_storage,
        building_hp,
        &ws.upgrades,
        &ws.policies,
        &ws.auto_upgrade,
        &ws.squad_state,
        &fs.raider_state,
        &fs.faction_stats,
        &fs.kill_stats,
        &fs.ai_state,
        &fs.migration_state,
        &fs.endless,
        npcs,
        &uid_alloc,
    );

    match write_save_to(&data, &path) {
        Ok(()) => {
            toast.message = format!("Autosaved slot {} ({} NPCs)", slot + 1, data.npcs.len());
            toast.timer = 2.0;
        }
        Err(e) => {
            error!("Autosave failed: {e}");
            toast.message = format!("Autosave failed: {e}");
            toast.timer = 3.0;
        }
    }
}

/// Spawn NPC entities from save data. Shared between in-game load (F9) and menu load.
pub fn spawn_npcs_from_save(
    save: &SaveData,
    commands: &mut Commands,
    entity_map: &mut EntityMap,
    pop_stats: &mut PopulationStats,
    npc_meta: &mut NpcMetaCache,
    npcs_by_town: &mut NpcsByTownCache,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    world_data: &WorldData,
    combat_config: &CombatConfig,
    upgrades: &TownUpgrades,
    uid_alloc: &mut NextEntityUid,
) {
    for npc in &save.npcs {
        let uid_override = npc.uid.map(crate::components::EntityUid);
        let overrides = NpcSpawnOverrides {
            health: Some(npc.health),
            energy: Some(npc.energy),
            activity: Some(npc.activity.to_activity()),
            combat_state: Some(npc.combat_state.to_combat_state()),
            personality: Some(npc.personality.to_personality()),
            name: Some(npc.name.clone()),
            level: Some(npc.level),
            xp: Some(npc.xp),
            weapon: npc.weapon,
            helmet: npc.helmet,
            armor: npc.armor,
            carried_gold: npc.carried_gold,
            squad_id: npc.squad_id,
            uid_override,
        };

        // Patrol units always get starting_post=0 on load (patrol route rebuilt from world)
        let starting_post =
            if crate::constants::npc_def(Job::from_i32(npc.job as i32)).is_patrol_unit {
                0
            } else {
                -1
            };

        materialize_npc(
            npc.slot,
            npc.position[0],
            npc.position[1],
            npc.job as i32,
            npc.faction,
            npc.town_id,
            npc.home,
            npc.work_position,
            starting_post,
            npc.attack_type as i32,
            &overrides,
            commands,
            entity_map,
            pop_stats,
            npc_meta,
            npcs_by_town,
            gpu_updates,
            world_data,
            combat_config,
            upgrades,
            uid_alloc,
        );
    }
}

/// Shared save-restore pipeline used by both menu load and in-game F9 load.
/// Assumes caller already read `save` and (for in-game load) despawned old entities.
pub fn restore_world_from_save(
    save: &SaveData,
    commands: &mut Commands,
    ws: &mut SaveWorldState,
    fs: &mut SaveFactionState,
    tracking: &mut LoadNpcTracking,
    entity_map: &mut EntityMap,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    combat_config: &CombatConfig,
    uid_alloc: &mut NextEntityUid,
) {
    // Reset transient runtime resources.
    *entity_map = Default::default();
    // Restore UID counter from save, or start fresh for old saves
    uid_alloc.0 = save.next_entity_uid.unwrap_or(1);
    *tracking.pop_stats = Default::default();
    *tracking.combat_log = Default::default();
    *tracking.gpu_state = Default::default();
    *tracking.building_hp_render = Default::default();
    *tracking.npc_gpu_state = Default::default();
    *tracking.active_healing = Default::default();
    tracking.dirty_writers.emit_all();
    tracking.tilemap_spawned.0 = false;

    // Apply save snapshot to world resources.
    apply_save(
        save,
        &mut ws.grid,
        &mut ws.world_data,
        &mut ws.town_grids,
        &mut ws.game_time,
        &mut ws.food_storage,
        &mut ws.gold_storage,
        &mut ws.upgrades,
        &mut ws.policies,
        &mut ws.auto_upgrade,
        &mut ws.squad_state,
        &mut fs.raider_state,
        &mut fs.faction_stats,
        &mut fs.kill_stats,
        &mut fs.ai_state,
        &mut fs.migration_state,
        &mut fs.endless,
        &mut tracking.npcs_by_town,
        &mut tracking.slots,
    );

    // Rebuild buildings from save payload.
    let world_size_px = ws.grid.width as f32 * ws.grid.cell_size;
    load_building_instances_from_save(
        save,
        &mut tracking.slots,
        entity_map,
        &ws.world_data,
        world_size_px,
        uid_alloc,
        commands,
        gpu_updates,
    );
    world::update_all_wall_sprites(&ws.grid, entity_map, gpu_updates);
    restore_growth_from_save(save, entity_map);

    // Rebuild NPCs from save payload.
    spawn_npcs_from_save(
        save,
        commands,
        entity_map,
        &mut tracking.pop_stats,
        &mut tracking.npc_meta,
        &mut tracking.npcs_by_town,
        gpu_updates,
        &ws.world_data,
        combat_config,
        &ws.upgrades,
        uid_alloc,
    );

    // Old-save fixup: convert legacy squad member slots to UIDs (NPCs are now spawned)
    for (si, ss) in save.squads.iter().enumerate() {
        if ss.member_uids.is_none() && si < ws.squad_state.squads.len() {
            ws.squad_state.squads[si].members = ss
                .members
                .iter()
                .filter_map(|&slot| entity_map.uid_for_slot(slot))
                .collect();
        }
    }

    // Migration markers are restored via NpcInstance.migrating in spawn — no ECS marker needed.
}

/// Execute load when requested. Despawns all NPCs and rebuilds from save.
pub fn load_game_system(
    mut commands: Commands,
    mut load_msgs: MessageReader<LoadGameMsg>,
    mut request: ResMut<SaveLoadRequest>,
    mut toast: ResMut<SaveToast>,
    mut ws: SaveWorldState,
    mut fs: SaveFactionState,
    mut tracking: LoadNpcTracking,
    mut entity_map: ResMut<EntityMap>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    combat_config: Res<CombatConfig>,
    npc_query: Query<Entity, With<GpuSlot>>,
    marker_query: Query<Entity, With<FarmReadyMarker>>,
    mut uid_alloc: ResMut<NextEntityUid>,
) {
    if load_msgs.read().next().is_none() {
        return;
    }

    // Read save file (from explicit path or quicksave)
    let save = match if let Some(path) = request.load_path.take() {
        read_save_from(&path)
    } else {
        read_save()
    } {
        Ok(data) => data,
        Err(e) => {
            error!("Load failed: {e}");
            toast.message = format!("Load failed: {e}");
            toast.timer = 3.0;
            return;
        }
    };

    let town_count = save
        .building_data
        .get("towns")
        .and_then(|v| v.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    info!(
        "Loading save: {} NPCs, {} towns",
        save.npcs.len(),
        town_count
    );

    // 1. Despawn all NPC entities + farm markers
    for entity in npc_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in marker_query.iter() {
        commands.entity(entity).despawn();
    }

    restore_world_from_save(
        &save,
        &mut commands,
        &mut ws,
        &mut fs,
        &mut tracking,
        &mut entity_map,
        &mut gpu_updates,
        &combat_config,
        &mut uid_alloc,
    );

    toast.message = format!("Game Loaded ({} NPCs)", save.npcs.len());
    toast.timer = 2.0;
    info!("Load complete: {} NPCs restored", save.npcs.len());
}

/// Tick down toast timer.
pub fn save_toast_tick_system(time: Res<Time>, mut toast: ResMut<SaveToast>) {
    if toast.timer > 0.0 {
        toast.timer -= time.delta_secs();
    }
}
