//! ECS Resources - Shared state accessible by all systems

use godot_bevy::prelude::bevy_ecs_prelude::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use crate::constants::MAX_NPC_COUNT;

/// Performance timing stats (updated each frame in process())
#[derive(Default)]
pub struct PerfStats {
    pub queue_ms: f32,
    pub dispatch_ms: f32,
    pub readpos_ms: f32,
    pub combat_ms: f32,
    pub build_ms: f32,
    pub upload_ms: f32,
    pub bevy_ms: f32,
    pub frame_ms: f32,  // Full frame time (process to process)
    pub prev_ecs_total_ms: f32,  // Previous frame's ECS time (for godot_ms calc)
    // Debug stats (cached to avoid extra GPU reads)
    pub arrived_count: i32,
    pub avg_backoff: i32,
    pub max_backoff: i32,
}

pub static PERF_STATS: Mutex<PerfStats> = Mutex::new(PerfStats {
    queue_ms: 0.0, dispatch_ms: 0.0, readpos_ms: 0.0,
    combat_ms: 0.0, build_ms: 0.0, upload_ms: 0.0, bevy_ms: 0.0, frame_ms: 0.0,
    prev_ecs_total_ms: 0.0,
    arrived_count: 0, avg_backoff: 0, max_backoff: 0,
});

/// Bevy frame timing resource
#[derive(Resource, Default)]
pub struct BevyFrameTimer {
    pub start: Option<std::time::Instant>,
}

/// Tracks total number of active NPCs.
#[derive(Resource, Default)]
pub struct NpcCount(pub usize);

/// Delta time for the current frame (seconds).
#[derive(Resource, Default)]
pub struct DeltaTime(pub f32);

/// O(1) lookup from NPC slot index to Bevy Entity.
/// Populated on spawn, used by damage_system for fast entity lookup.
#[derive(Resource, Default)]
pub struct NpcEntityMap(pub HashMap<usize, Entity>);

/// Population counts per (job_id, clan_id).
#[derive(Default, Clone)]
pub struct PopStats {
    pub alive: i32,
    pub working: i32,
    pub dead: i32,
}

/// Aggregated population stats, updated incrementally at spawn/death/state transitions.
#[derive(Resource, Default)]
pub struct PopulationStats(pub HashMap<(i32, i32), PopStats>);

/// Game config pushed from GDScript at startup.
#[derive(Resource)]
pub struct GameConfig {
    pub farmers_per_town: i32,
    pub guards_per_town: i32,
    pub raiders_per_camp: i32,
    pub spawn_interval_hours: i32,
    pub food_per_work_hour: i32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            farmers_per_town: 10,
            guards_per_town: 30,
            raiders_per_camp: 15,
            spawn_interval_hours: 4,
            food_per_work_hour: 1,
        }
    }
}

/// Game time tracking - Bevy-owned, uses PhysicsDelta from godot-bevy.
/// Only total_seconds is mutable. Day/hour/minute are derived on demand.
#[derive(Resource)]
pub struct GameTime {
    pub total_seconds: f32,        // Only mutable state - accumulates from PhysicsDelta
    pub seconds_per_hour: f32,     // Game speed: 5.0 = 1 game-hour per 5 real seconds
    pub start_hour: i32,           // Hour at game start (6 = 6am)
    pub time_scale: f32,           // 1.0 = normal, 2.0 = 2x speed
    pub paused: bool,
    pub last_hour: i32,            // Previous hour (for detecting hour ticks)
    pub hour_ticked: bool,         // True if hour just changed this frame
}

impl GameTime {
    pub fn total_hours(&self) -> i32 {
        (self.total_seconds / self.seconds_per_hour) as i32
    }

    pub fn day(&self) -> i32 {
        (self.start_hour + self.total_hours()) / 24 + 1
    }

    pub fn hour(&self) -> i32 {
        (self.start_hour + self.total_hours()) % 24
    }

    pub fn minute(&self) -> i32 {
        let seconds_into_hour = self.total_seconds % self.seconds_per_hour;
        ((seconds_into_hour / self.seconds_per_hour) * 60.0) as i32
    }

