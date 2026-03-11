//! Building registry — single source of truth for all building definitions.

use crate::world::BuildingKind;
use super::npcs::{ItemKind, LootDrop};
use super::{TowerStats, FOUNTAIN_TOWER, TOWER_STATS, MINE_WORK_RADIUS};

/// Tile specification: single 16x16 sprite or 2x2 composite of four 16x16 sprites.
#[derive(Clone, Copy, Debug)]
pub enum TileSpec {
    Single(u32, u32),
    Quad([(u32, u32); 4]),  // [TL, TR, BL, BR]
    External(&'static str), // asset path, e.g. "sprites/house.png"
}

/// How a building is placed on the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PlacementMode {
    /// Snap to town grid (farms, homes, beds, tents).
    TownGrid,
    /// Snap to world grid (waypoints, fountains, gold mines).
    Wilderness,
}

/// Special action when a building is placed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum OnPlace {
    None,
    /// Initialize farm growth on BuildingInstance.
    InitFarmGrowth,
}

/// How a spawner building finds work/patrol targets for its NPC.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SpawnBehavior {
    /// Find nearest free farm in own town (farmer).
    FindNearestFarm,
    /// Find nearest waypoint for patrol (archer, crossbow).
    FindNearestWaypoint,
    /// Use raider town faction (tent → raider).
    Raider,
    /// Use assigned mine or find nearest (miner).
    Miner,
}

/// NPC spawner definition — what kind of NPC a building produces.
#[derive(Clone, Copy, Debug)]
pub struct SpawnerDef {
    pub job: i32, // Job::from_i32 index (0=Farmer, 1=Archer, 2=Raider, 4=Miner, 5=Crossbow)
    pub behavior: SpawnBehavior,
}

/// Factions tab column assignment for building display.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DisplayCategory {
    Hidden,
    Economy,
    Military,
    Tower,
}

/// Worksite occupancy config for buildings that NPCs can claim and work at.
#[derive(Clone, Copy, Debug)]
pub struct WorksiteDef {
    pub max_occupants: i32,
    pub drift_radius: f32,
    pub upgrade_job: &'static str,
    pub harvest_item: ItemKind,
    pub town_scoped: bool,
}

/// Complete building definition — one entry per BuildingKind.
/// Index in BUILDING_REGISTRY = tileset index for GPU rendering.
#[derive(Clone, Copy, Debug)]
pub struct BuildingDef {
    pub kind: BuildingKind,
    pub display: DisplayCategory,
    pub tile: TileSpec,
    pub hp: f32,
    pub cost: i32,
    pub label: &'static str,
    pub help: &'static str,
    pub tooltip: &'static str,
    pub player_buildable: bool,
    pub raider_buildable: bool,
    pub placement: PlacementMode,
    pub is_tower: bool,
    pub tower_stats: Option<TowerStats>,
    pub on_place: OnPlace,
    pub spawner: Option<SpawnerDef>,
    /// Save key in JSON (None for Fountain which uses towns vec).
    pub save_key: Option<&'static str>,
    /// Whether this kind uses unit_homes BTreeMap storage.
    pub is_unit_home: bool,
    /// Worksite config (None = not a worksite NPCs can occupy).
    pub worksite: Option<WorksiteDef>,
    /// True = uses 4-neighbor auto-tiling (requires TileSpec::External sprite strip).
    pub autotile: bool,
}

impl BuildingDef {
    /// Loot dropped when this building is destroyed: half the build cost as food.
    pub fn loot_drop(&self) -> Option<LootDrop> {
        let amount = self.cost / 2;
        if amount > 0 {
            Some(LootDrop {
                item: ItemKind::Food,
                min: amount,
                max: amount,
            })
        } else {
            None
        }
    }
}

