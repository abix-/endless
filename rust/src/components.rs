//! ECS Components - Bevy entities have these attached

use bevy::prelude::*;

// ============================================================================
// CORE COMPONENTS
// ============================================================================

/// Stable identity for gameplay cross-references. Monotonically increasing u64 counter.
/// Survives slot recycling — unlike GpuSlot, an EntityUid is never reused.
/// EntityUid(0) is reserved as "none/invalid".
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct EntityUid(pub u64);

/// Links a Bevy entity to its unified slot in the GPU entity buffers.
/// Both NPCs and buildings get an GpuSlot(n) where n = GPU buffer index.
/// GpuSlot is a dense GPU address, NOT a stable identity — use EntityUid for cross-references.
#[derive(Component, Clone, Copy)]
pub struct GpuSlot(pub usize);

/// NPC position in world coordinates. Bevy owns this, syncs to GPU for physics.
/// Phase 11: Replaces GPU-owned positions with Bevy-owned + GPU accelerated.
#[derive(Component, Clone, Copy)]
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
#[derive(Component, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug)]
pub enum Job {
    Farmer,
    Archer,
    Raider,
    Fighter,
    Miner,
    Crossbow,
    Boat,
}

impl Job {
    /// Convert from integer (0=Farmer, 1=Archer, 2=Raider, 3=Fighter, 4=Miner, 5=Crossbow, 6=Boat)
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Job::Archer,
            2 => Job::Raider,
            3 => Job::Fighter,
            4 => Job::Miner,
            5 => Job::Crossbow,
            6 => Job::Boat,
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
#[derive(Component, Clone, Copy)]
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
#[derive(Component, Clone, Copy)]
pub struct TownId(pub i32);

// ============================================================================
// BEHAVIOR DATA COMPONENTS
// ============================================================================

/// NPC energy level (0-100). Drains while active, recovers while resting.
#[derive(Component, Clone, Copy)]
pub struct Energy(pub f32);

impl Default for Energy {
    fn default() -> Self {
        Self(100.0)
    }
}

/// Where the NPC goes to rest (bed position).
/// Home(-1, -1) means no home assigned — behavior systems should skip.
#[derive(Component, Clone, Copy)]
pub struct Home(pub Vec2);

impl Home {
    pub fn is_valid(&self) -> bool {
        self.0.x >= 0.0 && self.0.y >= 0.0
    }
}

/// Patrol route for guards (or any NPC that patrols).
#[derive(Component, Clone)]
pub struct PatrolRoute {
    pub posts: Vec<Vec2>,
    pub current: usize,
}

/// Combined work state for NPCs. Always present — avoids archetype churn from insert/remove.
/// `occupied_building`: UID of building being occupied (released on death/stop via entity_map.release).
/// `work_target_building`: UID of building being walked to (navigation target).
#[derive(Component, Default, Clone, Copy)]
pub struct NpcWorkState {
    pub occupied_building: Option<EntityUid>,
    pub work_target_building: Option<EntityUid>,
}

/// Unified carry component for ALL NPCs. Always present — replaces the old fragmented
/// Activity::Returning{loot} payload + CarriedGold component.
/// Loot lives here; Activity::Returning just means "going home."
#[derive(Component, Default, Clone)]
pub struct CarriedLoot {
    pub food: i32,
    pub gold: i32,
    pub equipment: Vec<crate::constants::LootItem>,
}

impl CarriedLoot {
    pub fn is_empty(&self) -> bool {
        self.food <= 0 && self.gold <= 0 && self.equipment.is_empty()
    }

    pub fn total_items(&self) -> usize {
        (self.food > 0) as usize + (self.gold > 0) as usize + self.equipment.len()
    }

    /// Visual key for GPU dirty tracking: same key = same visual overlay.
    pub fn visual_key(&self) -> u8 {
        if !self.equipment.is_empty() {
            4
        } else if self.gold > 0 {
            2
        } else if self.food > 0 {
            3
        } else {
            0
        }
    }
}

// ============================================================================
// NPC STATE — Two orthogonal enums (Activity × CombatState)
// ============================================================================

/// What the NPC is *doing*. Mutually exclusive — an NPC is in exactly one activity.
/// Transit variants (Patrolling, GoingToWork, GoingToRest, GoingToHeal, Wandering, Raiding, Returning)
/// mean the NPC is moving toward a destination; use `is_transit()` to check.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub enum Activity {
    #[default]
    Idle,
    Working,
    OnDuty {
        ticks_waiting: u32,
    },
    Patrolling,
    GoingToWork,
    GoingToRest,
    Resting,
    GoingToHeal,
    HealingAtFountain {
        recover_until: f32,
    },
    Wandering,
    Raiding {
        target: Vec2,
    },
    Returning,
    Mining {
        mine_pos: Vec2,
    },
    MiningAtMine,
}

