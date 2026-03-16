//! ECS Components - Bevy entities have these attached

use bevy::prelude::*;
use bevy::reflect::Reflect;

// ============================================================================
// CORE COMPONENTS
// ============================================================================

/// Links a Bevy entity to its unified slot in the GPU entity buffers.
/// Both NPCs and buildings get an GpuSlot(n) where n = GPU buffer index.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct GpuSlot(pub usize);

/// NPC position in world coordinates. Bevy owns this, syncs to GPU for physics.
/// Phase 11: Replaces GPU-owned positions with Bevy-owned + GPU accelerated.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct Position {
    pub x: f32,
    pub y: f32,
}

impl Position {
    pub fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// NPC's job determines behavior and color.
/// - Farmer (green): works at farms, avoids combat
/// - Archer (blue): patrols and fights raiders
/// - Raider (red): attacks guards, steals from farms
#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Reflect)]
#[reflect(Component)]
pub enum Job {
    Farmer,
    Archer,
    Raider,
    Fighter,
    Miner,
    Crossbow,
    Boat,
    Woodcutter,
    Quarrier,
    Mason,
}

impl Job {
    /// Convert from integer (0=Farmer, 1=Archer, 2=Raider, 3=Fighter, 4=Miner, 5=Crossbow, 6=Boat, 7=Woodcutter, 8=Quarrier, 9=Mason)
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Job::Archer,
            2 => Job::Raider,
            3 => Job::Fighter,
            4 => Job::Miner,
            5 => Job::Crossbow,
            6 => Job::Boat,
            7 => Job::Woodcutter,
            8 => Job::Quarrier,
            9 => Job::Mason,
            _ => Job::Farmer,
        }
    }

    /// Display label from NPC registry.
    pub fn label(&self) -> &'static str {
        crate::constants::npc_def(*self).label
    }

    /// Sprite (col, row) from NPC registry.
    pub fn sprite(&self) -> (f32, f32) {
        crate::constants::npc_def(*self).sprite
    }

    /// RGBA color from NPC registry. Alpha=1.0 means "has target" on GPU.
    pub fn color(&self) -> (f32, f32, f32, f32) {
        crate::constants::npc_def(*self).color
    }

    /// Returns true for jobs that patrol waypoints and use squads.
    pub fn is_patrol_unit(&self) -> bool {
        crate::constants::npc_def(*self).is_patrol_unit
    }

    /// Returns true for combat-capable jobs.
    pub fn is_military(&self) -> bool {
        crate::constants::npc_def(*self).is_military
    }
}

/// Movement speed in pixels per second.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct Speed(pub f32);

impl Default for Speed {
    fn default() -> Self {
        Self(100.0) // 100 pixels/second base speed
    }
}

// NPC type markers (Archer, Farmer, Miner, Crossbow) removed — job identity lives in Job component.
// is_military/is_patrol_unit derived from Job::is_military()/Job::is_patrol_unit() at query time.

/// TownId identifies which town an NPC belongs to.
/// Universal component on every NPC. All settlements are "towns" (villager or raider).
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct TownId(pub i32);

// ============================================================================
// BEHAVIOR DATA COMPONENTS
// ============================================================================

/// NPC energy level (0-100). Drains while active, recovers while resting.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct Energy(pub f32);

impl Default for Energy {
    fn default() -> Self {
        Self(100.0)
    }
}

/// Where the NPC goes to rest (bed position).
/// Home(-1, -1) means no home assigned — behavior systems should skip.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct Home(pub Vec2);

impl Home {
    pub fn is_valid(&self) -> bool {
        self.0.x >= 0.0 && self.0.y >= 0.0
    }
}

/// Patrol route for guards (or any NPC that patrols).
#[derive(Component, Clone, Reflect)]
#[reflect(Component)]
pub struct PatrolRoute {
    pub posts: Vec<Vec2>,
    pub current: usize,
}

/// Combined work state for NPCs. Always present — avoids archetype churn from insert/remove.
/// Single `worksite` field: claimed worksite (occupancy incremented). Cleared on release/death.
#[derive(Component, Default, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct NpcWorkState {
    pub worksite: Option<Entity>,
}

/// Unified carry component for ALL NPCs. Always present — replaces the old fragmented
/// Activity::Returning{loot} payload + CarriedGold component.
/// Loot lives here; Activity::Returning just means "going home."
#[derive(Component, Default, Clone, Reflect)]
#[reflect(Component)]
pub struct CarriedLoot {
    pub food: i32,
    pub gold: i32,
    pub wood: i32,
    pub stone: i32,
    pub equipment: Vec<crate::constants::LootItem>,
}

impl CarriedLoot {
    pub fn is_empty(&self) -> bool {
        self.food <= 0
            && self.gold <= 0
            && self.wood <= 0
            && self.stone <= 0
            && self.equipment.is_empty()
    }