    pub fn is_daytime(&self) -> bool {
        let h = self.hour();
        h >= 6 && h < 20
    }
}

impl Default for GameTime {
    fn default() -> Self {
        Self {
            total_seconds: 0.0,
            seconds_per_hour: 5.0,
            start_hour: 6,
            time_scale: 1.0,
            last_hour: 0,
            hour_ticked: false,
            paused: false,
        }
    }
}

/// Per-clan respawn cooldowns. Maps clan_id -> hours until next spawn check.
#[derive(Resource, Default)]
pub struct RespawnTimers(pub HashMap<i32, i32>);

// ============================================================================
// UI STATE RESOURCES
// ============================================================================

/// Kill statistics for UI display.
#[derive(Resource, Clone, Default)]
pub struct KillStats {
    pub guard_kills: i32,      // Raiders killed by guards
    pub villager_kills: i32,   // Villagers (farmers/guards) killed by raiders
}

/// Currently selected NPC index (-1 = none).
#[derive(Resource, Default)]
pub struct SelectedNpc(pub i32);

// ============================================================================
// DEBUG RESOURCES
// ============================================================================

/// Health system debug info - updated by damage/death systems, read by GDScript.
#[derive(Resource, Default)]
pub struct HealthDebug {
    pub damage_processed: usize,
    pub deaths_this_frame: usize,
    pub despawned_this_frame: usize,
    pub bevy_entity_count: usize,
    pub health_samples: Vec<(usize, f32)>,
    // Healing debug
    pub healing_npcs_checked: usize,
    pub healing_positions_len: usize,
    pub healing_towns_count: usize,
    pub healing_in_zone_count: usize,
    pub healing_healed_count: usize,
}

/// Combat system debug info - updated by cooldown/attack systems, read by GDScript.
#[derive(Resource)]
pub struct CombatDebug {
    pub attackers_queried: usize,
    pub targets_found: usize,
    pub attacks_made: usize,
    pub chases_started: usize,
    pub in_combat_added: usize,
    pub sample_target_idx: i32,
    pub positions_len: usize,
    pub combat_targets_len: usize,
    pub bounds_failures: usize,
    pub sample_dist: f32,
    pub in_range_count: usize,
    pub timer_ready_count: usize,
    pub sample_timer: f32,
    pub cooldown_entities: usize,
    pub frame_delta: f32,
    pub sample_combat_target_0: i32,
    pub sample_combat_target_1: i32,
    pub sample_pos_0: (f32, f32),
    pub sample_pos_1: (f32, f32),
}

impl Default for CombatDebug {
    fn default() -> Self {
        Self {
            attackers_queried: 0,
            targets_found: 0,
            attacks_made: 0,
            chases_started: 0,
            in_combat_added: 0,
            sample_target_idx: -99,
            positions_len: 0,
            combat_targets_len: 0,
            bounds_failures: 0,
            sample_dist: -1.0,
            in_range_count: 0,
            timer_ready_count: 0,
            sample_timer: -1.0,
            cooldown_entities: 0,
            frame_delta: 0.0,
            sample_combat_target_0: -99,
            sample_combat_target_1: -99,
            sample_pos_0: (0.0, 0.0),
            sample_pos_1: (0.0, 0.0),
        }
    }
}

// ============================================================================
// UI CACHE RESOURCES
// ============================================================================

const NPC_LOG_CAPACITY: usize = 100;

/// Per-NPC metadata for UI display (names, levels, traits).
#[derive(Clone, Default)]
pub struct NpcMeta {
    pub name: String,
    pub level: i32,
    pub xp: i32,
    pub trait_id: i32,
    pub town_id: i32,
    pub job: i32,
}

/// A single log entry for an NPC's activity history.
#[derive(Clone)]
pub struct NpcLogEntry {
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub message: String,
}

/// Per-NPC metadata cache (names, levels, traits). Indexed by slot.
#[derive(Resource)]
pub struct NpcMetaCache(pub Vec<NpcMeta>);

impl Default for NpcMetaCache {
    fn default() -> Self {
        Self(vec![NpcMeta::default(); MAX_NPC_COUNT])
    }
}

