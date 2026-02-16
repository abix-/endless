//! Save/Load system — quicksave (F5) / quickload (F9) with JSON serialization.
//! Save format is self-contained: dedicated serde structs decouple from ECS types.

use bevy::prelude::*;
use serde::{Serialize, Deserialize};

use crate::components::*;
use crate::constants::*;
use crate::messages::{GpuUpdate, GpuUpdateMsg};
use crate::resources::*;
use crate::systems::stats::{TownUpgrades, CombatConfig, UPGRADE_COUNT, resolve_combat_stats};
use crate::systems::{pop_inc_alive, AiPlayerState};
use crate::systems::spawn::build_patrol_route;
use crate::world::{self, WorldData, WorldGrid, WorldCell, TownGrids};

// ============================================================================
// VEC2 HELPERS
// ============================================================================

fn v2(v: Vec2) -> [f32; 2] { [v.x, v.y] }
fn to_vec2(a: [f32; 2]) -> Vec2 { Vec2::new(a[0], a[1]) }

// ============================================================================
// SAVE FORMAT STRUCTS
// ============================================================================

const SAVE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct SaveData {
    pub version: u32,

    // World grid
    pub grid_width: usize,
    pub grid_height: usize,
    pub grid_cell_size: f32,
    pub terrain: Vec<u8>,                     // Biome as u8
    pub buildings: Vec<Option<BuildingSave>>,  // parallel to terrain

    // World data
    pub towns: Vec<TownSave>,
    pub farms: Vec<PosTownSave>,
    pub beds: Vec<PosTownSave>,
    pub guard_posts: Vec<GuardPostSave>,
    pub farmer_homes: Vec<PosTownSave>,
    pub archer_homes: Vec<PosTownSave>,
    pub tents: Vec<PosTownSave>,
    pub miner_homes: Vec<PosTownSave>,
    pub gold_mines: Vec<[f32; 2]>,

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

    // Mine states
    pub mine_gold: Vec<f32>,
    pub mine_max_gold: Vec<f32>,
    pub mine_positions: Vec<[f32; 2]>,

    // Spawners
    pub spawners: Vec<SpawnerSave>,

    // Building HP
    pub building_hp: BuildingHpSave,

    // Upgrades + policies
    pub upgrades: Vec<Vec<u8>>,
    #[serde(default)]
    pub policies: Vec<PolicySet>,
    #[serde(default)]
    pub auto_upgrades: Vec<Vec<bool>>,

    // Squads
    pub squads: Vec<SquadSave>,

    // Guard post turret state
    pub guard_post_attack: Vec<bool>,

    // Camp state
    pub camp_respawn_timers: Vec<f32>,
    pub camp_forage_timers: Vec<f32>,
    pub camp_max_pop: Vec<i32>,

    // Faction stats
    pub faction_stats: Vec<FactionStatSave>,
    pub kill_stats: [i32; 2], // [archer_kills, villager_kills]

    // NPCs
    pub npcs: Vec<NpcSaveData>,

    // AI players
    pub ai_players: Vec<AiPlayerSave>,
}

// Sub-structs