    pub fn total_items(&self) -> usize {
        (self.food > 0) as usize
            + (self.gold > 0) as usize
            + (self.wood > 0) as usize
            + (self.stone > 0) as usize
            + self.equipment.len()
    }

    /// Visual key for GPU dirty tracking: same key = same visual overlay.
    pub fn visual_key(&self) -> u8 {
        if !self.equipment.is_empty() {
            4
        } else if self.gold > 0 {
            2
        } else if self.food > 0 || self.wood > 0 || self.stone > 0 {
            3
        } else {
            0
        }
    }
}

// ============================================================================
// NPC STATE — Command (Factorio-inspired) × CombatState
// ============================================================================

/// Combat distraction policy — determines when an NPC interrupts its command to fight.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
pub enum Distraction {
    /// Ignore enemies entirely (rest, heal, return loot).
    None,
    /// Fight back only when hit (working, mining).
    ByDamage,
    /// Engage nearby enemies proactively (patrol, squad, idle).
    #[default]
    ByEnemy,
}

/// NPC activity — the single source of truth for what the NPC is doing.
/// Movement destination is derived from the activity. No separate transit state.
/// Fieldless registry key — per-instance data lives on `Activity` struct fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
pub enum ActivityKind {
    #[default]
    Idle,
    Work,
    Patrol,
    SquadAttack,
    Rest,
    Heal,
    Wander,
    Raid,
    ReturnLoot,
    Mine,
    Repair,
}

impl ActivityKind {
    pub fn def(&self) -> &'static crate::constants::ActivityDef {
        crate::constants::activity_def(*self)
    }
    pub fn distraction(&self) -> Distraction {
        self.def().distraction
    }
    pub fn label(&self) -> &'static str {
        self.def().label
    }
}

/// Lifecycle progress within an activity. Small, generic, stable.
/// New location types add target variants, not phase variants.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Reflect)]
pub enum ActivityPhase {
    #[default]
    Ready, // no active route; eligible for idle choice
    Transit, // moving toward target
    Active,  // performing sustained work/recovery at target
    Holding, // at target, waiting on external condition (patrol wait, mine queue)
}

/// Where or what an activity is operating on.
/// Destination identity lives here, not in ActivityPhase.
#[derive(Clone, Copy, Debug, Default, PartialEq, Reflect)]
pub enum ActivityTarget {
    #[default]
    None,
    Home,
    Fountain,
    PatrolPost {
        route: u16,
        index: u16,
    },
    SquadPoint(Vec2),
    Worksite,          // semantic: actual identity in NpcWorkState.worksite
    RaidPoint(Vec2),   // enemy farm position
    Dropoff,           // delivering loot to home
    WanderPoint(Vec2), // random destination
}

/// What the NPC is doing. Kind identifies the goal; phase tracks lifecycle
/// progress; target identifies the destination.
/// `recover_until` is meaningful only for Heal (HP threshold).
/// `reason` and `last_frame` are debug-only: why and when the last transition happened.
#[derive(Component, Clone, Copy, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub struct Activity {
    pub kind: ActivityKind,
    pub phase: ActivityPhase,
    pub target: ActivityTarget,
    pub ticks_waiting: u32,
    pub recover_until: f32,
    #[reflect(ignore)]
    pub reason: &'static str,
    pub last_frame: u32,
}

impl Default for Activity {
    fn default() -> Self {
        Self {
            kind: ActivityKind::default(),
            phase: ActivityPhase::default(),
            target: ActivityTarget::default(),
            ticks_waiting: 0,
            recover_until: 0.0,
            reason: "",
            last_frame: 0,
        }
    }
}

impl Activity {
    pub fn name(&self) -> &'static str {
        self.kind.label()
    }

    /// Visual key for GPU dirty tracking (sleep icon overlay).
    /// Rest+Active shows sleep icon; all other states show normal.
    pub fn visual_key(&self) -> u8 {
        if self.kind.def().sleep_visual && self.phase == ActivityPhase::Active {
            1
        } else {
            0
        }
    }

    pub fn new(kind: ActivityKind) -> Self {
        Self {
            kind,
            ..Default::default()
        }
    }
}

/// Whether the NPC is in combat. Orthogonal to Activity — a Raiding NPC can be Fighting.
/// Activity is preserved through combat so the NPC resumes what it was doing when combat ends.
#[derive(Component, Default, Clone, Debug, PartialEq, Reflect)]
#[reflect(Component)]
pub enum CombatState {
    #[default]
    None,
    Fighting {
        origin: Vec2,
    },
    Fleeing,
}

impl CombatState {
    pub fn is_fighting(&self) -> bool {
        matches!(self, Self::Fighting { .. })
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "",
            Self::Fighting { .. } => "Fighting",
            Self::Fleeing => "Fleeing",
        }
    }
}