/// Energy per NPC. Synced from Bevy Energy component.
#[derive(Resource)]
pub struct NpcEnergyCache(pub Vec<f32>);

impl Default for NpcEnergyCache {
    fn default() -> Self {
        Self(vec![100.0; MAX_NPC_COUNT])
    }
}

/// Per-town NPC lists for O(1) roster queries. Index = town_id, value = Vec of NPC slots.
#[derive(Resource, Default)]
pub struct NpcsByTownCache(pub Vec<Vec<usize>>);

/// Per-NPC activity logs. Indexed by slot. 500 entries max per NPC.
#[derive(Resource)]
pub struct NpcLogCache(pub Vec<VecDeque<NpcLogEntry>>);

impl Default for NpcLogCache {
    fn default() -> Self {
        Self((0..MAX_NPC_COUNT).map(|_| VecDeque::with_capacity(NPC_LOG_CAPACITY)).collect())
    }
}

impl NpcLogCache {
    /// Push a log message for an NPC with timestamp.
    pub fn push(&mut self, idx: usize, day: i32, hour: i32, minute: i32, message: String) {
        if idx >= MAX_NPC_COUNT {
            return;
        }
        let entry = NpcLogEntry { day, hour, minute, message };
        if let Some(log) = self.0.get_mut(idx) {
            if log.len() >= NPC_LOG_CAPACITY {
                log.pop_front();
            }
            log.push_back(entry);
        }
    }
}

// ============================================================================
// FOOD EVENT RESOURCES
// ============================================================================

/// A raider delivered stolen food to their camp.
#[derive(Clone, Debug)]
pub struct FoodDelivered {
    pub camp_idx: u32,
}

/// An NPC consumed food at their home location.
#[derive(Clone, Debug)]
pub struct FoodConsumed {
    pub location_idx: u32,
    pub is_camp: bool,
}

/// Food events (deliveries and consumption). Polled and drained by GDScript.
#[derive(Resource, Default)]
pub struct FoodEvents {
    pub delivered: Vec<FoodDelivered>,
    pub consumed: Vec<FoodConsumed>,
}

// ============================================================================
// PHASE 11.7: RESOURCES REPLACING STATICS
// ============================================================================

/// NPC slot allocator. Manages slot indices with free list for reuse.
#[derive(Resource, Default)]
pub struct SlotAllocator {
    pub next: usize,
    pub free: Vec<usize>,
}

impl SlotAllocator {
    pub fn alloc(&mut self) -> Option<usize> {
        self.free.pop().or_else(|| {
            if self.next < MAX_NPC_COUNT {
                let idx = self.next;
                self.next += 1;
                Some(idx)
            } else {
                None
            }
        })
    }
    pub fn free(&mut self, slot: usize) { self.free.push(slot); }
    pub fn count(&self) -> usize { self.next }
    pub fn reset(&mut self) { self.next = 0; self.free.clear(); }
}

/// Projectile slot allocator. Replaces FREE_PROJ_SLOTS static.
#[derive(Resource)]
pub struct ProjSlotAllocator {
    pub next: usize,
    pub free: Vec<usize>,
    pub max: usize,
}

impl Default for ProjSlotAllocator {
    fn default() -> Self { Self { next: 0, free: Vec::new(), max: 50_000 } }
}

impl ProjSlotAllocator {
    pub fn alloc(&mut self) -> Option<usize> {
        self.free.pop().or_else(|| {
            if self.next < self.max { let i = self.next; self.next += 1; Some(i) } else { None }
        })
    }
    pub fn free(&mut self, slot: usize) { self.free.push(slot); }
    pub fn reset(&mut self) { self.next = 0; self.free.clear(); }
}

/// Reset flag. Replaces RESET_BEVY static.
#[derive(Resource, Default)]
pub struct ResetFlag(pub bool);

/// GPU dispatch count. Replaces GPU_DISPATCH_COUNT static.
#[derive(Resource, Default)]
pub struct GpuDispatchCount(pub usize);

