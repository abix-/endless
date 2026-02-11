//! ECS Components - Bevy entities have these attached

use bevy::prelude::*;

// ============================================================================
// CORE COMPONENTS
// ============================================================================

/// Links a Bevy entity to its index in the GPU buffers.
/// When spawning an NPC, we create an entity with NpcIndex(n) where n is the buffer slot.
#[derive(Component, Clone, Copy)]
pub struct NpcIndex(pub usize);

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
/// - Guard (blue): patrols and fights raiders
/// - Raider (red): attacks guards, steals from farms
#[derive(Component, Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Job {
    Farmer,
    Guard,
    Raider,
    Fighter,
}

impl Job {
    /// Convert from GDScript integer (0=Farmer, 1=Guard, 2=Raider, 3=Fighter)
    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Job::Guard,
            2 => Job::Raider,
            3 => Job::Fighter,
            _ => Job::Farmer,
        }
    }

    /// RGBA color for this job type. Alpha=1.0 means "has target" on GPU.
    pub fn color(&self) -> (f32, f32, f32, f32) {
        match self {
            Job::Farmer => (0.4, 1.0, 0.4, 1.0),  // Green tint
            Job::Guard => (0.3, 0.3, 1.0, 1.0),   // Blue tint
            Job::Raider => (1.0, 0.4, 0.4, 1.0),  // Red tint
            Job::Fighter => (1.0, 1.0, 0.4, 1.0), // Yellow tint
        }
    }
}

/// Movement speed in pixels per second.
#[derive(Component, Clone, Copy)]
pub struct Speed(pub f32);

impl Default for Speed {
    fn default() -> Self {
        Self(100.0)  // 100 pixels/second base speed
    }
}

// ============================================================================
// NPC TYPE MARKERS
// ============================================================================

/// Guard marker - identifies NPC as a guard (for queries).
#[derive(Component)]
pub struct Guard;

/// Farmer marker - identifies NPC as a farmer.
#[derive(Component)]
pub struct Farmer;

/// TownId identifies which town an NPC belongs to.
/// Universal component on every NPC. All settlements are "towns" (villager or raider).
#[derive(Component, Clone, Copy)]
pub struct TownId(pub i32);

// ============================================================================
// BEHAVIOR DATA COMPONENTS
// ============================================================================

/// NPC energy level (0-100). Drains while active, recovers while resting.
#[derive(Component)]
pub struct Energy(pub f32);

impl Default for Energy {
    fn default() -> Self {
        Self(100.0)
    }
}

/// Where the NPC goes to rest (bed position).
/// Home(-1, -1) means no home assigned — behavior systems should skip.
#[derive(Component)]
pub struct Home(pub Vec2);

impl Home {
    pub fn is_valid(&self) -> bool {
        self.0.x >= 0.0 && self.0.y >= 0.0
    }
}

/// Patrol route for guards (or any NPC that patrols).
#[derive(Component)]
pub struct PatrolRoute {
    pub posts: Vec<Vec2>,
    pub current: usize,
}

/// Work position for farmers (or any NPC that works at a location).
#[derive(Component)]
pub struct WorkPosition(pub Vec2);

// ============================================================================
// NPC STATE — Two orthogonal enums (Activity × CombatState)
// ============================================================================

/// What the NPC is *doing*. Mutually exclusive — an NPC is in exactly one activity.
/// Transit variants (Patrolling, GoingToWork, GoingToRest, Wandering, Raiding, Returning)
/// mean the NPC is moving toward a destination; use `is_transit()` to check.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub enum Activity {
    #[default]
    Idle,
    Working,
    OnDuty { ticks_waiting: u32 },
    Patrolling,
    GoingToWork,
    GoingToRest,
    Resting { recover_until: Option<f32> },
    Wandering,
    Raiding { target: Vec2 },
    Returning { has_food: bool },
}

impl Activity {
    /// Is this NPC moving toward a destination?
    pub fn is_transit(&self) -> bool {
        matches!(self, Self::Patrolling | Self::GoingToWork | Self::GoingToRest
            | Self::Wandering | Self::Raiding { .. } | Self::Returning { .. })
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
            Self::Resting { recover_until: Some(_) } => "Recovering",
            Self::Resting { recover_until: None } => "Resting",
            Self::Wandering => "Wandering",
            Self::Raiding { .. } => "Raiding",
            Self::Returning { has_food: true } => "Returning (food)",
            Self::Returning { has_food: false } => "Returning",
        }
    }
}

/// Whether the NPC is in combat. Orthogonal to Activity — a Raiding NPC can be Fighting.
/// Activity is preserved through combat so the NPC resumes what it was doing when combat ends.
#[derive(Component, Default, Clone, Debug, PartialEq)]
pub enum CombatState {
    #[default]
    None,
    Fighting { origin: Vec2 },
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

/// Farmer's assigned farm position for occupancy tracking.
/// Added when entering Working at a farm, removed when leaving.
/// Stores position (not index) so buildings can be deleted without breaking refs.
#[derive(Component)]
pub struct AssignedFarm(pub Vec2);

/// NPC has arrived at destination and needs transition handling.
/// Set by gpu_position_readback when within ARRIVAL_THRESHOLD; cleared by decision_system.
#[derive(Component)]
pub struct AtDestination;

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
#[derive(Component, Default)]
pub struct AttackTimer(pub f32);

// ============================================================================
// STEALING / EQUIPMENT COMPONENTS
// ============================================================================

/// Marker: this NPC steals food from farms. Any NPC with this + Home
/// will use the steal decision system.
#[derive(Component)]
pub struct Stealer;

/// Equipment rendering layer index.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum EquipLayer { Armor = 0, Helmet = 1, Weapon = 2, Item = 3, Status = 4, Healing = 5 }
impl EquipLayer { pub const COUNT: usize = 6; }

/// Equipped weapon sprite (col, row in atlas). Presence = has weapon.
#[derive(Component, Clone, Copy)]
pub struct EquippedWeapon(pub f32, pub f32);

/// Equipped helmet sprite (col, row in atlas). Presence = has helmet.
#[derive(Component, Clone, Copy)]
pub struct EquippedHelmet(pub f32, pub f32);

/// Equipped armor sprite (col, row in atlas). Presence = has armor.
#[derive(Component, Clone, Copy)]
pub struct EquippedArmor(pub f32, pub f32);

/// Tracks the NPC slot index of the last attacker (for XP on kill).
#[derive(Component)]
pub struct LastHitBy(pub i32);

/// Marker: NPC is inside a healing aura (near own faction's town center).
/// Used for visual feedback (halo effect).
#[derive(Component)]
pub struct Healing;

/// Marker: NPC is starving (energy reached 0).
/// Debuffs: HP capped at 50%, speed reduced 50%.
#[derive(Component)]
pub struct Starving;

/// Marker: farm is visually Ready (food icon overlay).
#[derive(Component)]
pub struct FarmReadyMarker {
    pub farm_idx: usize,
}

// ============================================================================
// BEHAVIOR CONFIG COMPONENTS (generic, attach to any NPC)
// ============================================================================

/// Flee combat when HP drops below this percentage.
#[derive(Component)]
pub struct FleeThreshold {
    pub pct: f32,
}

/// Disengage combat if distance from Home exceeds this.
#[derive(Component)]
pub struct LeashRange {
    pub distance: f32,
}

/// Drop everything and return home when HP drops below this percentage.
/// Distinct from FleeThreshold: wounded NPCs enter recovery mode.
#[derive(Component)]
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