/// Player-forced target for DirectControl NPCs.
#[derive(Component, Clone, Reflect)]
#[reflect(Component)]
pub enum ManualTarget {
    /// Attack a specific NPC (slot index).
    Npc(usize),
    /// Attack a building at position.
    Building(Vec2),
    /// Move to position.
    Position(Vec2),
}

/// High-churn NPC boolean flags bundled into one component to avoid archetype moves.
/// Toggled at runtime by various systems. Query-friendly: `Query<&mut NpcFlags>`.
#[derive(Component, Default, Clone, Reflect)]
#[reflect(Component)]
pub struct NpcFlags {
    pub healing: bool,
    pub starving: bool,
    pub direct_control: bool,
    pub migrating: bool,
    pub at_destination: bool,
}

/// A* pathfinding waypoints. Optional — only present on NPCs with active paths.
/// CPU-authoritative: A* produces waypoints, CPU advances on arrival, GPU steers
/// to current waypoint via existing goals[] upload.
#[derive(Component, Default, Clone, Reflect)]
#[reflect(Component)]
pub struct NpcPath {
    /// Waypoints in grid coordinates (col, row).
    pub waypoints: Vec<IVec2>,
    /// Index of the next waypoint to reach.
    pub current: usize,
    /// Original world-space destination (for invalidation check).
    pub goal_world: Vec2,
    /// Cooldown (seconds) after A* failure — prevents retry thrash.
    pub path_cooldown: f32,
    /// Precomputed set of HPA chunk coords this path passes through.
    pub path_chunks: Vec<(usize, usize)>,
    /// Set when A* finds no path to the goal (e.g. walled off).
    /// Decision system uses this to trigger wall-attack fallback for raiders.
    pub path_blocked: bool,
}

/// Squad assignment for military NPCs. Optional component — only present when recruited.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct SquadId(pub i32);

/// NPC is dead and pending removal.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Dead;

// ============================================================================
// HEALTH COMPONENT
// ============================================================================

/// NPC current health. Dies when reaching 0.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Health(pub f32);

impl Default for Health {
    fn default() -> Self {
        Self(100.0)
    }
}

/// Whether this NPC uses melee or ranged attacks.
/// Used as key into CombatConfig.attacks and stored on entity for re-resolution.
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
#[reflect(Component)]
pub enum BaseAttackType {
    Melee,
    Ranged,
}

/// Cached resolved combat stats. Populated on spawn from resolve_combat_stats().
/// Re-resolved on upgrade purchase or level-up (Stage 9+).
#[derive(Component, Clone, Debug, Reflect)]
#[reflect(Component)]
pub struct CachedStats {
    pub damage: f32,
    pub range: f32,
    pub cooldown: f32,
    pub projectile_speed: f32,
    pub projectile_lifetime: f32,
    pub max_health: f32,
    pub speed: f32,
    pub stamina: f32,
    pub hp_regen: f32,
    pub berserk_bonus: f32, // damage multiplier when HP <50% (from Ferocity axis)
}

// ============================================================================
// COMBAT COMPONENTS
// ============================================================================

/// Faction ID determines hostility. NPCs attack different factions.
/// GPU uses this for targeting queries (simple != comparison).
/// Convention: 0 = player, 1+ = AI/raider factions (each unique)
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Reflect)]
#[reflect(Component)]
pub struct Faction(pub i32);

impl Faction {
    pub fn to_i32(self) -> i32 {
        self.0
    }

    pub fn from_i32(v: i32) -> Self {
        Self(v)
    }
}

/// Cooldown timer for attacks. When > 0, NPC can't attack.
#[derive(Component, Default, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct AttackTimer(pub f32);

// ============================================================================
// NPC PROGRESSION
// ============================================================================

/// Per-NPC progression stats. Mirrors TowerBuildingState for buildings.
/// Level derived via level_from_xp(xp) — not stored.
/// Replaces NpcMetaCache sidecar — all other fields live on existing components
/// (Job, TownId, Personality::trait_summary()).
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct NpcStats {
    pub name: String,
    pub xp: i32,
}

/// Per-NPC skill proficiencies (0.0-100.0). Grow from doing work.
#[derive(Component, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct NpcSkills {
    pub farming: f32,
    pub combat: f32,
    pub dodge: f32,
}

// ============================================================================
// STEALING / EQUIPMENT COMPONENTS
// ============================================================================

/// Spawn-only marker: NPC can steal from farms.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct Stealer;

/// Spawn-only marker: NPC has energy system active.
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct HasEnergy;

/// Equipment rendering layer index.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
#[repr(u8)]
pub enum EquipLayer {
    Armor = 0,
    Helmet = 1,
    Weapon = 2,
    Item = 3,
    Status = 4,
    Healing = 5,
}
impl EquipLayer {
    pub const COUNT: usize = 6;
}