/// GPU readback state. Replaces GPU_READ_STATE static.
/// Populated by lib.rs after GPU readback, read by Bevy systems.
#[derive(Resource)]
pub struct GpuReadState {
    pub positions: Vec<f32>,       // [x0, y0, x1, y1, ...]
    pub combat_targets: Vec<i32>,  // target index per NPC (-1 = none)
    pub health: Vec<f32>,
    pub factions: Vec<i32>,
    pub npc_count: usize,
}

impl Default for GpuReadState {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            combat_targets: Vec::new(),
            health: Vec::new(),
            factions: Vec::new(),
            npc_count: 0,
        }
    }
}

/// Food storage per location. Replaces FOOD_STORAGE static.
#[derive(Resource, Default)]
pub struct FoodStorage {
    pub food: Vec<i32>,  // One entry per clan/location
}

impl FoodStorage {
    pub fn init(&mut self, count: usize) {
        self.food = vec![0; count];
    }
}

/// Farm growth state.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum FarmGrowthState {
    #[default]
    Growing,  // Crops growing, progress accumulating
    Ready,    // Ready to harvest, shows food icon
}

/// Per-farm growth tracking.
#[derive(Resource, Default)]
pub struct FarmStates {
    pub states: Vec<FarmGrowthState>,  // Per-farm state
    pub progress: Vec<f32>,            // Growth progress 0.0-1.0
}

impl FarmStates {
    pub fn push_farm(&mut self) {
        self.states.push(FarmGrowthState::Growing);
        self.progress.push(0.0);
    }
}

/// Per-faction statistics.
#[derive(Clone, Default)]
pub struct FactionStat {
    pub alive: i32,
    pub dead: i32,
    pub kills: i32,
}

/// Stats for all factions. Index 0 = player/villagers, 1+ = raider camps.
#[derive(Resource, Default)]
pub struct FactionStats {
    pub stats: Vec<FactionStat>,
}

/// Raider camp state for respawning and foraging.
/// Faction 1+ are raider camps. Index 0 in this struct = faction 1.
#[derive(Resource, Default)]
pub struct CampState {
    /// Max raiders per camp (set from config at init).
    pub max_pop: Vec<i32>,
    /// Hours accumulated since last respawn check.
    pub respawn_timers: Vec<f32>,
    /// Hours accumulated since last forage tick.
    pub forage_timers: Vec<f32>,
}

impl CampState {
    /// Initialize camp state for N camps.
    pub fn init(&mut self, num_camps: usize, max_per_camp: i32) {
        self.max_pop = vec![max_per_camp; num_camps];
        self.respawn_timers = vec![0.0; num_camps];
        self.forage_timers = vec![0.0; num_camps];
    }

    /// Get camp index from faction (faction 1 = camp 0, etc).
    pub fn faction_to_camp(faction: i32) -> Option<usize> {
        if faction > 0 {
            Some((faction - 1) as usize)
        } else {
            None
        }
    }
}

/// Queue of raiders waiting to form a raid group.
/// When enough raiders join the queue, they all dispatch together.
#[derive(Resource, Default)]
pub struct RaidQueue {
    /// (Entity, NpcIndex) waiting to raid, grouped by faction.
    /// Key = faction ID (1+ for raiders).
    pub waiting: HashMap<i32, Vec<(Entity, usize)>>,
}

impl RaidQueue {
    /// Remove a specific raider from the queue (e.g., when they die or enter combat).
    pub fn remove(&mut self, faction: i32, entity: Entity) {
        if let Some(queue) = self.waiting.get_mut(&faction) {
            queue.retain(|(e, _)| *e != entity);
        }
    }
}

impl FactionStats {
    pub fn init(&mut self, count: usize) {
        self.stats = vec![FactionStat::default(); count];
    }

    pub fn inc_alive(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.alive += 1;
        }
    }

    pub fn dec_alive(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.alive = (s.alive - 1).max(0);
        }
    }

    pub fn inc_dead(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.dead += 1;
        }
    }

    pub fn inc_kills(&mut self, faction: i32) {
        if let Some(s) = self.stats.get_mut(faction as usize) {
            s.kills += 1;
        }
    }
}