/// Single source of truth for all building types.
/// Order must match tileset strip (index = tileset_index).
pub const BUILDING_REGISTRY: &[BuildingDef] = &[
    // 0: Fountain (town center, auto-shoots)
    BuildingDef {
        kind: BuildingKind::Fountain,
        display: DisplayCategory::Hidden,
        tile: TileSpec::Single(50, 9),
        hp: 500.0,
        cost: 0,
        label: "Fountain",
        help: "Town center",
        tooltip: "",
        player_buildable: false,
        raider_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: true,
        tower_stats: Some(FOUNTAIN_TOWER),
        on_place: OnPlace::None,
        spawner: None,
        save_key: None,
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 1: Bed
    BuildingDef {
        kind: BuildingKind::Bed,
        display: DisplayCategory::Hidden,
        tile: TileSpec::Single(15, 2),
        hp: 50.0,
        cost: 0,
        label: "Bed",
        help: "NPC rest spot",
        tooltip: "",
        player_buildable: false,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("beds"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 2: Waypoint
    BuildingDef {
        kind: BuildingKind::Waypoint,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/waypoint.png"),
        hp: 200.0,
        cost: 1,
        label: "Waypoint",
        help: "Patrol waypoint",
        tooltip: "Archers patrol between waypoints to guard\nyour territory. Place outside town to extend\npatrol coverage. HP: 200",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("waypoints"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 3: Farm
    BuildingDef {
        kind: BuildingKind::Farm,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/farm_64x64.png"),
        hp: 80.0,
        cost: 2,
        label: "Farm",
        help: "Grows food over time",
        tooltip: "Grows food passively (0.08/hr). Farmers tend\nit 3x faster (0.25/hr). Harvest yields 1 food.\nBuild near Farmer Homes for fast delivery. HP: 80",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::InitFarmGrowth,
        spawner: None,
        save_key: Some("farms"),
        is_unit_home: false,
        worksite: Some(WorksiteDef {
            max_occupants: 1,
            drift_radius: 20.0,
            upgrade_job: "Farmer",
            harvest_item: ItemKind::Food,
            town_scoped: true,
        }),
        autotile: false,
    },
    // 5: Farmer Home
    BuildingDef {
        kind: BuildingKind::FarmerHome,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/farmer_home_64x64.png"),
        hp: 100.0,
        cost: 2,
        label: "Farmer Home",
        help: "Spawns 1 farmer",
        tooltip: "Trains 1 farmer who tends farms and carries\nfood home. 100 HP, speed 100. Respawns 12 hrs\nafter death. Build near farms for short trips.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 0,
            behavior: SpawnBehavior::FindNearestFarm,
        }),
        save_key: Some("farmer_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 6: Archer Home
    BuildingDef {
        kind: BuildingKind::ArcherHome,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/archer_home_64x64.png"),
        hp: 150.0,
        cost: 4,
        label: "Archer Home",
        help: "Spawns 1 archer",
        tooltip: "Trains 1 archer — ranged defender. 100 HP,\n15 dmg, range 100, 1.5s cooldown. Patrols\nbetween waypoints. Respawns 12 hrs after death.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 1,
            behavior: SpawnBehavior::FindNearestWaypoint,
        }),
        save_key: Some("archer_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 7: Tent (raider spawner)
    BuildingDef {
        kind: BuildingKind::Tent,
        display: DisplayCategory::Military,
        tile: TileSpec::Quad([(48, 10), (49, 10), (48, 11), (49, 11)]),
        hp: 100.0,
        cost: 3,
        label: "Tent",
        help: "Spawns 1 raider",
        tooltip: "",
        player_buildable: false,
        raider_buildable: true,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 2,
            behavior: SpawnBehavior::Raider,
        }),
        save_key: Some("tents"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 8: Gold Mine
    BuildingDef {
        kind: BuildingKind::GoldMine,
        display: DisplayCategory::Hidden,
        tile: TileSpec::Single(43, 11),
        hp: 200.0,
        cost: 0,
        label: "Gold Mine",
        help: "Source of gold",
        tooltip: "",
        player_buildable: false,
        raider_buildable: false,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("gold_mines"),
        is_unit_home: false,
        worksite: Some(WorksiteDef {
            max_occupants: 5,
            drift_radius: MINE_WORK_RADIUS,
            upgrade_job: "Miner",
            harvest_item: ItemKind::Gold,
            town_scoped: false,
        }),
        autotile: false,
    },
    // 9: Miner Home
    BuildingDef {
        kind: BuildingKind::MinerHome,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/miner_home_64x64.png"),
        hp: 100.0,
        cost: 4,
        label: "Miner Home",
        help: "Spawns 1 miner",
        tooltip: "Trains 1 miner who extracts gold from mines.\n5 gold per harvest (4 hr cycle). 100 HP, speed\n110. Gold funds upgrades. Respawns 12 hrs.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 4,
            behavior: SpawnBehavior::Miner,
        }),
        save_key: Some("miner_homes"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 10: Crossbow Home
    BuildingDef {
        kind: BuildingKind::CrossbowHome,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/crossbowman_home_64x64.png"),
        hp: 150.0,
        cost: 8,
        label: "Crossbow Home",
        help: "Spawns 1 crossbow",
        tooltip: "Trains 1 crossbow — elite ranged unit. 100 HP,\n25 dmg, range 150, 2s cooldown. Highest DPS\nranged unit. Patrols waypoints. Respawns 12 hrs.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 5,
            behavior: SpawnBehavior::FindNearestWaypoint,
        }),
        save_key: Some("crossbow_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 11: Fighter Home
    BuildingDef {
        kind: BuildingKind::FighterHome,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/fighter_home_64x64.png"),
        hp: 150.0,
        cost: 5,
        label: "Fighter Home",
        help: "Spawns 1 fighter",
        tooltip: "Trains 1 fighter — melee combatant. 100 HP,\n22.5 dmg, range 50, 1s cooldown. High melee\nDPS, engages up close. Patrols waypoints.\nRespawns 12 hrs.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: Some(SpawnerDef {
            job: 3,
            behavior: SpawnBehavior::FindNearestWaypoint,
        }),
        save_key: Some("fighter_homes"),
        is_unit_home: true,
        worksite: None,
        autotile: false,
    },
    // 12: Road (dirt) — expands buildable area by 3 tiles
    BuildingDef {
        kind: BuildingKind::Road,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/dirt_roads_131_32.png"),
        hp: 30.0,
        cost: 1,
        label: "Dirt Road",
        help: "1.5x speed, +3 build radius",
        tooltip: "Expands buildable area 3 tiles around the road.\nNPCs move 50% faster. Click-drag to build lines.\nUpgrade to Stone Road for more range. HP: 30",
        player_buildable: true,
        raider_buildable: true,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("roads"),
        is_unit_home: false,
        worksite: None,
        autotile: true,
    },
    // 13: StoneRoad — expands buildable area by 5 tiles
    BuildingDef {
        kind: BuildingKind::StoneRoad,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/dirt_roads_131_32.png"), // TODO: stone road sprite
        hp: 60.0,
        cost: 3,
        label: "Stone Road",
        help: "2x speed, +5 build radius",
        tooltip: "Expands buildable area 5 tiles around the road.\nNPCs move 2x faster. Click existing dirt road\nto upgrade. HP: 60",
        player_buildable: true,
        raider_buildable: true,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("stone_roads"),
        is_unit_home: false,
        worksite: None,
        autotile: true,
    },
    // 14: MetalRoad — expands buildable area by 7 tiles
    BuildingDef {
        kind: BuildingKind::MetalRoad,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/dirt_roads_131_32.png"), // TODO: metal road sprite
        hp: 100.0,
        cost: 8,
        label: "Metal Road",
        help: "2.5x speed, +7 build radius",
        tooltip: "Expands buildable area 7 tiles around the road.\nNPCs move 2.5x faster. Click existing stone road\nto upgrade. HP: 100",
        player_buildable: true,
        raider_buildable: true,
        placement: PlacementMode::Wilderness,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("metal_roads"),
        is_unit_home: false,
        worksite: None,
        autotile: true,
    },
    // 15: Wall
    BuildingDef {
        kind: BuildingKind::Wall,
        display: DisplayCategory::Military,
        tile: TileSpec::External("sprites/wood_walls_131x32.png"),
        hp: 80.0,
        cost: 1,
        label: "Wall",
        help: "Blocks enemy movement",
        tooltip: "Defensive wall — blocks enemy NPCs from\npassing through. Click to upgrade tier.\nWooden: 80 HP, Stone: 200, Fortified: 400.",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("walls"),
        is_unit_home: false,
        worksite: None,
        autotile: true,
    },
    // 14: Tower (auto-shoots enemies)
    BuildingDef {
        kind: BuildingKind::Tower,
        display: DisplayCategory::Tower,
        tile: TileSpec::External("sprites/tower-1.png"),
        hp: 1000.0,
        cost: 40,
        label: "Tower",
        help: "Auto-attacks nearby enemies",
        tooltip: "Defensive tower — auto-shoots nearest enemy.\nSame range/damage as archer. 15 dmg, 1.5s cooldown. HP: 1000",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: true,
        tower_stats: Some(TOWER_STATS),
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("towers"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 15: Merchant (buy/sell equipment)
    BuildingDef {
        kind: BuildingKind::Merchant,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/merchant_64x64.png"),
        hp: 200.0,
        cost: 50,
        label: "Merchant",
        help: "Buy and sell equipment",
        tooltip: "Merchant — buy gear with gold, sell unwanted items.\nStock refreshes every 12 game-hours. 1 per town. HP: 200",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("merchants"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
    // 16: Casino (blackjack minigame, 1 per town)
    BuildingDef {
        kind: BuildingKind::Casino,
        display: DisplayCategory::Economy,
        tile: TileSpec::External("sprites/casino_64x64.png"),
        hp: 200.0,
        cost: 80,
        label: "Casino",
        help: "Play blackjack",
        tooltip: "Casino — play blackjack against AI factions for gold.\n1 per town. HP: 200",
        player_buildable: true,
        raider_buildable: false,
        placement: PlacementMode::TownGrid,
        is_tower: false,
        tower_stats: None,
        on_place: OnPlace::None,
        spawner: None,
        save_key: Some("casinos"),
        is_unit_home: false,
        worksite: None,
        autotile: false,
    },
];

/// Look up a building definition by kind. Panics if kind is not in registry.
pub fn building_def(kind: BuildingKind) -> &'static BuildingDef {
    BUILDING_REGISTRY
        .iter()
        .find(|d| d.kind == kind)
        .unwrap_or_else(|| panic!("no BuildingDef for {:?}", kind))
}

/// Look up the tileset index for a BuildingKind (its position in BUILDING_REGISTRY).
pub fn tileset_index(kind: BuildingKind) -> u16 {
    BUILDING_REGISTRY
        .iter()
        .position(|d| d.kind == kind)
        .unwrap_or_else(|| panic!("no tileset index for {:?}", kind)) as u16
}

/// Food cost to build a building. Returns 0 for non-buildable types.
pub fn building_cost(kind: BuildingKind) -> i32 {
    building_def(kind).cost
}

// ── Autotile helpers ──────────────────────────────────────────────

/// Extra atlas layers per auto-tile kind (NS, 4 corners, cross, 4 T-junctions = 10).
pub const AUTOTILE_EXTRA_PER_KIND: usize = 10;

/// Auto-tile variant indices (0 = base/E-W layer at tileset_index, 1-10 = appended extras).
pub const AUTOTILE_EW: u16 = 0;
pub const AUTOTILE_NS: u16 = 1;
pub const AUTOTILE_BL: u16 = 2;  // BR src(0°) → BL on screen
pub const AUTOTILE_BR: u16 = 3;  // BL(90°) → BR on screen
pub const AUTOTILE_TR: u16 = 4;  // TL(180°) → TR on screen
pub const AUTOTILE_TL: u16 = 5;  // TR(270°) → TL on screen
pub const AUTOTILE_CROSS: u16 = 6;
pub const AUTOTILE_T_OPEN_N: u16 = 7;
pub const AUTOTILE_T_OPEN_W: u16 = 8;
pub const AUTOTILE_T_OPEN_S: u16 = 9;
pub const AUTOTILE_T_OPEN_E: u16 = 10;

/// Number of building kinds with autotile enabled.
pub fn autotile_kind_count() -> usize {
    BUILDING_REGISTRY.iter().filter(|d| d.autotile).count()
}

/// Total extra atlas layers for all auto-tiled kinds.
pub fn autotile_total_extra_layers() -> usize {
    autotile_kind_count() * AUTOTILE_EXTRA_PER_KIND
}

/// Get the autotile order index (0, 1, 2...) for a building kind among all autotile kinds.
/// Returns None if the kind doesn't use autotile.
pub fn autotile_order(kind: BuildingKind) -> Option<usize> {
    let mut order = 0;
    for def in BUILDING_REGISTRY {
        if def.kind == kind {
            return if def.autotile { Some(order) } else { None };
        }
        if def.autotile {
            order += 1;
        }
    }
    None
}

/// Compute the atlas column for an auto-tile variant.
/// Variant 0 (E-W) uses the building's base tileset index.
/// Variants 1-10 use appended extra layers.
pub fn autotile_col(kind: BuildingKind, variant: u16) -> f32 {
    if variant == 0 {
        return tileset_index(kind) as f32;
    }
    let order = autotile_order(kind).unwrap_or(0);
    let extra_base = BUILDING_REGISTRY.len() + order * AUTOTILE_EXTRA_PER_KIND;
    (extra_base as u16 + variant - 1) as f32
}