/// Unified equipment component — always present on NPCs with equip_slots.
/// Replaces EquippedWeapon, EquippedArmor, EquippedHelmet.
#[derive(Component, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct NpcEquipment {
    pub helm: Option<crate::constants::LootItem>,
    pub armor: Option<crate::constants::LootItem>,
    pub weapon: Option<crate::constants::LootItem>,
    pub shield: Option<crate::constants::LootItem>,
    pub gloves: Option<crate::constants::LootItem>,
    pub boots: Option<crate::constants::LootItem>,
    pub belt: Option<crate::constants::LootItem>,
    pub amulet: Option<crate::constants::LootItem>,
    pub ring1: Option<crate::constants::LootItem>,
    pub ring2: Option<crate::constants::LootItem>,
}

impl NpcEquipment {
    /// Weapon sprite: loot item sprite → sentinel.
    pub fn weapon_sprite(&self) -> (f32, f32) {
        self.weapon
            .as_ref()
            .map(|i| i.sprite)
            .unwrap_or((-1.0, 0.0))
    }

    /// Helm sprite: loot item sprite → sentinel.
    pub fn helm_sprite(&self) -> (f32, f32) {
        self.helm.as_ref().map(|i| i.sprite).unwrap_or((-1.0, 0.0))
    }

    /// Armor sprite: loot item sprite → sentinel (no NpcDef default for armor).
    pub fn armor_sprite(&self) -> (f32, f32) {
        self.armor.as_ref().map(|i| i.sprite).unwrap_or((-1.0, 0.0))
    }

    /// Shield sprite: loot item sprite → sentinel.
    pub fn shield_sprite(&self) -> (f32, f32) {
        self.shield
            .as_ref()
            .map(|i| i.sprite)
            .unwrap_or((-1.0, 0.0))
    }

    /// Get slot by enum.
    pub fn slot(&self, kind: crate::constants::ItemKind) -> &Option<crate::constants::LootItem> {
        use crate::constants::ItemKind::*;
        match kind {
            Helm => &self.helm,
            Armor => &self.armor,
            Weapon => &self.weapon,
            Shield => &self.shield,
            Gloves => &self.gloves,
            Boots => &self.boots,
            Belt => &self.belt,
            Amulet => &self.amulet,
            Ring => &self.ring1,
            Food | Gold => {
                const NONE: Option<crate::constants::LootItem> = None;
                &NONE
            }
        }
    }

    /// Get mutable slot by enum.
    pub fn slot_mut(
        &mut self,
        kind: crate::constants::ItemKind,
    ) -> &mut Option<crate::constants::LootItem> {
        use crate::constants::ItemKind::*;
        match kind {
            Helm => &mut self.helm,
            Armor => &mut self.armor,
            Weapon => &mut self.weapon,
            Shield => &mut self.shield,
            Gloves => &mut self.gloves,
            Boots => &mut self.boots,
            Belt => &mut self.belt,
            Amulet => &mut self.amulet,
            Ring => &mut self.ring1,
            Food | Gold => unreachable!("slot_mut called with resource ItemKind"),
        }
    }

    fn item_bonus(item: &Option<crate::constants::LootItem>) -> f32 {
        item.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0)
    }

    /// Total damage bonus from weapon + gloves + half(amulet + rings).
    pub fn total_weapon_bonus(&self) -> f32 {
        Self::item_bonus(&self.weapon)
            + Self::item_bonus(&self.gloves)
            + 0.5
                * (Self::item_bonus(&self.amulet)
                    + Self::item_bonus(&self.ring1)
                    + Self::item_bonus(&self.ring2))
    }

    /// Total HP bonus from armor + helm + shield + half(amulet + rings).
    pub fn total_armor_bonus(&self) -> f32 {
        Self::item_bonus(&self.armor)
            + Self::item_bonus(&self.helm)
            + Self::item_bonus(&self.shield)
            + 0.5
                * (Self::item_bonus(&self.amulet)
                    + Self::item_bonus(&self.ring1)
                    + Self::item_bonus(&self.ring2))
    }

    /// Speed bonus from boots.
    pub fn total_speed_bonus(&self) -> f32 {
        Self::item_bonus(&self.boots)
    }

    /// Stamina bonus from belt.
    pub fn total_stamina_bonus(&self) -> f32 {
        Self::item_bonus(&self.belt)
    }

    /// Iterate all equipped items (cloned) for death-drop transfer.
    pub fn all_items(&self) -> impl Iterator<Item = crate::constants::LootItem> + '_ {
        [
            &self.helm,
            &self.armor,
            &self.weapon,
            &self.shield,
            &self.gloves,
            &self.boots,
            &self.belt,
            &self.amulet,
            &self.ring1,
            &self.ring2,
        ]
        .into_iter()
        .filter_map(|slot| slot.clone())
    }
}

/// Tracks the NPC/building slot of the last attacker (for XP on kill).
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct LastHitBy(pub i32);