impl Activity {
    /// Is this NPC moving toward a destination?
    pub fn is_transit(&self) -> bool {
        matches!(
            self,
            Self::Patrolling
                | Self::GoingToWork
                | Self::GoingToRest
                | Self::GoingToHeal
                | Self::Wandering
                | Self::Raiding { .. }
                | Self::Returning
                | Self::Mining { .. }
        )
    }

    /// Visual key for dirty tracking: same key = same visual representation.
    /// Only Resting affects rendered overlays from Activity. Carried-item overlays
    /// are tracked by CarriedLoot::visual_key() instead.
    pub fn visual_key(&self) -> u8 {
        match self {
            Self::Resting => 1,
            _ => 0,
        }
    }

    /// Display name for UI/debug.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::Working => "Working",
            Self::OnDuty { .. } => "On Duty",
            Self::Patrolling => "Patrolling",
            Self::GoingToWork => "Going to Work",
            Self::GoingToRest => "Going to Rest",
            Self::Resting => "Resting",
            Self::GoingToHeal => "Going to Heal",
            Self::HealingAtFountain { .. } => "Healing",
            Self::Wandering => "Wandering",
            Self::Raiding { .. } => "Raiding",
            Self::Returning => "Returning",
            Self::Mining { .. } => "Mining",
            Self::MiningAtMine => "Mining",
        }
    }
}

/// Whether the NPC is in combat. Orthogonal to Activity — a Raiding NPC can be Fighting.
/// Activity is preserved through combat so the NPC resumes what it was doing when combat ends.
#[derive(Component, Default, Clone, Debug, PartialEq)]
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
#[derive(Component, Clone)]
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
#[derive(Component, Default, Clone)]
pub struct NpcFlags {
    pub healing: bool,
    pub starving: bool,
    pub direct_control: bool,
    pub migrating: bool,
    pub at_destination: bool,
}

/// Squad assignment for military NPCs. Optional component — only present when recruited.
#[derive(Component, Clone, Copy)]
pub struct SquadId(pub i32);

/// NPC is dead and pending removal.
#[derive(Component)]
pub struct Dead;

// ============================================================================
// HEALTH COMPONENT
// ============================================================================

/// NPC current health. Dies when reaching 0.
#[derive(Component)]
pub struct Health(pub f32);

impl Default for Health {
    fn default() -> Self {
        Self(100.0)
    }
}

/// Whether this NPC uses melee or ranged attacks.
/// Used as key into CombatConfig.attacks and stored on entity for re-resolution.
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum BaseAttackType {
    Melee,
    Ranged,
}

/// Cached resolved combat stats. Populated on spawn from resolve_combat_stats().
/// Re-resolved on upgrade purchase or level-up (Stage 9+).
#[derive(Component, Clone, Debug)]
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
}

// ============================================================================
// COMBAT COMPONENTS
// ============================================================================

/// Faction ID determines hostility. NPCs attack different factions.
/// GPU uses this for targeting queries (simple != comparison).
/// Convention: 0 = player, 1+ = AI/raider factions (each unique)
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Component, Default, Clone, Copy)]
pub struct AttackTimer(pub f32);

// ============================================================================
// STEALING / EQUIPMENT COMPONENTS
// ============================================================================

/// Spawn-only marker: NPC can steal from farms.
#[derive(Component)]
pub struct Stealer;

/// Spawn-only marker: NPC has energy system active.
#[derive(Component)]
pub struct HasEnergy;

