//! ECS Components - Bevy entities have these attached

use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::bevy_ecs_prelude::*;

// ============================================================================
// CORE COMPONENTS
// ============================================================================

/// Links a Bevy entity to its index in the GPU buffers.
/// When spawning an NPC, we create an entity with NpcIndex(n) where n is the buffer slot.
#[derive(Component, Clone, Copy)]
pub struct NpcIndex(pub usize);

/// NPC's job determines behavior and color.
/// - Farmer (green): works at farms, avoids combat
/// - Guard (blue): patrols and fights raiders
/// - Raider (red): attacks guards, steals from farms
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
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
            Job::Farmer => (0.2, 0.8, 0.2, 1.0),  // Green
            Job::Guard => (0.2, 0.4, 0.9, 1.0),   // Blue
            Job::Raider => (0.9, 0.2, 0.2, 1.0),  // Red
            Job::Fighter => (0.8, 0.8, 0.2, 1.0), // Yellow
        }
    }
}

/// Marker component: this NPC has an active target to move toward.
/// Added when set_target() is called, could be removed when arrived.
#[derive(Component)]
pub struct HasTarget;

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
pub struct Guard {
    pub town_idx: u32,
}

/// Farmer marker - identifies NPC as a farmer.
#[derive(Component)]
pub struct Farmer {
    pub town_idx: u32,
}

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
pub struct Home(pub Vector2);

impl Home {
    pub fn is_valid(&self) -> bool {
        self.0.x >= 0.0 && self.0.y >= 0.0
    }
}

/// Patrol route for guards (or any NPC that patrols).
#[derive(Component)]
pub struct PatrolRoute {
    pub posts: Vec<Vector2>,
    pub current: usize,
}

/// Work position for farmers (or any NPC that works at a location).
#[derive(Component)]
pub struct WorkPosition(pub Vector2);

// ============================================================================
// STATE MARKERS (mutually exclusive)
// ============================================================================

/// NPC is moving toward next patrol post.
#[derive(Component)]
pub struct Patrolling;

/// NPC is standing at a post, waiting before moving to next.
#[derive(Component)]
pub struct OnDuty {
    pub ticks_waiting: u32,
}

/// NPC is at home/bed recovering energy.
#[derive(Component)]
pub struct Resting;

/// NPC is walking home to rest.
#[derive(Component)]
pub struct GoingToRest;

/// NPC is at work position, working.
#[derive(Component)]
pub struct Working;

/// NPC is walking to work position.
#[derive(Component)]
pub struct GoingToWork;

/// NPC is dead and pending removal.
#[derive(Component)]
pub struct Dead;

// ============================================================================
// HEALTH COMPONENT
// ============================================================================

/// NPC health (0-100). Dies when reaching 0.
#[derive(Component)]
pub struct Health(pub f32);

impl Default for Health {
    fn default() -> Self {
        Self(100.0)
    }
}

// ============================================================================
// COMBAT COMPONENTS
// ============================================================================

/// Faction determines hostility. NPCs attack different factions.
/// GPU uses this for targeting queries.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub enum Faction {
    Villager = 0,  // Guards and Farmers
    Raider = 1,    // Raiders
}

impl Faction {
    pub fn to_i32(self) -> i32 {
        self as i32
    }

    pub fn from_i32(v: i32) -> Self {
        match v {
            1 => Faction::Raider,
            _ => Faction::Villager,
        }
    }
}

/// Combat stats for NPCs that can fight.
/// Both melee and ranged attacks use projectiles — melee is just a fast, short-range projectile.
#[derive(Component)]
pub struct AttackStats {
    pub range: f32,              // Attack range in pixels
    pub damage: f32,             // Damage per hit
    pub cooldown: f32,           // Seconds between attacks
    pub projectile_speed: f32,   // Projectile travel speed (9999 = instant melee)
    pub projectile_lifetime: f32, // Projectile lifetime in seconds
}

impl AttackStats {
    pub fn melee() -> Self {
        Self {
            range: 150.0,
            damage: 15.0,
            cooldown: 1.0,
            projectile_speed: 500.0,
            projectile_lifetime: 0.5,
        }
    }

    pub fn ranged() -> Self {
        Self {
            range: 300.0,
            damage: 10.0,
            cooldown: 1.5,
            projectile_speed: 200.0,
            projectile_lifetime: 3.0,
        }
    }
}

impl Default for AttackStats {
    fn default() -> Self {
        Self::melee()
    }
}

/// Cooldown timer for attacks. When > 0, NPC can't attack.
#[derive(Component, Default)]
pub struct AttackTimer(pub f32);

/// Marker: NPC is actively fighting (has valid combat target).
/// Behavior systems should skip NPCs with this component.
#[derive(Component)]
pub struct InCombat;

// ============================================================================
// STEALING / RAIDING COMPONENTS
// ============================================================================

/// Marker: this NPC steals food from farms. Any NPC with this + Home
/// will use the steal decision system.
#[derive(Component)]
pub struct Stealer;

/// Marker: NPC is currently carrying stolen food.
#[derive(Component)]
pub struct CarryingFood;

/// State: NPC is walking to a farm to steal food.
#[derive(Component)]
pub struct Raiding;

/// State: NPC is walking back to home base (with or without food).
#[derive(Component)]
pub struct Returning;

/// State: NPC is resting until HP reaches threshold before resuming activity.
#[derive(Component)]
pub struct Recovering {
    pub threshold: f32,
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