// Healing, Starving, Migrating, DirectControl, AtDestination → NpcFlags component
// SquadId → optional ECS component

/// Marker: entity is a building (not a walking NPC).
/// Buildings are NPC-like entities with Speed(0.0) on the building atlas.
/// They share GPU slots, EntityMap registration, and the death pipeline with NPCs.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct Building {
    pub kind: crate::world::BuildingKind,
}

/// Marker: entity is static and excluded from per-frame CPU building systems.
/// Applied to density-spawned trees/rocks that never change state until an NPC
/// claims them as a worksite (which removes Sleeping).
#[derive(Component)]
pub struct Sleeping;

/// Marker: farm is visually Ready (food icon overlay).
#[derive(Component, Reflect)]
#[reflect(Component)]
pub struct FarmReadyMarker {
    pub farm_slot: usize,
}

// ============================================================================
// BUILDING STATE COMPONENTS (CRD pattern: runtime state on ECS entities)
// ============================================================================

/// Farm production mode: crops (daytime-only, farmer-tended) or cows (autonomous, food cost).
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug, Reflect)]
pub enum FarmMode {
    #[default]
    Crops,
    Cows,
}

/// Per-farm mode component. Defaults to Crops.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct FarmModeComp(pub FarmMode);

/// Production cycle for worksites (farms grow food, mines extract gold).
/// Replaces BuildingInstance.growth_ready/growth_progress.
/// Occupants tracked by EntityMap (worksite claim counter).
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct ProductionState {
    pub ready: bool,
    pub progress: f32,
}

impl ProductionState {
    /// Take yield from a ready worksite. Resets progress, returns yield amount.
    /// Farm mode affects cow yield. One-shot worksites return yield once.
    pub fn take_yield(&mut self, kind: crate::world::BuildingKind, mode: FarmMode) -> i32 {
        if !self.ready {
            return 0;
        }
        self.ready = false;
        self.progress = 0.0;
        match kind {
            crate::world::BuildingKind::Farm => match mode {
                FarmMode::Crops => 1,
                FarmMode::Cows => crate::constants::COW_HARVEST_YIELD,
            },
            crate::world::BuildingKind::GoldMine => crate::constants::MINE_EXTRACT_PER_CYCLE,
            crate::world::BuildingKind::TreeNode => 1,
            crate::world::BuildingKind::RockNode => 1,
            _ => 0,
        }
    }

    /// Log message for a harvest event.
    pub fn yield_log_msg(kind: crate::world::BuildingKind, pos: Vec2, yield_amount: i32) -> String {
        match kind {
            crate::world::BuildingKind::Farm => {
                format!("Farm harvested at ({:.0},{:.0})", pos.x, pos.y)
            }
            crate::world::BuildingKind::GoldMine => {
                format!("Mine harvested ({} gold)", yield_amount)
            }
            crate::world::BuildingKind::TreeNode => {
                format!("Tree harvested at ({:.0},{:.0})", pos.x, pos.y)
            }
            crate::world::BuildingKind::RockNode => {
                format!("Rock harvested at ({:.0},{:.0})", pos.x, pos.y)
            }
            _ => String::new(),
        }
    }
}

/// Spawner building state (homes that produce NPCs).
/// Replaces BuildingInstance.npc_uid/respawn_timer.
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct SpawnerState {
    pub npc_slot: Option<usize>,
    pub respawn_timer: f32,
}

/// Tower/Fountain per-building combat stats.
/// Replaces BuildingInstance.kills/xp/upgrade_levels/auto_upgrade_flags.
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct TowerBuildingState {
    pub kills: i32,
    pub xp: i32,
    pub upgrade_levels: Vec<u8>,
    pub auto_upgrade_flags: Vec<bool>,
    /// Equipped weapon item (boosts tower damage via stat_bonus).
    pub equipped_weapon: Option<crate::constants::LootItem>,
}

/// Building under construction (seconds remaining; 0 = complete).
/// Replaces BuildingInstance.under_construction.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct ConstructionProgress(pub f32);

/// Waypoint patrol order within a town.
/// Replaces BuildingInstance.patrol_order.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct WaypointOrder(pub u32);

/// Wall upgrade level.
/// Replaces BuildingInstance.wall_level.
#[derive(Component, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct WallLevel(pub u8);

/// Miner home assignment config.
/// Replaces BuildingInstance.assigned_mine/manual_mine.
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct MinerHomeConfig {
    pub assigned_mine: Option<Vec2>,
    pub manual_mine: bool,
}

// ============================================================================
// TOWN ENTITY COMPONENTS (CRD pattern: runtime state on ECS town entities)
// ============================================================================

/// Marker component for town entities.
#[derive(Component)]
pub struct TownMarker;

/// Town food storage.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct FoodStore(pub i32);