/// Equipment rendering layer index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
#[derive(Component, Clone, Default, serde::Serialize, serde::Deserialize)]
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
    /// Weapon sprite: loot item sprite → NpcDef default weapon → sentinel.
    pub fn weapon_sprite(&self, job: Job) -> (f32, f32) {
        self.weapon
            .as_ref()
            .map(|i| i.sprite)
            .or(crate::constants::npc_def(job).weapon)
            .unwrap_or((-1.0, 0.0))
    }

    /// Helm sprite: loot item sprite → NpcDef default helmet → sentinel.
    pub fn helm_sprite(&self, job: Job) -> (f32, f32) {
        self.helm
            .as_ref()
            .map(|i| i.sprite)
            .or(crate::constants::npc_def(job).helmet)
            .unwrap_or((-1.0, 0.0))
    }

    /// Armor sprite: loot item sprite → sentinel (no NpcDef default for armor).
    pub fn armor_sprite(&self) -> (f32, f32) {
        self.armor
            .as_ref()
            .map(|i| i.sprite)
            .unwrap_or((-1.0, 0.0))
    }

    /// Shield sprite: loot item sprite → sentinel.
    pub fn shield_sprite(&self) -> (f32, f32) {
        self.shield
            .as_ref()
            .map(|i| i.sprite)
            .unwrap_or((-1.0, 0.0))
    }

    /// Get slot by enum.
    pub fn slot(&self, slot: crate::constants::EquipmentSlot) -> &Option<crate::constants::LootItem> {
        use crate::constants::EquipmentSlot::*;
        match slot {
            Helm => &self.helm,
            Armor => &self.armor,
            Weapon => &self.weapon,
            Shield => &self.shield,
            Gloves => &self.gloves,
            Boots => &self.boots,
            Belt => &self.belt,
            Amulet => &self.amulet,
            Ring => &self.ring1,
        }
    }

    /// Get mutable slot by enum.
    pub fn slot_mut(&mut self, slot: crate::constants::EquipmentSlot) -> &mut Option<crate::constants::LootItem> {
        use crate::constants::EquipmentSlot::*;
        match slot {
            Helm => &mut self.helm,
            Armor => &mut self.armor,
            Weapon => &mut self.weapon,
            Shield => &mut self.shield,
            Gloves => &mut self.gloves,
            Boots => &mut self.boots,
            Belt => &mut self.belt,
            Amulet => &mut self.amulet,
            Ring => &mut self.ring1,
        }
    }

    fn item_bonus(item: &Option<crate::constants::LootItem>) -> f32 {
        item.as_ref().map(|i| i.stat_bonus).unwrap_or(0.0)
    }

    /// Total damage bonus from weapon + gloves + half(amulet + rings).
    pub fn total_weapon_bonus(&self) -> f32 {
        Self::item_bonus(&self.weapon)
            + Self::item_bonus(&self.gloves)
            + 0.5 * (Self::item_bonus(&self.amulet) + Self::item_bonus(&self.ring1) + Self::item_bonus(&self.ring2))
    }

    /// Total HP bonus from armor + helm + shield + half(amulet + rings).
    pub fn total_armor_bonus(&self) -> f32 {
        Self::item_bonus(&self.armor)
            + Self::item_bonus(&self.helm)
            + Self::item_bonus(&self.shield)
            + 0.5 * (Self::item_bonus(&self.amulet) + Self::item_bonus(&self.ring1) + Self::item_bonus(&self.ring2))
    }

    /// Speed bonus from boots.
    pub fn total_speed_bonus(&self) -> f32 {
        Self::item_bonus(&self.boots)
    }

    /// Stamina bonus from belt.
    pub fn total_stamina_bonus(&self) -> f32 {
        Self::item_bonus(&self.belt)
    }
}

/// Tracks the NPC/building slot of the last attacker (for XP on kill).
#[derive(Component)]
pub struct LastHitBy(pub i32);

// Healing, Starving, Migrating, DirectControl, AtDestination → NpcFlags component
// SquadId → optional ECS component

/// Marker: entity is a building (not a walking NPC).
/// Buildings are NPC-like entities with Speed(0.0) on the building atlas.
/// They share GPU slots, EntityMap registration, and the death pipeline with NPCs.
#[derive(Component, Clone, Copy)]
pub struct Building {
    pub kind: crate::world::BuildingKind,
}

/// Marker: farm is visually Ready (food icon overlay).
#[derive(Component)]
pub struct FarmReadyMarker {
    pub farm_slot: usize,
}

// ============================================================================
// BEHAVIOR CONFIG COMPONENTS (generic, attach to any NPC)
// ============================================================================