#[derive(Serialize, Deserialize, Clone)]
pub struct TownSave {
    pub name: String,
    pub center: [f32; 2],
    pub faction: i32,
    pub sprite_type: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct PosTownSave {
    pub position: [f32; 2],
    pub town_idx: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct GuardPostSave {
    pub position: [f32; 2],
    pub town_idx: u32,
    pub patrol_order: u32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TownGridSave {
    pub town_data_idx: usize,
    pub area_level: i32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct FarmGrowthSave {
    pub state: u8, // 0=Growing, 1=Ready
    pub progress: f32,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SpawnerSave {
    pub building_kind: i32,
    pub town_idx: i32,
    pub position: [f32; 2],
    pub npc_slot: i32,
    pub respawn_timer: f32,
}

#[derive(Serialize, Deserialize, Clone, Default)]
pub struct BuildingHpSave {
    pub guard_posts: Vec<f32>,
    pub farmer_homes: Vec<f32>,
    pub archer_homes: Vec<f32>,
    pub tents: Vec<f32>,
    pub miner_homes: Vec<f32>,
    pub farms: Vec<f32>,
    pub towns: Vec<f32>,
    pub beds: Vec<f32>,
    pub gold_mines: Vec<f32>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SquadSave {
    pub members: Vec<usize>,
    pub target: Option<[f32; 2]>,
    pub target_size: usize,
    pub patrol_enabled: bool,
    pub rest_when_tired: bool,
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
    pub kind: u8,         // 0=Raider, 1=Builder
    pub personality: u8,  // 0=Aggressive, 1=Balanced, 2=Economic
}

// Building save (mirrors world::Building)
#[derive(Serialize, Deserialize, Clone)]
pub enum BuildingSave {
    Fountain { town_idx: u32 },
    Farm { town_idx: u32 },
    Bed { town_idx: u32 },
    GuardPost { town_idx: u32, patrol_order: u32 },
    Camp { town_idx: u32 },
    FarmerHome { town_idx: u32 },
    ArcherHome { town_idx: u32 },
    Tent { town_idx: u32 },
    GoldMine,
    MinerHome { town_idx: u32 },
}

impl BuildingSave {
    fn from_building(b: &world::Building) -> Self {
        match *b {
            world::Building::Fountain { town_idx } => Self::Fountain { town_idx },
            world::Building::Farm { town_idx } => Self::Farm { town_idx },
            world::Building::Bed { town_idx } => Self::Bed { town_idx },
            world::Building::GuardPost { town_idx, patrol_order } => Self::GuardPost { town_idx, patrol_order },
            world::Building::Camp { town_idx } => Self::Camp { town_idx },
            world::Building::FarmerHome { town_idx } => Self::FarmerHome { town_idx },
            world::Building::ArcherHome { town_idx } => Self::ArcherHome { town_idx },
            world::Building::Tent { town_idx } => Self::Tent { town_idx },
            world::Building::GoldMine => Self::GoldMine,
            world::Building::MinerHome { town_idx } => Self::MinerHome { town_idx },
        }
    }

    fn to_building(&self) -> world::Building {
        match *self {
            Self::Fountain { town_idx } => world::Building::Fountain { town_idx },
            Self::Farm { town_idx } => world::Building::Farm { town_idx },
            Self::Bed { town_idx } => world::Building::Bed { town_idx },
            Self::GuardPost { town_idx, patrol_order } => world::Building::GuardPost { town_idx, patrol_order },
            Self::Camp { town_idx } => world::Building::Camp { town_idx },
            Self::FarmerHome { town_idx } => world::Building::FarmerHome { town_idx },
            Self::ArcherHome { town_idx } => world::Building::ArcherHome { town_idx },
            Self::Tent { town_idx } => world::Building::Tent { town_idx },
            Self::GoldMine => world::Building::GoldMine,
            Self::MinerHome { town_idx } => world::Building::MinerHome { town_idx },
        }
    }
}

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
    Returning { has_food: bool, gold: i32 },
    Mining { mine_pos: [f32; 2] },
    MiningAtMine,
}

impl ActivitySave {
    fn from_activity(a: &Activity) -> Self {
        match a {
            Activity::Idle => Self::Idle,
            Activity::Working => Self::Working,
            Activity::OnDuty { ticks_waiting } => Self::OnDuty { ticks_waiting: *ticks_waiting },
            Activity::Patrolling => Self::Patrolling,
            Activity::GoingToWork => Self::GoingToWork,
            Activity::GoingToRest => Self::GoingToRest,
            Activity::Resting => Self::Resting,
            Activity::GoingToHeal => Self::GoingToHeal,
            Activity::HealingAtFountain { recover_until } => Self::HealingAtFountain { recover_until: *recover_until },
            Activity::Wandering => Self::Wandering,
            Activity::Raiding { target } => Self::Raiding { target: v2(*target) },
            Activity::Returning { has_food, gold } => Self::Returning { has_food: *has_food, gold: *gold },
            Activity::Mining { mine_pos } => Self::Mining { mine_pos: v2(*mine_pos) },
            Activity::MiningAtMine => Self::MiningAtMine,
        }
    }

    fn to_activity(&self) -> Activity {
        match self {
            Self::Idle => Activity::Idle,
            Self::Working => Activity::Working,
            Self::OnDuty { ticks_waiting } => Activity::OnDuty { ticks_waiting: *ticks_waiting },
            Self::Patrolling => Activity::Patrolling,
            Self::GoingToWork => Activity::GoingToWork,
            Self::GoingToRest => Activity::GoingToRest,
            Self::Resting => Activity::Resting,
            Self::GoingToHeal => Activity::GoingToHeal,
            Self::HealingAtFountain { recover_until } => Activity::HealingAtFountain { recover_until: *recover_until },
            Self::Wandering => Activity::Wandering,
            Self::Raiding { target } => Activity::Raiding { target: to_vec2(*target) },
            Self::Returning { has_food, gold } => Activity::Returning { has_food: *has_food, gold: *gold },
            Self::Mining { mine_pos } => Activity::Mining { mine_pos: to_vec2(*mine_pos) },
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
            CombatState::Fighting { origin } => Self::Fighting { origin: v2(*origin) },
            CombatState::Fleeing => Self::Fleeing,
        }
    }

    fn to_combat_state(&self) -> CombatState {
        match self {
            Self::None => CombatState::None,
            Self::Fighting { origin } => CombatState::Fighting { origin: to_vec2(*origin) },
            Self::Fleeing => CombatState::Fleeing,
        }
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TraitSave {
    pub kind: u8,       // 0=Brave, 1=Tough, 2=Swift, 3=Focused
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
            let kind = match t.kind {
                TraitKind::Brave => 0,
                TraitKind::Tough => 1,
                TraitKind::Swift => 2,
                TraitKind::Focused => 3,
            };
            TraitSave { kind, magnitude: t.magnitude }
        };
        Self {
            trait1: p.trait1.as_ref().map(map_trait),
            trait2: p.trait2.as_ref().map(map_trait),
        }
    }

    fn to_personality(&self) -> Personality {
        let map_trait = |t: &TraitSave| -> TraitInstance {
            let kind = match t.kind {
                0 => TraitKind::Brave,
                1 => TraitKind::Tough,
                2 => TraitKind::Swift,
                _ => TraitKind::Focused,
            };
            TraitInstance { kind, magnitude: t.magnitude }
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
    let dir = std::path::PathBuf::from(home).join("Documents").join("Endless").join("saves");
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
    town_grids: &TownGrids,
    game_time: &GameTime,
    food_storage: &FoodStorage,
    gold_storage: &GoldStorage,
    farm_states: &FarmStates,
    mine_states: &MineStates,
    spawner_state: &SpawnerState,
    building_hp: &BuildingHpState,
    upgrades: &TownUpgrades,
    policies: &TownPolicies,
    auto_upgrade: &AutoUpgrade,
    squad_state: &SquadState,
    guard_post_state: &GuardPostState,
    camp_state: &CampState,
    faction_stats: &FactionStats,
    kill_stats: &KillStats,
    ai_state: &AiPlayerState,
    npcs: Vec<NpcSaveData>,
) -> SaveData {
    // Terrain + buildings
    let terrain: Vec<u8> = grid.cells.iter().map(|c| biome_to_u8(c.terrain)).collect();
    let buildings: Vec<Option<BuildingSave>> = grid.cells.iter()
        .map(|c| c.building.as_ref().map(BuildingSave::from_building))
        .collect();

    // Towns
    let towns: Vec<TownSave> = world_data.towns.iter().map(|t| TownSave {
        name: t.name.clone(),
        center: v2(t.center),
        faction: t.faction,
        sprite_type: t.sprite_type,
    }).collect();

    // Buildings
    let farms: Vec<PosTownSave> = world_data.farms.iter().map(|f| PosTownSave {
        position: v2(f.position), town_idx: f.town_idx,
    }).collect();
    let beds: Vec<PosTownSave> = world_data.beds.iter().map(|b| PosTownSave {
        position: v2(b.position), town_idx: b.town_idx,
    }).collect();
    let guard_posts: Vec<GuardPostSave> = world_data.guard_posts.iter().map(|g| GuardPostSave {
        position: v2(g.position), town_idx: g.town_idx, patrol_order: g.patrol_order,
    }).collect();
    let farmer_homes: Vec<PosTownSave> = world_data.farmer_homes.iter().map(|h| PosTownSave {
        position: v2(h.position), town_idx: h.town_idx,
    }).collect();
    let archer_homes: Vec<PosTownSave> = world_data.archer_homes.iter().map(|h| PosTownSave {
        position: v2(h.position), town_idx: h.town_idx,
    }).collect();
    let tents_save: Vec<PosTownSave> = world_data.tents.iter().map(|t| PosTownSave {
        position: v2(t.position), town_idx: t.town_idx,
    }).collect();
    let miner_homes: Vec<PosTownSave> = world_data.miner_homes.iter().map(|m| PosTownSave {
        position: v2(m.position), town_idx: m.town_idx,
    }).collect();
    let gold_mines_save: Vec<[f32; 2]> = world_data.gold_mines.iter().map(|m| v2(m.position)).collect();

    // Town grids
    let town_grids_save: Vec<TownGridSave> = town_grids.grids.iter().map(|g| TownGridSave {
        town_data_idx: g.town_data_idx, area_level: g.area_level,
    }).collect();

    // Farm growth
    let farm_growth: Vec<FarmGrowthSave> = farm_states.states.iter().zip(farm_states.progress.iter())
        .map(|(s, p)| FarmGrowthSave {
            state: match s { FarmGrowthState::Growing => 0, FarmGrowthState::Ready => 1 },
            progress: *p,
        }).collect();

    // Spawners
    let spawners: Vec<SpawnerSave> = spawner_state.0.iter().map(|s| SpawnerSave {
        building_kind: s.building_kind,
        town_idx: s.town_idx,
        position: v2(s.position),
        npc_slot: s.npc_slot,
        respawn_timer: s.respawn_timer,
    }).collect();

    // Building HP
    let building_hp_save = BuildingHpSave {
        guard_posts: building_hp.guard_posts.clone(),
        farmer_homes: building_hp.farmer_homes.clone(),
        archer_homes: building_hp.archer_homes.clone(),
        tents: building_hp.tents.clone(),
        miner_homes: building_hp.miner_homes.clone(),
        farms: building_hp.farms.clone(),
        towns: building_hp.towns.clone(),
        beds: building_hp.beds.clone(),
        gold_mines: building_hp.gold_mines.clone(),
    };

    // Upgrades (convert [u8; UPGRADE_COUNT] to Vec<u8>)
    let upgrades_save: Vec<Vec<u8>> = upgrades.levels.iter()
        .map(|l| l.to_vec())
        .collect();

    // Auto-upgrades (convert [bool; UPGRADE_COUNT] to Vec<bool>)
    let auto_upgrades_save: Vec<Vec<bool>> = auto_upgrade.flags.iter()
        .map(|f| f.to_vec())
        .collect();

    // Squads
    let squads: Vec<SquadSave> = squad_state.squads.iter().map(|s| SquadSave {
        members: s.members.clone(),
        target: s.target.map(v2),
        target_size: s.target_size,
        patrol_enabled: s.patrol_enabled,
        rest_when_tired: s.rest_when_tired,
    }).collect();

    // Faction stats
    let faction_stats_save: Vec<FactionStatSave> = faction_stats.stats.iter().map(|s| FactionStatSave {
        alive: s.alive, dead: s.dead, kills: s.kills,
    }).collect();

    // AI players
    let ai_players: Vec<AiPlayerSave> = ai_state.players.iter().map(|p| {
        use crate::systems::ai_player::*;
        AiPlayerSave {
            town_data_idx: p.town_data_idx,
            grid_idx: p.grid_idx,
            kind: match p.kind { AiKind::Raider => 0, AiKind::Builder => 1 },
            personality: match p.personality {
                AiPersonality::Aggressive => 0,
                AiPersonality::Balanced => 1,
                AiPersonality::Economic => 2,
            },
        }
    }).collect();

    SaveData {
        version: SAVE_VERSION,
        grid_width: grid.width,
        grid_height: grid.height,
        grid_cell_size: grid.cell_size,
        terrain,
        buildings,
        towns,
        farms,
        beds,
        guard_posts,
        farmer_homes,
        archer_homes,
        tents: tents_save,
        miner_homes,
        gold_mines: gold_mines_save,
        town_grids: town_grids_save,
        total_seconds: game_time.total_seconds,
        seconds_per_hour: game_time.seconds_per_hour,
        time_scale: game_time.time_scale,
        food: food_storage.food.clone(),
        gold: gold_storage.gold.clone(),
        farm_growth,
        mine_gold: mine_states.gold.clone(),
        mine_max_gold: mine_states.max_gold.clone(),
        mine_positions: mine_states.positions.iter().map(|p| v2(*p)).collect(),
        spawners,
        building_hp: building_hp_save,
        upgrades: upgrades_save,
        policies: policies.policies.clone(),
        auto_upgrades: auto_upgrades_save,
        squads,
        guard_post_attack: guard_post_state.attack_enabled.clone(),
        camp_respawn_timers: camp_state.respawn_timers.clone(),
        camp_forage_timers: camp_state.forage_timers.clone(),
        camp_max_pop: camp_state.max_pop.clone(),
        faction_stats: faction_stats_save,
        kill_stats: [kill_stats.archer_kills, kill_stats.villager_kills],
        npcs,
        ai_players,
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

/// Read SaveData from the quicksave file.
pub fn read_save() -> Result<SaveData, String> {
    let path = quicksave_path().ok_or("cannot determine save directory")?;
    let json = std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))?;
    let data: SaveData = serde_json::from_str(&json).map_err(|e| format!("deserialize: {e}"))?;
    if data.version > SAVE_VERSION {
        return Err(format!("save version {} > supported {}", data.version, SAVE_VERSION));
    }
    Ok(data)
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
    farm_states: &mut FarmStates,
    mine_states: &mut MineStates,
    spawner_state: &mut SpawnerState,
    building_hp: &mut BuildingHpState,
    upgrades: &mut TownUpgrades,
    policies: &mut TownPolicies,
    auto_upgrade: &mut AutoUpgrade,
    squad_state: &mut SquadState,
    guard_post_state: &mut GuardPostState,
    camp_state: &mut CampState,
    faction_stats: &mut FactionStats,
    kill_stats: &mut KillStats,
    ai_state: &mut AiPlayerState,
    npcs_by_town: &mut NpcsByTownCache,
    slots: &mut SlotAllocator,
) {
    // World grid
    grid.width = save.grid_width;
    grid.height = save.grid_height;
    grid.cell_size = save.grid_cell_size;
    grid.cells = save.terrain.iter().zip(save.buildings.iter())
        .map(|(&t, b)| WorldCell {
            terrain: u8_to_biome(t),
            building: b.as_ref().map(|bs| bs.to_building()),
        }).collect();

    // World data
    world_data.towns = save.towns.iter().map(|t| world::Town {
        name: t.name.clone(), center: to_vec2(t.center), faction: t.faction, sprite_type: t.sprite_type,
    }).collect();
    world_data.farms = save.farms.iter().map(|f| world::Farm {
        position: to_vec2(f.position), town_idx: f.town_idx,
    }).collect();
    world_data.beds = save.beds.iter().map(|b| world::Bed {
        position: to_vec2(b.position), town_idx: b.town_idx,
    }).collect();
    world_data.guard_posts = save.guard_posts.iter().map(|g| world::GuardPost {
        position: to_vec2(g.position), town_idx: g.town_idx, patrol_order: g.patrol_order,
        npc_slot: None,
    }).collect();
    world_data.farmer_homes = save.farmer_homes.iter().map(|h| world::FarmerHome {
        position: to_vec2(h.position), town_idx: h.town_idx,
    }).collect();
    world_data.archer_homes = save.archer_homes.iter().map(|h| world::ArcherHome {
        position: to_vec2(h.position), town_idx: h.town_idx,
    }).collect();
    world_data.tents = save.tents.iter().map(|t| world::Tent {
        position: to_vec2(t.position), town_idx: t.town_idx,
    }).collect();
    world_data.miner_homes = save.miner_homes.iter().map(|m| world::MinerHome {
        position: to_vec2(m.position), town_idx: m.town_idx,
    }).collect();
    world_data.gold_mines = save.gold_mines.iter().map(|p| world::GoldMine {
        position: to_vec2(*p),
    }).collect();

    // Town grids
    town_grids.grids = save.town_grids.iter().map(|g| world::TownGrid {
        town_data_idx: g.town_data_idx, area_level: g.area_level,
    }).collect();

    // Game time
    game_time.total_seconds = save.total_seconds;
    game_time.seconds_per_hour = save.seconds_per_hour;
    game_time.time_scale = save.time_scale;
    game_time.start_hour = 6;
    game_time.last_hour = game_time.total_hours();
    game_time.hour_ticked = false;
    game_time.paused = false;

    // Economy
    food_storage.food = save.food.clone();
    gold_storage.gold = save.gold.clone();

    // Farm states
    farm_states.states = save.farm_growth.iter().map(|fg| {
        if fg.state == 1 { FarmGrowthState::Ready } else { FarmGrowthState::Growing }
    }).collect();
    farm_states.progress = save.farm_growth.iter().map(|fg| fg.progress).collect();
    farm_states.positions = save.farms.iter().map(|f| to_vec2(f.position)).collect();

    // Mine states
    mine_states.gold = save.mine_gold.clone();
    mine_states.max_gold = save.mine_max_gold.clone();
    mine_states.positions = save.mine_positions.iter().map(|p| to_vec2(*p)).collect();

    // Spawners
    spawner_state.0 = save.spawners.iter().map(|s| SpawnerEntry {
        building_kind: s.building_kind,
        town_idx: s.town_idx,
        position: to_vec2(s.position),
        npc_slot: s.npc_slot,
        respawn_timer: s.respawn_timer,
    }).collect();

    // Building HP
    *building_hp = BuildingHpState {
        guard_posts: save.building_hp.guard_posts.clone(),
        farmer_homes: save.building_hp.farmer_homes.clone(),
        archer_homes: save.building_hp.archer_homes.clone(),
        tents: save.building_hp.tents.clone(),
        miner_homes: save.building_hp.miner_homes.clone(),
        farms: save.building_hp.farms.clone(),
        towns: save.building_hp.towns.clone(),
        beds: save.building_hp.beds.clone(),
        gold_mines: save.building_hp.gold_mines.clone(),
    };

    // Upgrades
    upgrades.levels = save.upgrades.iter().map(|v| {
        let mut arr = [0u8; UPGRADE_COUNT];
        for (i, &val) in v.iter().enumerate().take(UPGRADE_COUNT) {
            arr[i] = val;
        }
        arr
    }).collect();

    // Policies
    let num_towns = save.towns.len();
    policies.policies = save.policies.clone();
    policies.policies.resize(num_towns.max(16), PolicySet::default());

    // Auto-upgrades
    auto_upgrade.flags = save.auto_upgrades.iter().map(|v| {
        let mut arr = [false; UPGRADE_COUNT];
        for (i, &val) in v.iter().enumerate().take(UPGRADE_COUNT) {
            arr[i] = val;
        }
        arr
    }).collect();
    auto_upgrade.flags.resize(num_towns.max(16), [false; UPGRADE_COUNT]);

    // Squads
    for (i, ss) in save.squads.iter().enumerate() {
        if i < squad_state.squads.len() {
            squad_state.squads[i] = Squad {
                members: ss.members.clone(),
                target: ss.target.map(to_vec2),
                target_size: ss.target_size,
                patrol_enabled: ss.patrol_enabled,
                rest_when_tired: ss.rest_when_tired,
            };
        }
    }
    squad_state.selected = 0;
    squad_state.placing_target = false;

    // Guard post turret state
    guard_post_state.timers = vec![0.0; save.guard_post_attack.len()];
    guard_post_state.attack_enabled = save.guard_post_attack.clone();

    // Camp state
    camp_state.max_pop = save.camp_max_pop.clone();
    camp_state.respawn_timers = save.camp_respawn_timers.clone();
    camp_state.forage_timers = save.camp_forage_timers.clone();

    // Faction stats
    faction_stats.stats = save.faction_stats.iter().map(|s| FactionStat {
        alive: s.alive, dead: s.dead, kills: s.kills,
    }).collect();

    // Kill stats
    kill_stats.archer_kills = save.kill_stats[0];
    kill_stats.villager_kills = save.kill_stats[1];

    // AI players
    {
        use crate::systems::ai_player::*;
        use std::collections::VecDeque;
        ai_state.players = save.ai_players.iter().map(|p| AiPlayer {
            town_data_idx: p.town_data_idx,
            grid_idx: p.grid_idx,
            kind: if p.kind == 0 { AiKind::Raider } else { AiKind::Builder },
            personality: match p.personality {
                0 => AiPersonality::Aggressive,
                2 => AiPersonality::Economic,
                _ => AiPersonality::Balanced,
            },
            last_actions: VecDeque::new(),
        }).collect();
    }

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
    slots.next = max_slot;
    // Free list = all slots below max_slot that aren't used
    for i in 0..max_slot {
        if !used_slots.contains(&i) {
            slots.free.push(i);
        }
    }
}

// ============================================================================
// SAVE/LOAD TRIGGER RESOURCE
// ============================================================================

/// Trigger resource for save/load operations.
#[derive(Resource, Default)]
pub struct SaveLoadRequest {
    pub save_requested: bool,
    pub load_requested: bool,
    /// Set by main menu "Load Game" — tells game_startup_system to load instead of world gen.
    pub load_on_enter: bool,
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

/// Toast notification state for save/load feedback.
#[derive(Resource, Default)]
pub struct SaveToast {
    pub message: String,
    pub timer: f32,
}

// ============================================================================
// NPC QUERY — uses nested tuples to stay under Bevy's 16-element limit
// ============================================================================

// Core query: 12 elements (under 16)
type NpcCoreQuery = (
    &'static NpcIndex, &'static Position, &'static Job, &'static Faction, &'static TownId,
    &'static Health, &'static Activity, &'static CombatState, &'static Personality,
    &'static BaseAttackType, &'static Home, Option<&'static Energy>,
);

// Extras query: 7 elements
type NpcExtrasQuery = (
    &'static NpcIndex,
    Option<&'static WorkPosition>, Option<&'static SquadId>, Option<&'static CarriedGold>,
    Option<&'static EquippedWeapon>, Option<&'static EquippedHelmet>, Option<&'static EquippedArmor>,
);

/// Collect NPC save data from two ECS queries (core + extras) joined by NpcEntityMap.
pub fn collect_npc_data(
    core_query: &Query<NpcCoreQuery, Without<Dead>>,
    extras_query: &Query<NpcExtrasQuery, Without<Dead>>,
    npc_map: &NpcEntityMap,
    npc_meta: &NpcMetaCache,
) -> Vec<NpcSaveData> {
    let mut npcs = Vec::new();
    for (npc_idx, pos, job, faction, town_id,
         health, activity, combat_state, personality,
         attack_type, home, energy) in core_query.iter()
    {
        let idx = npc_idx.0;
        let meta = &npc_meta.0[idx];

        // Look up extras via entity
        let (work_pos, squad_id, carried_gold, weapon, helmet, armor) =
            if let Some(&entity) = npc_map.0.get(&idx) {
                if let Ok((_idx, wp, sq, cg, wep, hel, arm)) = extras_query.get(entity) {
                    (wp, sq, cg, wep, hel, arm)
                } else {
                    (None, None, None, None, None, None)
                }
            } else {
                (None, None, None, None, None, None)
            };

        npcs.push(NpcSaveData {
            slot: idx,
            position: [pos.x, pos.y],
            job: match *job {
                Job::Farmer => 0, Job::Archer => 1, Job::Raider => 2,
                Job::Fighter => 3, Job::Miner => 4,
            },
            faction: faction.to_i32(),
            town_id: town_id.0,
            health: health.0,
            energy: energy.map(|e| e.0).unwrap_or(100.0),
            activity: ActivitySave::from_activity(activity),
            combat_state: CombatStateSave::from_combat_state(combat_state),
            personality: PersonalitySave::from_personality(personality),
            name: meta.name.clone(),
            level: meta.level,
            xp: meta.xp,
            attack_type: match *attack_type { BaseAttackType::Melee => 0, BaseAttackType::Ranged => 1 },
            home: v2(home.0),
            work_position: work_pos.map(|w| v2(w.0)),
            squad_id: squad_id.map(|s| s.0),
            carried_gold: carried_gold.map(|g| g.0),
            weapon: weapon.map(|w| [w.0, w.1]),
            helmet: helmet.map(|h| [h.0, h.1]),
            armor: armor.map(|a| [a.0, a.1]),
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
    pub farm_states: ResMut<'w, FarmStates>,
    pub mine_states: ResMut<'w, MineStates>,
    pub spawner_state: ResMut<'w, SpawnerState>,
    pub building_hp: ResMut<'w, BuildingHpState>,
    pub upgrades: ResMut<'w, TownUpgrades>,
    pub policies: ResMut<'w, TownPolicies>,
    pub auto_upgrade: ResMut<'w, AutoUpgrade>,
    pub squad_state: ResMut<'w, SquadState>,
    pub guard_post_state: ResMut<'w, GuardPostState>,
}

/// More world state + faction/AI resources.
#[derive(SystemParam)]
pub struct SaveFactionState<'w> {
    pub camp_state: ResMut<'w, CampState>,
    pub faction_stats: ResMut<'w, FactionStats>,
    pub kill_stats: ResMut<'w, KillStats>,
    pub ai_state: ResMut<'w, AiPlayerState>,
}

/// NPC tracking resources for load.
#[derive(SystemParam)]
pub struct LoadNpcTracking<'w> {
    pub npc_map: ResMut<'w, NpcEntityMap>,
    pub pop_stats: ResMut<'w, PopulationStats>,
    pub npc_meta: ResMut<'w, NpcMetaCache>,
    pub npcs_by_town: ResMut<'w, NpcsByTownCache>,
    pub slots: ResMut<'w, SlotAllocator>,
    pub combat_log: ResMut<'w, CombatLog>,
    pub gpu_state: ResMut<'w, GpuReadState>,
    pub dirty: ResMut<'w, DirtyFlags>,
    pub tilemap_spawned: ResMut<'w, crate::render::TilemapSpawned>,
    pub building_hp_render: ResMut<'w, BuildingHpRender>,
    pub bgrid: ResMut<'w, world::BuildingSpatialGrid>,
    pub healing_cache: ResMut<'w, HealingZoneCache>,
}

// ============================================================================
// BEVY SYSTEMS
// ============================================================================

/// F5 = save, F9 = load. Sets flags on SaveLoadRequest.
pub fn save_load_input_system(
    keys: Res<ButtonInput<KeyCode>>,
    mut request: ResMut<SaveLoadRequest>,
) {
    if keys.just_pressed(KeyCode::F5) {
        request.save_requested = true;
    }
    if keys.just_pressed(KeyCode::F9) {
        request.load_requested = true;
    }
}

/// Execute save when requested.
pub fn save_game_system(
    mut request: ResMut<SaveLoadRequest>,
    mut toast: ResMut<SaveToast>,
    ws: SaveWorldState,
    fs: SaveFactionState,
    npc_map: Res<NpcEntityMap>,
    npc_meta: Res<NpcMetaCache>,
    core_query: Query<NpcCoreQuery, Without<Dead>>,
    extras_query: Query<NpcExtrasQuery, Without<Dead>>,
) {
    if !request.save_requested { return; }
    request.save_requested = false;

    let npcs = collect_npc_data(&core_query, &extras_query, &npc_map, &npc_meta);
    let data = collect_save_data(
        &ws.grid, &ws.world_data, &ws.town_grids, &ws.game_time,
        &ws.food_storage, &ws.gold_storage, &ws.farm_states, &ws.mine_states,
        &ws.spawner_state, &ws.building_hp, &ws.upgrades, &ws.policies, &ws.auto_upgrade,
        &ws.squad_state, &ws.guard_post_state, &fs.camp_state, &fs.faction_stats,
        &fs.kill_stats, &fs.ai_state, npcs,
    );

    match write_save(&data) {
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
    npc_map: Res<NpcEntityMap>,
    npc_meta: Res<NpcMetaCache>,
    core_query: Query<NpcCoreQuery, Without<Dead>>,
    extras_query: Query<NpcExtrasQuery, Without<Dead>>,
) {
    if request.autosave_hours <= 0 || !ws.game_time.hour_ticked { return; }

    let current_hour = ws.game_time.total_hours();
    if current_hour - request.autosave_last_hour < request.autosave_hours { return; }
    request.autosave_last_hour = current_hour;

    let slot = request.autosave_slot;
    request.autosave_slot = (slot + 1) % 3;

    let Some(path) = autosave_path(slot) else { return };

    let npcs = collect_npc_data(&core_query, &extras_query, &npc_map, &npc_meta);
    let data = collect_save_data(
        &ws.grid, &ws.world_data, &ws.town_grids, &ws.game_time,
        &ws.food_storage, &ws.gold_storage, &ws.farm_states, &ws.mine_states,
        &ws.spawner_state, &ws.building_hp, &ws.upgrades, &ws.policies, &ws.auto_upgrade,
        &ws.squad_state, &ws.guard_post_state, &fs.camp_state, &fs.faction_stats,
        &fs.kill_stats, &fs.ai_state, npcs,
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
    npc_map: &mut NpcEntityMap,
    pop_stats: &mut PopulationStats,
    npc_meta: &mut NpcMetaCache,
    npcs_by_town: &mut NpcsByTownCache,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    world_data: &WorldData,
    combat_config: &CombatConfig,
    upgrades: &TownUpgrades,
) {
    for npc in &save.npcs {
        let job = Job::from_i32(npc.job as i32);
        let attack_type = if npc.attack_type == 1 { BaseAttackType::Ranged } else { BaseAttackType::Melee };
        let personality = npc.personality.to_personality();
        let faction = Faction::from_i32(npc.faction);

        let cached = resolve_combat_stats(
            job, attack_type, npc.town_id, npc.level, &personality, combat_config, upgrades,
        );

        let (sprite_col, sprite_row) = match job {
            Job::Farmer => SPRITE_FARMER,
            Job::Archer => SPRITE_ARCHER,
            Job::Raider => SPRITE_RAIDER,
            Job::Fighter => SPRITE_FIGHTER,
            Job::Miner => SPRITE_MINER,
        };
        let idx = npc.slot;
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx, x: npc.position[0], y: npc.position[1] }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: npc.position[0], y: npc.position[1] }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx, faction: npc.faction }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: npc.health }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx, col: sprite_col, row: sprite_row, atlas: 0.0 }));

        let activity = npc.activity.to_activity();
        let combat_state = npc.combat_state.to_combat_state();

        let mut ec = commands.spawn((
            NpcIndex(idx),
            Position::new(npc.position[0], npc.position[1]),
            job,
            TownId(npc.town_id),
            Speed(cached.speed),
            Health(npc.health),
            cached,
            attack_type,
            faction,
            Home(to_vec2(npc.home)),
            personality,
            activity,
            combat_state,
        ));

        match job {
            Job::Archer => {
                ec.insert(Energy(npc.energy));
                ec.insert(AttackTimer(0.0));
                ec.insert(Archer);
                let w = npc.weapon.unwrap_or([EQUIP_SWORD.0, EQUIP_SWORD.1]);
                let h = npc.helmet.unwrap_or([EQUIP_HELMET.0, EQUIP_HELMET.1]);
                ec.insert((EquippedWeapon(w[0], w[1]), EquippedHelmet(h[0], h[1])));
                if let Some(a) = npc.armor {
                    ec.insert(EquippedArmor(a[0], a[1]));
                }
                let patrol_posts = build_patrol_route(world_data, npc.town_id as u32);
                if !patrol_posts.is_empty() {
                    ec.insert(PatrolRoute { posts: patrol_posts, current: 0 });
                }
            }
            Job::Farmer => {
                ec.insert(Energy(npc.energy));
                ec.insert(Farmer);
                if let Some(wp) = npc.work_position {
                    ec.insert(WorkPosition(to_vec2(wp)));
                }
            }
            Job::Raider => {
                ec.insert(Energy(npc.energy));
                ec.insert(AttackTimer(0.0));
                ec.insert(Stealer);
                let w = npc.weapon.unwrap_or([EQUIP_SWORD.0, EQUIP_SWORD.1]);
                ec.insert(EquippedWeapon(w[0], w[1]));
                ec.insert(LeashRange { distance: 400.0 });
                if let Some(cg) = npc.carried_gold {
                    ec.insert(CarriedGold(cg));
                }
            }
            Job::Miner => {
                ec.insert(Energy(npc.energy));
                ec.insert(Miner);
                if let Some(wp) = npc.work_position {
                    ec.insert(WorkPosition(to_vec2(wp)));
                }
            }
            Job::Fighter => {
                ec.insert(AttackTimer(0.0));
            }
        }

        if let Some(sq) = npc.squad_id {
            ec.insert(SquadId(sq));
        }

        npc_map.0.insert(idx, ec.id());
        pop_inc_alive(pop_stats, job, npc.town_id);

        if idx < npc_meta.0.len() {
            npc_meta.0[idx] = NpcMeta {
                name: npc.name.clone(),
                level: npc.level,
                xp: npc.xp,
                trait_id: 0,
                town_id: npc.town_id,
                job: npc.job as i32,
            };
        }

        if npc.town_id >= 0 {
            let ti = npc.town_id as usize;
            if ti < npcs_by_town.0.len() {
                npcs_by_town.0[ti].push(idx);
            }
        }
    }
}

/// Execute load when requested. Despawns all NPCs and rebuilds from save.
pub fn load_game_system(
    mut commands: Commands,
    mut request: ResMut<SaveLoadRequest>,
    mut toast: ResMut<SaveToast>,
    mut ws: SaveWorldState,
    mut fs: SaveFactionState,
    mut tracking: LoadNpcTracking,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    combat_config: Res<CombatConfig>,
    npc_query: Query<Entity, With<NpcIndex>>,
    marker_query: Query<Entity, With<FarmReadyMarker>>,
) {
    if !request.load_requested { return; }
    request.load_requested = false;

    // Read save file
    let save = match read_save() {
        Ok(data) => data,
        Err(e) => {
            error!("Load failed: {e}");
            toast.message = format!("Load failed: {e}");
            toast.timer = 3.0;
            return;
        }
    };

    info!("Loading save: {} NPCs, {} towns", save.npcs.len(), save.towns.len());

    // 1. Despawn all NPC entities + farm markers
    for entity in npc_query.iter() {
        commands.entity(entity).despawn();
    }
    for entity in marker_query.iter() {
        commands.entity(entity).despawn();
    }

    // 2. Reset transient resources
    *tracking.npc_map = Default::default();
    *tracking.pop_stats = Default::default();
    *tracking.combat_log = Default::default();
    *tracking.gpu_state = Default::default();
    *tracking.building_hp_render = Default::default();
    tracking.dirty.patrols = true;
    tracking.dirty.guard_post_slots = true;
    tracking.tilemap_spawned.0 = false; // Force tilemap rebuild with new terrain

    // 3. Apply save data to all game resources
    apply_save(
        &save,
        &mut ws.grid, &mut ws.world_data, &mut ws.town_grids, &mut ws.game_time,
        &mut ws.food_storage, &mut ws.gold_storage, &mut ws.farm_states, &mut ws.mine_states,
        &mut ws.spawner_state, &mut ws.building_hp, &mut ws.upgrades, &mut ws.policies,
        &mut ws.auto_upgrade, &mut ws.squad_state, &mut ws.guard_post_state, &mut fs.camp_state,
        &mut fs.faction_stats, &mut fs.kill_stats, &mut fs.ai_state,
        &mut tracking.npcs_by_town, &mut tracking.slots,
    );

    // 4. Rebuild spatial grid
    tracking.bgrid.rebuild(&ws.world_data, ws.grid.width as f32 * ws.grid.cell_size);
    tracking.dirty.building_grid = false;
    tracking.dirty.healing_zones = true;

    // 5. Spawn NPC entities from save data
    spawn_npcs_from_save(
        &save, &mut commands,
        &mut tracking.npc_map, &mut tracking.pop_stats, &mut tracking.npc_meta,
        &mut tracking.npcs_by_town, &mut gpu_updates,
        &ws.world_data, &combat_config, &ws.upgrades,
    );

    toast.message = format!("Game Loaded ({} NPCs)", save.npcs.len());
    toast.timer = 2.0;
    info!("Load complete: {} NPCs restored", save.npcs.len());
}

/// Tick down toast timer.
pub fn save_toast_tick_system(
    time: Res<Time>,
    mut toast: ResMut<SaveToast>,
) {
    if toast.timer > 0.0 {
        toast.timer -= time.delta_secs();
    }
}