/// Town gold storage.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct GoldStore(pub i32);

/// Town wood storage.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct WoodStore(pub i32);

/// Town stone storage.
#[derive(Component, Default, Reflect)]
#[reflect(Component)]
pub struct StoneStore(pub i32);

/// Town behavior policies.
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct TownPolicy(pub crate::resources::PolicySet);

/// Town upgrade levels (dynamic size, matches upgrade_count()).
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct TownUpgradeLevel(pub Vec<u8>);

/// Town equipment inventory (unequipped items).
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct TownEquipment(pub Vec<crate::constants::LootItem>);

/// Town build-area expansion level. 0 = base 8x8, each level adds 1 ring.
#[derive(Component, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct TownAreaLevel(pub i32);

// ============================================================================
// BEHAVIOR CONFIG COMPONENTS (generic, attach to any NPC)
// ============================================================================

/// Flee combat when HP drops below this percentage.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct FleeThreshold {
    pub pct: f32,
}

/// Disengage combat if distance from Home exceeds this.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct LeashRange(pub f32);

/// Drop everything and return home when HP drops below this percentage.
/// Distinct from FleeThreshold: wounded NPCs enter recovery mode.
#[derive(Component, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct WoundedThreshold {
    pub pct: f32,
}

// ============================================================================
// PERSONALITY SYSTEM (Utility AI)
// ============================================================================

/// 7 spectrum axes. Magnitude sign determines pole (+Brave/-Coward, etc).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub enum TraitKind {
    Courage,   // +Brave / -Coward
    Diligence, // +Efficient / -Lazy
    Vitality,  // +Hardy / -Frail
    Power,     // +Strong / -Weak
    Agility,   // +Swift / -Slow
    Precision, // +Sharpshot / -Myopic
    Ferocity,  // +Berserker / -Timid
}

pub const TRAIT_COUNT: usize = 7;

impl TraitKind {
    pub const ALL: [TraitKind; TRAIT_COUNT] = [
        TraitKind::Courage,
        TraitKind::Diligence,
        TraitKind::Vitality,
        TraitKind::Power,
        TraitKind::Agility,
        TraitKind::Precision,
        TraitKind::Ferocity,
    ];

    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(TraitKind::Courage),
            1 => Some(TraitKind::Diligence),
            2 => Some(TraitKind::Vitality),
            3 => Some(TraitKind::Power),
            4 => Some(TraitKind::Agility),
            5 => Some(TraitKind::Precision),
            6 => Some(TraitKind::Ferocity),
            _ => None,
        }
    }

    /// Map old save trait IDs (0-3) to new axes.
    pub fn from_legacy_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(TraitKind::Courage),   // was Brave
            1 => Some(TraitKind::Vitality),  // was Tough
            2 => Some(TraitKind::Agility),   // was Swift
            3 => Some(TraitKind::Diligence), // was Focused
            _ => None,
        }
    }

    pub fn to_id(self) -> i32 {
        match self {
            TraitKind::Courage => 0,
            TraitKind::Diligence => 1,
            TraitKind::Vitality => 2,
            TraitKind::Power => 3,
            TraitKind::Agility => 4,
            TraitKind::Precision => 5,
            TraitKind::Ferocity => 6,
        }
    }

    /// Display name based on magnitude sign.
    pub fn name(self, magnitude: f32) -> &'static str {
        if magnitude >= 0.0 {
            match self {
                TraitKind::Courage => "Brave",
                TraitKind::Diligence => "Efficient",
                TraitKind::Vitality => "Hardy",
                TraitKind::Power => "Strong",
                TraitKind::Agility => "Swift",
                TraitKind::Precision => "Sharpshot",
                TraitKind::Ferocity => "Berserker",
            }
        } else {
            match self {
                TraitKind::Courage => "Coward",
                TraitKind::Diligence => "Lazy",
                TraitKind::Vitality => "Frail",
                TraitKind::Power => "Weak",
                TraitKind::Agility => "Slow",
                TraitKind::Precision => "Myopic",
                TraitKind::Ferocity => "Timid",
            }
        }
    }
}

/// A trait axis with signed magnitude (-1.5..+1.5). Sign = pole, abs = strength.
#[derive(Clone, Copy, Debug, Reflect)]
pub struct TraitInstance {
    pub kind: TraitKind,
    pub magnitude: f32,
}

/// Stat modifiers computed from personality traits.
pub struct TraitStatMods {
    pub damage: f32,
    pub hp: f32,
    pub speed: f32,
    pub work_yield: f32,
    pub range: f32,
    pub cooldown: f32,
    pub berserk_bonus: f32, // applied when HP <50%: damage *= (1 + berserk_bonus)
}

impl Default for TraitStatMods {
    fn default() -> Self {
        Self {
            damage: 1.0,
            hp: 1.0,
            speed: 1.0,
            work_yield: 1.0,
            range: 1.0,
            cooldown: 1.0,
            berserk_bonus: 0.0,
        }
    }
}