/// Flee combat when HP drops below this percentage.
#[derive(Component, Clone, Copy)]
pub struct FleeThreshold {
    pub pct: f32,
}

/// Disengage combat if distance from Home exceeds this.
#[derive(Component, Clone, Copy)]
pub struct LeashRange(pub f32);

/// Drop everything and return home when HP drops below this percentage.
/// Distinct from FleeThreshold: wounded NPCs enter recovery mode.
#[derive(Component, Clone, Copy)]
pub struct WoundedThreshold {
    pub pct: f32,
}

// ============================================================================
// PERSONALITY SYSTEM (Utility AI)
// ============================================================================

/// Trait types that affect both stats and behavior weights.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TraitKind {
    /// +damage, fights more, flees less
    Brave,
    /// +HP, rests/eats less (pushes through)
    Tough,
    /// +speed, wanders more
    Swift,
    /// +yield, works more, wanders less
    Focused,
}

impl TraitKind {
    pub fn from_id(id: i32) -> Option<Self> {
        match id {
            0 => Some(TraitKind::Brave),
            1 => Some(TraitKind::Tough),
            2 => Some(TraitKind::Swift),
            3 => Some(TraitKind::Focused),
            _ => None,
        }
    }

    pub fn to_id(self) -> i32 {
        match self {
            TraitKind::Brave => 0,
            TraitKind::Tough => 1,
            TraitKind::Swift => 2,
            TraitKind::Focused => 3,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TraitKind::Brave => "Brave",
            TraitKind::Tough => "Tough",
            TraitKind::Swift => "Swift",
            TraitKind::Focused => "Focused",
        }
    }
}

/// A trait with its magnitude (0.5 = weak, 1.0 = normal, 1.5 = strong).
#[derive(Clone, Copy, Debug)]
pub struct TraitInstance {
    pub kind: TraitKind,
    pub magnitude: f32,
}

/// NPC personality: 0-2 traits that modify stats and decision weights.
#[derive(Component, Clone, Debug, Default)]
pub struct Personality {
    pub trait1: Option<TraitInstance>,
    pub trait2: Option<TraitInstance>,
}

impl Personality {
    /// Human-readable trait summary for UI (0-2 traits).
    pub fn trait_summary(&self) -> String {
        let mut names: Vec<&'static str> = Vec::new();
        if let Some(t) = self.trait1 {
            names.push(t.kind.name());
        }
        if let Some(t) = self.trait2 {
            names.push(t.kind.name());
        }
        names.join(" + ")
    }

    /// Get behavior multipliers: (fight, flee, rest, eat, work, wander)
    pub fn get_multipliers(&self) -> (f32, f32, f32, f32, f32, f32) {
        let mut fight = 1.0;
        let mut flee = 1.0;
        let mut rest = 1.0;
        let mut eat = 1.0;
        let mut work = 1.0;
        let mut wander = 1.0;

        for t in [self.trait1, self.trait2].iter().flatten() {
            let m = t.magnitude;
            match t.kind {
                TraitKind::Brave => {
                    fight *= 1.0 + m;
                    flee *= 1.0 / (1.0 + m);
                }
                TraitKind::Tough => {
                    rest *= 1.0 / (1.0 + m);
                    eat *= 1.0 / (1.0 + m);
                }
                TraitKind::Swift => {
                    wander *= 1.0 + m;
                }
                TraitKind::Focused => {
                    work *= 1.0 + m;
                    wander *= 1.0 / (1.0 + m);
                }
            }
        }

        (fight, flee, rest, eat, work, wander)
    }

    /// Get stat multipliers: (damage, hp, speed, yield)
    pub fn get_stat_multipliers(&self) -> (f32, f32, f32, f32) {
        let mut damage = 1.0;
        let mut hp = 1.0;
        let mut speed = 1.0;
        let mut work_yield = 1.0;

        for t in [self.trait1, self.trait2].iter().flatten() {
            let bonus = 0.25 * t.magnitude;
            match t.kind {
                TraitKind::Brave => damage += bonus,
                TraitKind::Tough => hp += bonus,
                TraitKind::Swift => speed += bonus,
                TraitKind::Focused => work_yield += bonus,
            }
        }

        (damage, hp, speed, work_yield)
    }
}