/// Behavior modifiers computed from personality traits.
pub struct TraitBehaviorMods {
    pub fight: f32,
    pub flee: f32,
    pub rest: f32,
    pub eat: f32,
    pub work: f32,
    pub wander: f32,
    pub never_flees: bool,
    pub flee_threshold_add: f32,
}

impl Default for TraitBehaviorMods {
    fn default() -> Self {
        Self {
            fight: 1.0,
            flee: 1.0,
            rest: 1.0,
            eat: 1.0,
            work: 1.0,
            wander: 1.0,
            never_flees: false,
            flee_threshold_add: 0.0,
        }
    }
}

/// NPC personality: 0-2 spectrum traits that modify stats and decision weights.
#[derive(Component, Clone, Debug, Default, Reflect)]
#[reflect(Component)]
pub struct Personality {
    pub trait1: Option<TraitInstance>,
    pub trait2: Option<TraitInstance>,
}

impl Personality {
    /// Human-readable trait summary for UI (0-2 traits).
    pub fn trait_summary(&self) -> String {
        let mut names: Vec<&'static str> = Vec::new();
        if let Some(t) = self.trait1 {
            names.push(t.kind.name(t.magnitude));
        }
        if let Some(t) = self.trait2 {
            names.push(t.kind.name(t.magnitude));
        }
        names.join(" + ")
    }

    /// Compute behavior modifiers from traits.
    pub fn get_behavior_mods(&self) -> TraitBehaviorMods {
        let mut mods = TraitBehaviorMods::default();
        for t in [self.trait1, self.trait2].iter().flatten() {
            let m = t.magnitude;
            let a = m.abs();
            match t.kind {
                TraitKind::Courage => {
                    if m > 0.0 {
                        mods.never_flees = true;
                    } else {
                        mods.flee_threshold_add += 0.20 * a;
                    }
                }
                TraitKind::Diligence => {
                    if m > 0.0 {
                        mods.work *= 1.0 + a;
                    } else {
                        mods.work *= 1.0 / (1.0 + a);
                        mods.wander *= 1.0 + a;
                    }
                }
                TraitKind::Vitality => {
                    if m > 0.0 {
                        mods.rest *= 1.0 / (1.0 + a);
                        mods.eat *= 1.0 / (1.0 + a);
                    } else {
                        mods.rest *= 1.0 + a;
                        mods.eat *= 1.0 + a;
                    }
                }
                TraitKind::Power => {
                    if m > 0.0 {
                        mods.fight *= 1.0 + a;
                    } else {
                        mods.fight *= 1.0 / (1.0 + a);
                    }
                }
                TraitKind::Agility => {
                    if m > 0.0 {
                        mods.wander *= 1.0 + a;
                    } else {
                        mods.wander *= 1.0 / (1.0 + a);
                    }
                }
                TraitKind::Precision => {} // no behavior effect
                TraitKind::Ferocity => {
                    if m > 0.0 {
                        mods.fight *= 1.0 + a;
                        mods.flee *= 1.0 / (1.0 + a);
                    } else {
                        mods.fight *= 1.0 / (1.0 + a);
                        mods.flee *= 1.0 + a;
                    }
                }
            }
        }
        mods
    }

    /// Compute stat modifiers from traits.
    pub fn get_stat_mods(&self) -> TraitStatMods {
        let mut mods = TraitStatMods::default();
        for t in [self.trait1, self.trait2].iter().flatten() {
            let m = t.magnitude;
            match t.kind {
                TraitKind::Courage => {} // no stat effect
                TraitKind::Diligence => {
                    mods.work_yield *= 1.0 + 0.25 * m;
                    mods.cooldown *= 1.0 - 0.25 * m;
                }
                TraitKind::Vitality => {
                    mods.hp *= 1.0 + 0.25 * m;
                }
                TraitKind::Power => {
                    mods.damage *= 1.0 + 0.25 * m;
                }
                TraitKind::Agility => {
                    mods.speed *= 1.0 + 0.25 * m;
                }
                TraitKind::Precision => {
                    mods.range *= 1.0 + 0.25 * m;
                }
                TraitKind::Ferocity => {
                    mods.berserk_bonus += 0.50 * m;
                }
            }
        }
        mods
    }
}

// ============================================================================
// UNIT TESTS
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn neutral() -> Personality {
        Personality::default()
    }

    fn with_trait(kind: TraitKind, magnitude: f32) -> Personality {
        Personality {
            trait1: Some(TraitInstance { kind, magnitude }),
            trait2: None,
        }
    }

    // -- Neutral personality (no traits) -------------------------------------

    #[test]
    fn neutral_stat_mods_all_one() {
        let mods = neutral().get_stat_mods();
        assert_eq!(mods.damage, 1.0);
        assert_eq!(mods.hp, 1.0);
        assert_eq!(mods.speed, 1.0);
        assert_eq!(mods.range, 1.0);
        assert_eq!(mods.cooldown, 1.0);
        assert_eq!(mods.work_yield, 1.0);
        assert_eq!(mods.berserk_bonus, 0.0);
    }

    #[test]
    fn neutral_behavior_mods_all_one() {
        let mods = neutral().get_behavior_mods();
        assert_eq!(mods.fight, 1.0);
        assert_eq!(mods.flee, 1.0);
        assert_eq!(mods.rest, 1.0);
        assert_eq!(mods.eat, 1.0);
        assert_eq!(mods.work, 1.0);
        assert_eq!(mods.wander, 1.0);
        assert!(!mods.never_flees);
        assert_eq!(mods.flee_threshold_add, 0.0);
    }

    // -- Courage axis --------------------------------------------------------

    #[test]
    fn brave_never_flees() {
        let mods = with_trait(TraitKind::Courage, 1.0).get_behavior_mods();
        assert!(mods.never_flees);
    }

    #[test]
    fn coward_flees_earlier() {
        let mods = with_trait(TraitKind::Courage, -1.0).get_behavior_mods();
        assert!(!mods.never_flees);
        assert!(
            mods.flee_threshold_add > 0.0,
            "coward should increase flee threshold"
        );
    }

    #[test]
    fn courage_no_stat_effect() {
        let mods = with_trait(TraitKind::Courage, 1.0).get_stat_mods();
        assert_eq!(mods.damage, 1.0);
        assert_eq!(mods.hp, 1.0);
    }

    // -- Ferocity axis -------------------------------------------------------

    #[test]
    fn berserker_positive_berserk_bonus() {
        let mods = with_trait(TraitKind::Ferocity, 1.0).get_stat_mods();
        assert!(
            (mods.berserk_bonus - 0.5).abs() < 0.01,
            "expected 0.5, got {}",
            mods.berserk_bonus
        );
    }

    #[test]
    fn timid_negative_berserk_bonus() {
        let mods = with_trait(TraitKind::Ferocity, -1.0).get_stat_mods();
        assert!(
            (mods.berserk_bonus - (-0.5)).abs() < 0.01,
            "expected -0.5, got {}",
            mods.berserk_bonus
        );
    }

    #[test]
    fn berserker_increases_fight_decreases_flee() {
        let mods = with_trait(TraitKind::Ferocity, 1.0).get_behavior_mods();
        assert!(mods.fight > 1.0);
        assert!(mods.flee < 1.0);
    }

    // -- Power axis ----------------------------------------------------------

    #[test]
    fn strong_increases_damage() {
        let mods = with_trait(TraitKind::Power, 1.0).get_stat_mods();
        assert!(mods.damage > 1.0, "strong should increase damage");
    }

    #[test]
    fn weak_decreases_damage() {
        let mods = with_trait(TraitKind::Power, -1.0).get_stat_mods();
        assert!(mods.damage < 1.0, "weak should decrease damage");
    }

    // -- Vitality axis -------------------------------------------------------

    #[test]
    fn hardy_increases_hp() {
        let mods = with_trait(TraitKind::Vitality, 1.0).get_stat_mods();
        assert!(mods.hp > 1.0, "hardy should increase HP");
    }

    // -- Agility axis --------------------------------------------------------

    #[test]
    fn swift_increases_speed() {
        let mods = with_trait(TraitKind::Agility, 1.0).get_stat_mods();
        assert!(mods.speed > 1.0);
    }

    // -- Diligence axis ------------------------------------------------------

    #[test]
    fn efficient_increases_work_yield() {
        let mods = with_trait(TraitKind::Diligence, 1.0).get_stat_mods();
        assert!(mods.work_yield > 1.0);
    }

    #[test]
    fn efficient_increases_work_behavior() {
        let mods = with_trait(TraitKind::Diligence, 1.0).get_behavior_mods();
        assert!(mods.work > 1.0);
    }

    // -- trait_summary -------------------------------------------------------

    #[test]
    fn trait_summary_empty_for_neutral() {
        assert_eq!(neutral().trait_summary(), "");
    }

    #[test]
    fn trait_summary_one_trait() {
        let p = with_trait(TraitKind::Courage, 1.0);
        let s = p.trait_summary();
        assert!(!s.is_empty());
        assert!(!s.contains("+"), "one trait should not have separator");
    }

    #[test]
    fn trait_summary_two_traits() {
        let p = Personality {
            trait1: Some(TraitInstance {
                kind: TraitKind::Courage,
                magnitude: 1.0,
            }),
            trait2: Some(TraitInstance {
                kind: TraitKind::Power,
                magnitude: 1.0,
            }),
        };
        assert!(p.trait_summary().contains(" + "));
    }
}
