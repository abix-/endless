//! ECS Resources - Shared state accessible by all systems

use bevy::prelude::*;
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use crate::constants::MAX_NPC_COUNT;

/// Per-system profiling (Factorio-style). RAII guard pattern: `let _t = timings.scope("name");`
/// Uses Res<SystemTimings> (not ResMut) with internal Mutex so parallel systems don't serialize.
#[derive(Resource)]
pub struct SystemTimings {
    data: Mutex<HashMap<&'static str, f32>>,
    pub frame_ms: Mutex<f32>,
    pub enabled: bool,
}

impl Default for SystemTimings {
    fn default() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
            frame_ms: Mutex::new(0.0),
            enabled: false,
        }
    }
}

impl SystemTimings {
    pub fn scope(&self, name: &'static str) -> TimerGuard<'_> {
        TimerGuard {
            timings: self,
            name,
            start: if self.enabled { Some(std::time::Instant::now()) } else { None },
        }
    }

    /// Record true frame time from Bevy's Time::delta (captures render + vsync + everything).
    pub fn record_frame_delta(&self, dt_secs: f32) {
        if self.enabled {
            let ms = dt_secs * 1000.0;
            if let Ok(mut fm) = self.frame_ms.lock() {
                *fm = *fm * 0.95 + ms * 0.05;
            }
        }
    }

    /// Record a timing value directly (same EMA as scope guard).
    /// Use for accumulated sub-section timings recorded after a loop.
    pub fn record(&self, name: &'static str, ms: f32) {
        if let Ok(mut data) = self.data.lock() {
            let entry = data.entry(name).or_insert(0.0);
            *entry = *entry * 0.95 + ms * 0.05;
        }
    }

    pub fn get_timings(&self) -> HashMap<&'static str, f32> {
        self.data.lock().map(|d| d.clone()).unwrap_or_default()
    }

    pub fn get_frame_ms(&self) -> f32 {
        self.frame_ms.lock().map(|f| *f).unwrap_or(0.0)
    }
}

pub struct TimerGuard<'a> {
    timings: &'a SystemTimings,
    name: &'static str,
    start: Option<std::time::Instant>,
}

impl Drop for TimerGuard<'_> {
    fn drop(&mut self) {
        if let Some(start) = self.start {
            let ms = start.elapsed().as_secs_f64() as f32 * 1000.0;
            if let Ok(mut data) = self.timings.data.lock() {
                let entry = data.entry(self.name).or_insert(0.0);
                *entry = *entry * 0.95 + ms * 0.05;
            }
        }
    }
}


/// Delta time for the current frame (seconds).
#[derive(Resource, Default)]
pub struct DeltaTime(pub f32);

/// NPC decision throttling config. Controls how often non-combat decisions are evaluated.
#[derive(Resource)]
pub struct NpcDecisionConfig {
    pub interval: f32, // seconds between decision evaluations (default 2.0)
}

impl Default for NpcDecisionConfig {
    fn default() -> Self { Self { interval: 2.0 } }
}

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
    /// Per-job home count (mirrors WorldGenConfig.npc_counts).
    pub npc_counts: std::collections::BTreeMap<crate::components::Job, i32>,
    pub spawn_interval_hours: i32,
    pub food_per_work_hour: i32,
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            npc_counts: crate::constants::NPC_REGISTRY.iter().map(|d| (d.job, d.default_count as i32)).collect(),
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
    /// Gameplay-scaled delta. Zero when paused, multiplied by time_scale otherwise.
    pub fn delta(&self, time: &Time) -> f32 {
        if self.paused { 0.0 } else { time.delta_secs() * self.time_scale }
    }

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

// ============================================================================
// UI STATE RESOURCES
// ============================================================================

/// Kill statistics for UI display.
#[derive(Resource, Clone, Default)]
pub struct KillStats {
    pub archer_kills: i32,      // Raiders killed by archers
    pub villager_kills: i32,   // Villagers (farmers/archers) killed by raiders
}

/// Currently selected NPC index (-1 = none).
#[derive(Resource)]
pub struct SelectedNpc(pub i32);
impl Default for SelectedNpc { fn default() -> Self { Self(-1) } }

/// Currently selected building (grid cell). `active = false` means no building selected.
#[derive(Resource, Default)]
pub struct SelectedBuilding {
    pub col: usize,
    pub row: usize,
    pub active: bool,
    pub slot: Option<usize>,
    pub kind: Option<crate::world::BuildingKind>,
}

/// Camera follow mode — when true, camera tracks the selected NPC.
#[derive(Resource, Default)]
pub struct FollowSelected(pub bool);

// ============================================================================
// DEBUG RESOURCES
// ============================================================================

/// Toggleable debug log flags. Controlled via pause menu settings.
#[derive(Resource)]
pub struct DebugFlags {
    /// Log GPU readback positions each tick
    pub readback: bool,
    /// Log combat stats each tick
    pub combat: bool,
    /// Log spawn/death events
    pub spawns: bool,
    /// Log behavior state changes
    pub behavior: bool,
}

impl Default for DebugFlags {
    fn default() -> Self {
        Self {
            readback: false,
            combat: false,
            spawns: false,
            behavior: false,
        }
    }
}

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

/// Runtime metric for target intent thrashing.
/// Tracks per-NPC SetTarget reason flips within the current game minute.
#[derive(Resource, Default)]
pub struct NpcTargetThrashDebug {
    pub minute_key: i32,
    pub sink_window_key: i64,
    pub writes_this_minute: Vec<u16>,
    pub reason_flips_this_minute: Vec<u16>,
    pub target_changes_this_minute: Vec<u16>,
    pub ping_pong_this_minute: Vec<u16>,
    pub last_reason: Vec<String>,
    pub last_target_q: Vec<(i32, i32)>,
    pub prev_target_q: Vec<(i32, i32)>,
    pub sink_writes_this_minute: Vec<u16>,
    pub sink_target_changes_this_minute: Vec<u16>,
    pub sink_ping_pong_this_minute: Vec<u16>,
    pub sink_last_target: Vec<(f32, f32)>,
    pub sink_prev_target: Vec<(f32, f32)>,
    pub sink_has_target: Vec<bool>,
}

impl NpcTargetThrashDebug {
    #[inline]
    fn target_delta_sq(a: (f32, f32), b: (f32, f32)) -> f32 {
        let dx = a.0 - b.0;
        let dy = a.1 - b.1;
        dx * dx + dy * dy
    }

    pub fn record(&mut self, idx: usize, reason: &'static str, minute_key: i32, x: f32, y: f32) {
        if self.minute_key != minute_key {
            self.minute_key = minute_key;
            self.writes_this_minute.fill(0);
            self.reason_flips_this_minute.fill(0);
            self.target_changes_this_minute.fill(0);
            self.ping_pong_this_minute.fill(0);
            self.sink_writes_this_minute.fill(0);
            self.sink_target_changes_this_minute.fill(0);
            self.sink_ping_pong_this_minute.fill(0);
        }
        self.ensure_len(idx + 1);
        self.writes_this_minute[idx] = self.writes_this_minute[idx].saturating_add(1);

        let q = (x.round() as i32, y.round() as i32);
        let last_q = self.last_target_q[idx];
        if last_q != (0, 0) && last_q != q {
            self.target_changes_this_minute[idx] = self.target_changes_this_minute[idx].saturating_add(1);
            if self.prev_target_q[idx] == q {
                self.ping_pong_this_minute[idx] = self.ping_pong_this_minute[idx].saturating_add(1);
            }
        }
        self.prev_target_q[idx] = last_q;
        self.last_target_q[idx] = q;

        if self.last_reason[idx] != reason {
            if !self.last_reason[idx].is_empty() {
                self.reason_flips_this_minute[idx] = self.reason_flips_this_minute[idx].saturating_add(1);
            }
            self.last_reason[idx].clear();
            self.last_reason[idx].push_str(reason);
        }
    }

    pub fn record_sink(&mut self, idx: usize, window_key: i64, x: f32, y: f32) {
        if self.sink_window_key != window_key {
            self.sink_window_key = window_key;
            self.sink_writes_this_minute.fill(0);
            self.sink_target_changes_this_minute.fill(0);
            self.sink_ping_pong_this_minute.fill(0);
            self.sink_has_target.fill(false);
        }
        self.ensure_len(idx + 1);
        self.sink_writes_this_minute[idx] = self.sink_writes_this_minute[idx].saturating_add(1);
        let curr = (x, y);
        if self.sink_has_target[idx] {
            let last = self.sink_last_target[idx];
            // Tiny epsilon to avoid float jitter noise while still catching visible flips.
            if Self::target_delta_sq(last, curr) > 0.01 {
                self.sink_target_changes_this_minute[idx] = self.sink_target_changes_this_minute[idx].saturating_add(1);
                let prev = self.sink_prev_target[idx];
                if Self::target_delta_sq(prev, curr) <= 0.01 {
                    self.sink_ping_pong_this_minute[idx] = self.sink_ping_pong_this_minute[idx].saturating_add(1);
                }
            }
            self.sink_prev_target[idx] = last;
        } else {
            self.sink_has_target[idx] = true;
        }
        self.sink_last_target[idx] = curr;
    }

    pub fn top_offenders(&self, top_n: usize) -> Vec<(usize, u16, u16, u16, u16, &str)> {
        let mut rows: Vec<(usize, u16, u16, u16, u16, &str)> = self.sink_target_changes_this_minute
            .iter()
            .enumerate()
            .filter_map(|(idx, &sink_changes)| {
                if sink_changes == 0 {
                    return None;
                }
                let reason_flips = self.reason_flips_this_minute.get(idx).copied().unwrap_or(0);
                let ping_pong = self.sink_ping_pong_this_minute.get(idx).copied().unwrap_or(0);
                let writes = self.sink_writes_this_minute.get(idx).copied().unwrap_or(0);
                let reason = self.last_reason.get(idx).map(|s| s.as_str()).unwrap_or("");
                Some((idx, sink_changes, ping_pong, reason_flips, writes, reason))
            })
            .collect();
        rows.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then_with(|| b.2.cmp(&a.2))
                .then_with(|| b.4.cmp(&a.4))
        });
        rows.truncate(top_n);
        rows
    }

    fn ensure_len(&mut self, len: usize) {
        if self.writes_this_minute.len() < len {
            self.writes_this_minute.resize(len, 0);
        }
        if self.reason_flips_this_minute.len() < len {
            self.reason_flips_this_minute.resize(len, 0);
        }
        if self.target_changes_this_minute.len() < len {
            self.target_changes_this_minute.resize(len, 0);
        }
        if self.ping_pong_this_minute.len() < len {
            self.ping_pong_this_minute.resize(len, 0);
        }
        if self.last_reason.len() < len {
            self.last_reason.resize_with(len, String::new);
        }
        if self.last_target_q.len() < len {
            self.last_target_q.resize(len, (0, 0));
        }
        if self.prev_target_q.len() < len {
            self.prev_target_q.resize(len, (0, 0));
        }
        if self.sink_writes_this_minute.len() < len {
            self.sink_writes_this_minute.resize(len, 0);
        }
        if self.sink_target_changes_this_minute.len() < len {
            self.sink_target_changes_this_minute.resize(len, 0);
        }
        if self.sink_ping_pong_this_minute.len() < len {
            self.sink_ping_pong_this_minute.resize(len, 0);
        }
        if self.sink_last_target.len() < len {
            self.sink_last_target.resize(len, (0.0, 0.0));
        }
        if self.sink_prev_target.len() < len {
            self.sink_prev_target.resize(len, (0.0, 0.0));
        }
        if self.sink_has_target.len() < len {
            self.sink_has_target.resize(len, false);
        }
    }
}

// ============================================================================
// MOVEMENT INTENT — Single-owner arbitration for NPC SetTarget
// ============================================================================

/// Priority ladder for movement intent resolution.
/// Higher value wins. Derive Ord so `max()` picks the winner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum MovementPriority {
    Wander = 0,
    JobRoute = 1,
    Squad = 2,
    Combat = 3,
    Survival = 4,
    ManualTarget = 5,
    DirectControl = 6,
}

/// A single movement intent submitted by a gameplay system.
#[derive(Clone, Debug)]
pub struct MovementIntent {
    pub target: Vec2,
    pub priority: MovementPriority,
    pub source: &'static str,
}

/// Per-NPC intent map. Keyed by Entity, cleared every frame.
/// Sparse — only NPCs whose target changes get an entry.
#[derive(Resource, Default)]
pub struct MovementIntents {
    intents: HashMap<Entity, MovementIntent>,
}

impl MovementIntents {
    /// Submit a movement intent. Keeps the highest-priority intent per entity.
    #[inline]
    pub fn submit(&mut self, entity: Entity, target: Vec2, priority: MovementPriority, source: &'static str) {
        match self.intents.entry(entity) {
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if priority > e.get().priority {
                    *e.get_mut() = MovementIntent { target, priority, source };
                }
            }
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(MovementIntent { target, priority, source });
            }
        }
    }

    /// Drain all intents for resolution. Clears the map but keeps allocation.
    pub fn drain(&mut self) -> std::collections::hash_map::Drain<'_, Entity, MovementIntent> {
        self.intents.drain()
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
    pub message: Cow<'static, str>,
}

/// Per-NPC metadata cache (names, levels, traits). Indexed by slot.
#[derive(Resource)]
pub struct NpcMetaCache(pub Vec<NpcMeta>);

impl Default for NpcMetaCache {
    fn default() -> Self {
        Self(vec![NpcMeta::default(); MAX_NPC_COUNT])
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
        Self((0..MAX_NPC_COUNT).map(|_| VecDeque::new()).collect())
    }
}

impl NpcLogCache {
    /// Push a log message for an NPC with timestamp.
    pub fn push(&mut self, idx: usize, day: i32, hour: i32, minute: i32, message: impl Into<Cow<'static, str>>) {
        if idx >= MAX_NPC_COUNT {
            return;
        }
        let entry = NpcLogEntry { day, hour, minute, message: message.into() };
        if let Some(log) = self.0.get_mut(idx) {
            if log.len() >= NPC_LOG_CAPACITY {
                log.pop_front();
            }
            log.push_back(entry);
        }
    }
}


// ============================================================================
// PHASE 11.7: RESOURCES REPLACING STATICS
// ============================================================================

/// Shared slot allocator logic. Wraps a free-list allocator with configurable max.
pub struct SlotPool {
    pub next: usize,
    pub max: usize,
    pub free: Vec<usize>,
}

impl SlotPool {
    pub fn new(max: usize) -> Self { Self { next: 0, max, free: Vec::new() } }
    pub fn alloc(&mut self) -> Option<usize> {
        self.free.pop().or_else(|| {
            if self.next < self.max {
                let idx = self.next;
                self.next += 1;
                Some(idx)
            } else {
                None
            }
        })
    }
    pub fn free(&mut self, slot: usize) { self.free.push(slot); }
    /// High-water mark: max slot index ever allocated. Use for GPU dispatch bounds.
    pub fn count(&self) -> usize { self.next }
    /// Currently alive: allocated minus freed. Use for UI display counts.
    pub fn alive(&self) -> usize { self.next - self.free.len() }
    pub fn reset(&mut self) { self.next = 0; self.free.clear(); }
}

/// NPC slot allocator. Manages slot indices 0..MAX_NPC_COUNT with free list for reuse.
#[derive(Resource)]
pub struct SlotAllocator(pub SlotPool);

impl Default for SlotAllocator {
    fn default() -> Self { Self(SlotPool::new(MAX_NPC_COUNT)) }
}

impl std::ops::Deref for SlotAllocator {
    type Target = SlotPool;
    fn deref(&self) -> &SlotPool { &self.0 }
}

impl std::ops::DerefMut for SlotAllocator {
    fn deref_mut(&mut self) -> &mut SlotPool { &mut self.0 }
}

/// Building slot allocator. Manages slot indices 0..MAX_BUILDINGS with free list for reuse.
#[derive(Resource)]
pub struct BuildingSlots(pub SlotPool);

impl Default for BuildingSlots {
    fn default() -> Self { Self(SlotPool::new(crate::constants::MAX_BUILDINGS)) }
}

impl std::ops::Deref for BuildingSlots {
    type Target = SlotPool;
    fn deref(&self) -> &SlotPool { &self.0 }
}

impl std::ops::DerefMut for BuildingSlots {
    fn deref_mut(&mut self) -> &mut SlotPool { &mut self.0 }
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


/// GPU readback state. Populated by ReadbackComplete observers, read by main-world Bevy systems.
#[derive(Resource)]
pub struct GpuReadState {
    pub positions: Vec<f32>,       // [x0, y0, x1, y1, ...]
    pub combat_targets: Vec<i32>,  // target index per NPC (-1 = none)
    pub health: Vec<f32>,
    pub factions: Vec<i32>,
    pub threat_counts: Vec<u32>,   // packed (enemies << 16 | allies) per NPC
    pub npc_count: usize,
}

impl Default for GpuReadState {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            combat_targets: Vec::new(),
            health: Vec::new(),
            factions: Vec::new(),
            threat_counts: Vec::new(),
            npc_count: 0,
        }
    }
}

/// GPU→CPU readback of projectile hit results. Each entry is [npc_idx, processed].
/// Populated by ReadbackComplete observer, read by process_proj_hits.
#[derive(Resource, Default)]
pub struct ProjHitState(pub Vec<[i32; 2]>);

/// GPU→CPU readback of projectile positions. [x0, y0, x1, y1, ...] flattened.
/// Populated by ReadbackComplete observer, read by extract_proj_data (ExtractSchedule).
#[derive(Resource, Default)]
pub struct ProjPositionState(pub Vec<f32>);

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

/// Gold storage per town. Mirrors FoodStorage.
#[derive(Resource, Default)]
pub struct GoldStorage {
    pub gold: Vec<i32>,
}

impl GoldStorage {
    pub fn init(&mut self, count: usize) {
        self.gold = vec![0; count];
    }
}


/// Per-faction statistics.
#[derive(Clone, Default)]
pub struct FactionStat {
    pub alive: i32,
    pub dead: i32,
    pub kills: i32,
}

/// Stats for all factions. Index 0 = player/villagers, 1+ = raider towns.
#[derive(Resource, Default)]
pub struct FactionStats {
    pub stats: Vec<FactionStat>,
}

/// Raider town state for respawning and foraging.
/// Faction 1+ are raider towns. Index 0 in this struct = faction 1.
#[derive(Resource, Default)]
pub struct RaiderState {
    /// Max raiders per town (set from config at init).
    pub max_pop: Vec<i32>,
    /// Hours accumulated since last respawn check.
    pub respawn_timers: Vec<f32>,
    /// Hours accumulated since last forage tick.
    pub forage_timers: Vec<f32>,
}

impl RaiderState {
    /// Initialize raider state for N towns.
    pub fn init(&mut self, count: usize, max_pop: i32) {
        self.max_pop = vec![max_pop; count];
        self.respawn_timers = vec![0.0; count];
        self.forage_timers = vec![0.0; count];
    }

    /// Get raider index from faction (faction 1 = index 0, etc).
    pub fn faction_to_idx(faction: i32) -> Option<usize> {
        if faction > 0 {
            Some((faction - 1) as usize)
        } else {
            None
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

// ============================================================================
// UI STATE
// ============================================================================

/// Active tab in the left panel.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum LeftPanelTab {
    #[default]
    Roster,
    Upgrades,
    Policies,
    Patrols,
    Squads,
    Factions,
    Profiler,
    Help,
}

/// Which UI panels are open. Toggled by keyboard shortcuts and HUD buttons.
#[derive(Resource)]
pub struct UiState {
    pub build_menu_open: bool,
    pub pause_menu_open: bool,
    pub left_panel_open: bool,
    pub left_panel_tab: LeftPanelTab,
    pub combat_log_visible: bool,
    /// MinerHome building data index — next click assigns a gold mine.
    pub assigning_mine: Option<usize>,
    /// Currently selected faction in the Factions tab (for world overlays).
    pub factions_overlay_faction: Option<i32>,
    /// Preferred inspector tab after latest click when both NPC and building are selected.
    pub inspector_prefer_npc: bool,
    /// Monotonic click counter for inspector tab auto-focus application.
    pub inspector_click_seq: u64,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            build_menu_open: false,
            pause_menu_open: false,
            left_panel_open: false,
            left_panel_tab: LeftPanelTab::default(),
            combat_log_visible: true,
            assigning_mine: None,
            factions_overlay_faction: None,
            inspector_prefer_npc: true,
            inspector_click_seq: 0,
        }
    }
}

impl UiState {
    /// Toggle left panel to a specific tab, or close if already showing that tab.
    pub fn toggle_left_tab(&mut self, tab: LeftPanelTab) {
        if self.left_panel_open && self.left_panel_tab == tab {
            self.left_panel_open = false;
        } else {
            self.left_panel_open = true;
            self.left_panel_tab = tab;
        }
    }
}

// ============================================================================
// BUILD MENU STATE
// ============================================================================

/// Context for build palette + placement mode.
#[derive(Resource)]
pub struct BuildMenuContext {
    /// Which town in WorldData.towns this placement targets.
    pub town_data_idx: Option<usize>,
    /// Active building selection for click-to-place mode.
    pub selected_build: Option<crate::world::BuildingKind>,
    /// Destroy mode — click to remove buildings.
    pub destroy_mode: bool,
    /// Last hovered snapped world position (for indicators/tooltips).
    pub hover_world_pos: Vec2,
    /// Drag-line start slot in town-grid coordinates (row, col).
    pub drag_start_slot: Option<(i32, i32)>,
    /// Drag-line current/end slot in town-grid coordinates (row, col).
    pub drag_current_slot: Option<(i32, i32)>,
    /// Show the mouse-follow build hint sprite (hidden when snapped over a valid build slot).
    pub show_cursor_hint: bool,
    /// Bevy image handles for ghost preview sprites (populated by build_menu init).
    pub ghost_sprites: std::collections::HashMap<crate::world::BuildingKind, Handle<Image>>,
}

impl Default for BuildMenuContext {
    fn default() -> Self {
        Self {
            town_data_idx: None,
            selected_build: None,
            destroy_mode: false,
            hover_world_pos: Vec2::ZERO,
            drag_start_slot: None,
            drag_current_slot: None,
            show_cursor_hint: true,
            ghost_sprites: std::collections::HashMap::new(),
        }
    }
}

impl BuildMenuContext {
    #[inline]
    pub fn clear_drag(&mut self) {
        self.drag_start_slot = None;
        self.drag_current_slot = None;
    }
}

// ============================================================================
// COMBAT LOG
// ============================================================================

/// Event type for combat log color coding.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum CombatEventKind {
    Kill,
    Spawn,
    Raid,
    Harvest,
    LevelUp,
    Ai,
    BuildingDamage,
    Loot,
}

/// A single combat log entry.
#[derive(Clone)]
pub struct CombatLogEntry {
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub kind: CombatEventKind,
    pub faction: i32,
    pub message: String,
    /// Optional world position — rendered as a clickable camera-pan button in the log.
    pub location: Option<bevy::math::Vec2>,
}

const COMBAT_LOG_MAX: usize = 200;
const COMBAT_LOG_PRIORITY_MAX: usize = 200;

/// Global combat event log. Ring buffer, newest at back.
#[derive(Resource, Default)]
pub struct CombatLog {
    pub entries: VecDeque<CombatLogEntry>,
    pub priority_entries: VecDeque<CombatLogEntry>,
}

impl CombatLog {
    pub fn push(&mut self, kind: CombatEventKind, faction: i32, day: i32, hour: i32, minute: i32, message: String) {
        self.push_at(kind, faction, day, hour, minute, message, None);
    }

    pub fn push_at(&mut self, kind: CombatEventKind, faction: i32, day: i32, hour: i32, minute: i32, message: String, location: Option<bevy::math::Vec2>) {
        let (target, cap) = if matches!(kind, CombatEventKind::Raid | CombatEventKind::Ai) {
            (&mut self.priority_entries, COMBAT_LOG_PRIORITY_MAX)
        } else {
            (&mut self.entries, COMBAT_LOG_MAX)
        };
        if target.len() >= cap {
            target.pop_front();
        }
        target.push_back(CombatLogEntry { day, hour, minute, kind, faction, message, location });
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &CombatLogEntry> {
        self.entries.iter().chain(self.priority_entries.iter())
    }
}

// ============================================================================
// BUILDING TOWER STATE
// ============================================================================

/// Per-building tower state for one building kind.
#[derive(Default)]
pub struct TowerKindState {
    /// Cooldown timer per building (seconds remaining).
    pub timers: Vec<f32>,
    /// Whether auto-attack is enabled per building.
    pub attack_enabled: Vec<bool>,
}

/// Tower state for all building kinds that can shoot.
#[derive(Resource, Default)]
pub struct TowerState {
    pub town: TowerKindState,
}

// ============================================================================
// BUILDING SPAWNERS
// ============================================================================

/// A single placed building instance. All runtime state for one building.
#[derive(Clone)]
pub struct BuildingInstance {
    pub kind: crate::world::BuildingKind,
    pub position: Vec2,
    pub town_idx: u32,
    pub slot: usize,
    pub entity: Entity,
    pub faction: i32,
    // Kind-specific fields (zero/None for non-applicable kinds)
    pub patrol_order: u32,           // Waypoint only
    pub assigned_mine: Option<Vec2>, // MinerHome only
    pub manual_mine: bool,           // MinerHome only
    pub wall_level: u8,              // Wall only
    pub npc_slot: i32,               // Spawner buildings only (-1 = no NPC alive)
    pub respawn_timer: f32,          // Spawner buildings only (-1.0 = not respawning)
    pub growth_ready: bool,             // Farm/Mine only (false = growing, true = ready to harvest)
    pub growth_progress: f32,            // Farm/Mine only (0.0 to 1.0)
}

impl BuildingInstance {
    /// Harvest a Ready farm/mine. Resets to Growing, returns yield (farm=1 food, mine=MINE_EXTRACT_PER_CYCLE gold). Returns 0 if not Ready.
    pub fn harvest(&mut self) -> i32 {
        if !self.growth_ready { return 0; }
        self.growth_ready = false;
        self.growth_progress = 0.0;
        match self.kind {
            crate::world::BuildingKind::Farm => 1,
            crate::world::BuildingKind::GoldMine => crate::constants::MINE_EXTRACT_PER_CYCLE,
            _ => 0,
        }
    }

    /// Log message for a harvest event.
    pub fn harvest_log_msg(&self, yield_amount: i32) -> String {
        match self.kind {
            crate::world::BuildingKind::Farm => format!("Farm harvested at ({:.0},{:.0})", self.position.x, self.position.y),
            crate::world::BuildingKind::GoldMine => format!("Mine harvested ({} gold)", yield_amount),
            _ => String::new(),
        }
    }
}

/// Building identity map: single source of truth for all building instances.
/// Stores all runtime building data (position, entity, kind-specific fields).
/// Also provides O(1) spatial lookups (absorbs BuildingSpatialGrid).
#[derive(Resource, Default)]
pub struct BuildingEntityMap {
    // Legacy index maps (Phase 1 compat — kept until all consumers migrated)
    to_slot: HashMap<(crate::world::BuildingKind, usize), usize>,
    from_slot: HashMap<usize, (crate::world::BuildingKind, usize)>,
    slot_to_entity: HashMap<usize, Entity>,

    // Instance storage (new — primary data store)
    instances: HashMap<usize, BuildingInstance>,          // slot → instance
    by_kind: HashMap<crate::world::BuildingKind, Vec<usize>>, // kind → slot list
    by_entity: HashMap<Entity, usize>,                    // entity → slot
    by_grid_cell: HashMap<(i32, i32), usize>,             // (grid_col, grid_row) → slot

    // Spatial grid (absorbs BuildingSpatialGrid)
    spatial_cell_size: f32,
    spatial_width: usize,
    spatial_cells: Vec<Vec<usize>>,
}

impl BuildingEntityMap {
    // ── Legacy identity API (kept for Phase 1 compat) ──────────────────

    pub fn insert(&mut self, kind: crate::world::BuildingKind, index: usize, slot: usize) {
        self.to_slot.insert((kind, index), slot);
        self.from_slot.insert(slot, (kind, index));
    }

    pub fn set_entity(&mut self, slot: usize, entity: Entity) {
        self.slot_to_entity.insert(slot, entity);
        // Also update instance + by_entity if instance exists
        if self.instances.contains_key(&slot) {
            self.by_entity.insert(entity, slot);
            if let Some(inst) = self.instances.get_mut(&slot) {
                inst.entity = entity;
            }
        }
    }

    pub fn insert_full(&mut self, kind: crate::world::BuildingKind, index: usize, slot: usize, entity: Entity) {
        self.insert(kind, index, slot);
        self.set_entity(slot, entity);
    }

    pub fn remove_by_building(&mut self, kind: crate::world::BuildingKind, index: usize) -> Option<usize> {
        if let Some(slot) = self.to_slot.remove(&(kind, index)) {
            self.from_slot.remove(&slot);
            self.slot_to_entity.remove(&slot);
            // Also remove from instance storage
            self.remove_instance(slot);
            Some(slot)
        } else {
            None
        }
    }

    pub fn get_slot(&self, kind: crate::world::BuildingKind, index: usize) -> Option<usize> {
        self.to_slot.get(&(kind, index)).copied()
    }

    pub fn get_building(&self, slot: usize) -> Option<(crate::world::BuildingKind, usize)> {
        self.from_slot.get(&slot).copied()
    }

    pub fn is_building(&self, slot: usize) -> bool {
        self.from_slot.contains_key(&slot)
    }

    pub fn get_entity(&self, slot: usize) -> Option<Entity> {
        self.slot_to_entity.get(&slot).copied()
    }

    pub fn get_entity_by_building(&self, kind: crate::world::BuildingKind, index: usize) -> Option<Entity> {
        self.get_slot(kind, index).and_then(|s| self.get_entity(s))
    }

    pub fn clear(&mut self) {
        self.to_slot.clear();
        self.from_slot.clear();
        self.slot_to_entity.clear();
        self.instances.clear();
        self.by_kind.clear();
        self.by_entity.clear();
        self.by_grid_cell.clear();
        self.spatial_cells.iter_mut().for_each(|c| c.clear());
    }

    pub fn len(&self) -> usize {
        self.to_slot.len()
    }

    /// Iterate all instance slot keys.
    pub fn all_slots(&self) -> impl Iterator<Item = usize> + '_ {
        self.instances.keys().copied()
    }

    // ── Instance API (new) ─────────────────────────────────────────────

    /// Add or update a building instance. Updates all indexes.
    /// If the slot already exists, removes old index entries first to avoid duplicates.
    pub fn add_instance(&mut self, inst: BuildingInstance) {
        let slot = inst.slot;
        let kind = inst.kind;
        // Remove old index entries if updating an existing slot
        if let Some(old) = self.instances.remove(&slot) {
            self.by_entity.remove(&old.entity);
            if let Some(slots) = self.by_kind.get_mut(&old.kind) {
                slots.retain(|&s| s != slot);
            }
            let old_gc = (old.position.x / 32.0).floor() as i32;
            let old_gr = (old.position.y / 32.0).floor() as i32;
            self.by_grid_cell.remove(&(old_gc, old_gr));
            self.spatial_remove(slot, old.position);
        }
        self.by_entity.insert(inst.entity, slot);
        self.by_kind.entry(kind).or_default().push(slot);
        // Grid cell index (32px grid)
        let gc = (inst.position.x / 32.0).floor() as i32;
        let gr = (inst.position.y / 32.0).floor() as i32;
        self.by_grid_cell.insert((gc, gr), slot);
        // Spatial grid
        self.spatial_insert(slot, inst.position);
        self.instances.insert(slot, inst);
    }

    /// Remove an instance by slot. Returns removed instance if any.
    fn remove_instance(&mut self, slot: usize) -> Option<BuildingInstance> {
        if let Some(inst) = self.instances.remove(&slot) {
            self.by_entity.remove(&inst.entity);
            if let Some(slots) = self.by_kind.get_mut(&inst.kind) {
                slots.retain(|&s| s != slot);
            }
            let gc = (inst.position.x / 32.0).floor() as i32;
            let gr = (inst.position.y / 32.0).floor() as i32;
            self.by_grid_cell.remove(&(gc, gr));
            self.spatial_remove(slot, inst.position);
            Some(inst)
        } else {
            None
        }
    }

    /// Get instance by slot (read-only).
    pub fn get_instance(&self, slot: usize) -> Option<&BuildingInstance> {
        self.instances.get(&slot)
    }

    /// Get instance by slot (mutable).
    pub fn get_instance_mut(&mut self, slot: usize) -> Option<&mut BuildingInstance> {
        self.instances.get_mut(&slot)
    }

    /// Iterate all instances.
    pub fn iter_instances(&self) -> impl Iterator<Item = &BuildingInstance> {
        self.instances.values()
    }

    /// Iterate all instances (mutable).
    pub fn iter_instances_mut(&mut self) -> impl Iterator<Item = &mut BuildingInstance> {
        self.instances.values_mut()
    }

    /// Iterate all instances of a given kind.
    pub fn iter_kind(&self, kind: crate::world::BuildingKind) -> impl Iterator<Item = &BuildingInstance> {
        let slots = self.by_kind.get(&kind);
        let instances = &self.instances;
        slots.into_iter().flat_map(|v| v.iter()).filter_map(move |&s| instances.get(&s))
    }

    /// Iterate all instances of a given kind for a specific town.
    pub fn iter_kind_for_town(&self, kind: crate::world::BuildingKind, town_idx: u32) -> impl Iterator<Item = &BuildingInstance> {
        self.iter_kind(kind).filter(move |i| i.town_idx == town_idx)
    }

    /// Count alive buildings of a kind for a town.
    pub fn count_for_town(&self, kind: crate::world::BuildingKind, town_idx: u32) -> usize {
        self.iter_kind_for_town(kind, town_idx).count()
    }

    /// Count alive buildings per kind for a town.
    pub fn building_counts(&self, town_idx: u32) -> HashMap<crate::world::BuildingKind, usize> {
        let mut counts = HashMap::new();
        for (kind, slots) in &self.by_kind {
            let count = slots.iter().filter(|&&s| self.instances.get(&s).is_some_and(|i| i.town_idx == town_idx)).count();
            if count > 0 { counts.insert(*kind, count); }
        }
        counts
    }

    /// Find gold mine display index by position (for "Gold Mine N" labels).
    pub fn gold_mine_index(&self, pos: Vec2) -> Option<usize> {
        self.iter_kind(crate::world::BuildingKind::GoldMine)
            .enumerate()
            .find(|(_, inst)| (inst.position - pos).length() < 1.0)
            .map(|(i, _)| i)
    }

    /// Find instance by grid-snapped position (< 1px tolerance).
    pub fn find_by_position(&self, pos: Vec2) -> Option<&BuildingInstance> {
        let gc = (pos.x / 32.0).floor() as i32;
        let gr = (pos.y / 32.0).floor() as i32;
        self.by_grid_cell.get(&(gc, gr)).and_then(|&s| self.instances.get(&s))
    }

    /// Find instance by grid-snapped position (mutable).
    pub fn find_by_position_mut(&mut self, pos: Vec2) -> Option<&mut BuildingInstance> {
        let gc = (pos.x / 32.0).floor() as i32;
        let gr = (pos.y / 32.0).floor() as i32;
        let slot = self.by_grid_cell.get(&(gc, gr)).copied()?;
        self.instances.get_mut(&slot)
    }

    /// Find farm at position (O(1) spatial lookup).
    pub fn find_farm_at(&self, pos: Vec2) -> Option<&BuildingInstance> {
        self.find_by_position(pos).filter(|i| i.kind == crate::world::BuildingKind::Farm)
    }

    /// Find farm at position (mutable).
    pub fn find_farm_at_mut(&mut self, pos: Vec2) -> Option<&mut BuildingInstance> {
        self.find_by_position_mut(pos).filter(|i| i.kind == crate::world::BuildingKind::Farm)
    }

    /// Find mine at position (O(1) spatial lookup).
    pub fn find_mine_at(&self, pos: Vec2) -> Option<&BuildingInstance> {
        self.find_by_position(pos).filter(|i| i.kind == crate::world::BuildingKind::GoldMine)
    }

    /// Find mine at position (mutable).
    pub fn find_mine_at_mut(&mut self, pos: Vec2) -> Option<&mut BuildingInstance> {
        self.find_by_position_mut(pos).filter(|i| i.kind == crate::world::BuildingKind::GoldMine)
    }

    /// Iterate all growable instances (Farm + GoldMine).
    pub fn iter_growable(&self) -> impl Iterator<Item = &BuildingInstance> {
        self.iter_kind(crate::world::BuildingKind::Farm)
            .chain(self.iter_kind(crate::world::BuildingKind::GoldMine))
    }

    /// Find slot by position.
    pub fn slot_at_position(&self, pos: Vec2) -> Option<usize> {
        let gc = (pos.x / 32.0).floor() as i32;
        let gr = (pos.y / 32.0).floor() as i32;
        self.by_grid_cell.get(&(gc, gr)).copied()
    }

    // ── Spatial grid (absorbs BuildingSpatialGrid) ─────────────────────

    /// Initialize spatial grid dimensions.
    pub fn init_spatial(&mut self, world_size_px: f32) {
        self.spatial_cell_size = 256.0;
        self.spatial_width = (world_size_px / self.spatial_cell_size).ceil() as usize + 1;
        let total = self.spatial_width * self.spatial_width;
        self.spatial_cells.resize_with(total, Vec::new);
    }

    /// Rebuild spatial grid from current instances. Called when dirty flag is set.
    pub fn rebuild_spatial(&mut self) {
        for cell in &mut self.spatial_cells { cell.clear(); }
        let slots: Vec<(usize, Vec2)> = self.instances.values().map(|i| (i.slot, i.position)).collect();
        for (slot, pos) in slots {
            self.spatial_insert(slot, pos);
        }
    }

    fn spatial_insert(&mut self, slot: usize, pos: Vec2) {
        if self.spatial_width == 0 { return; }
        let cx = (pos.x / self.spatial_cell_size) as usize;
        let cy = (pos.y / self.spatial_cell_size) as usize;
        if cx < self.spatial_width && cy < self.spatial_width {
            self.spatial_cells[cy * self.spatial_width + cx].push(slot);
        }
    }

    fn spatial_remove(&mut self, slot: usize, pos: Vec2) {
        if self.spatial_width == 0 { return; }
        let cx = (pos.x / self.spatial_cell_size) as usize;
        let cy = (pos.y / self.spatial_cell_size) as usize;
        if cx < self.spatial_width && cy < self.spatial_width {
            let idx = cy * self.spatial_width + cx;
            self.spatial_cells[idx].retain(|&s| s != slot);
        }
    }

    /// Iterate all buildings in cells overlapping the AABB (pos ± radius).
    /// Caller must do fine distance check if needed.
    pub fn for_each_nearby(&self, pos: Vec2, radius: f32, mut f: impl FnMut(&BuildingInstance)) {
        if self.spatial_width == 0 { return; }
        let cs = self.spatial_cell_size;
        let min_cx = ((pos.x - radius).max(0.0) / cs) as usize;
        let max_cx = (((pos.x + radius) / cs) as usize).min(self.spatial_width - 1);
        let min_cy = ((pos.y - radius).max(0.0) / cs) as usize;
        let max_cy = (((pos.y + radius) / cs) as usize).min(self.spatial_width - 1);
        for cy in min_cy..=max_cy {
            let row = cy * self.spatial_width;
            for cx in min_cx..=max_cx {
                for &slot in &self.spatial_cells[row + cx] {
                    if let Some(inst) = self.instances.get(&slot) {
                        f(inst);
                    }
                }
            }
        }
    }
}

/// Building HP render data. Read by build_overlay_instances for rendering.
#[derive(Resource, Default)]
pub struct BuildingHpRender {
    pub positions: Vec<Vec2>,
    pub health_pcts: Vec<f32>,
}

/// Per-town auto-upgrade flags. When enabled, upgrades are purchased automatically
/// once per game hour whenever the town has enough food.
#[derive(Resource)]
pub struct AutoUpgrade {
    pub flags: Vec<Vec<bool>>,
}

impl AutoUpgrade {
    /// Ensure flags vec has at least `n` town entries, each sized to current upgrade count.
    pub fn ensure_towns(&mut self, n: usize) {
        let count = crate::systems::stats::upgrade_count();
        while self.flags.len() < n {
            self.flags.push(vec![false; count]);
        }
        for v in &mut self.flags {
            v.resize(count, false);
        }
    }
}

impl Default for AutoUpgrade {
    fn default() -> Self {
        let count = crate::systems::stats::upgrade_count();
        Self { flags: vec![vec![false; count]; 16] }
    }
}

// ============================================================================
// TOWN POLICIES
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum WorkSchedule {
    #[default]
    Both,
    DayOnly,
    NightOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum OffDutyBehavior {
    #[default]
    GoToBed,
    StayAtFountain,
    WanderTown,
}

fn default_policy_mining_radius() -> f32 { crate::constants::DEFAULT_MINING_RADIUS }

/// Per-town behavior configuration. Controls flee thresholds, work schedules, off-duty behavior.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PolicySet {
    pub eat_food: bool,
    #[serde(alias = "guard_aggressive")]
    pub archer_aggressive: bool,
    #[serde(alias = "guard_leash")]
    pub archer_leash: bool,
    pub farmer_fight_back: bool,
    pub prioritize_healing: bool,
    pub farmer_flee_hp: f32,     // 0.0-1.0 percentage
    #[serde(alias = "guard_flee_hp")]
    pub archer_flee_hp: f32,
    pub recovery_hp: f32,        // 0.0-1.0 — go rest/heal when below this
    pub farmer_schedule: WorkSchedule,
    #[serde(alias = "guard_schedule")]
    pub archer_schedule: WorkSchedule,
    pub farmer_off_duty: OffDutyBehavior,
    #[serde(alias = "guard_off_duty")]
    pub archer_off_duty: OffDutyBehavior,
    #[serde(default = "default_policy_mining_radius")]
    pub mining_radius: f32,
}

impl Default for PolicySet {
    fn default() -> Self {
        Self {
            eat_food: true,
            archer_aggressive: false,
            archer_leash: true,
            farmer_fight_back: false,
            prioritize_healing: true,
            farmer_flee_hp: 0.30,
            archer_flee_hp: 0.15,
            recovery_hp: 0.80,
            farmer_schedule: WorkSchedule::Both,
            archer_schedule: WorkSchedule::Both,
            farmer_off_duty: OffDutyBehavior::GoToBed,
            archer_off_duty: OffDutyBehavior::GoToBed,
            mining_radius: crate::constants::DEFAULT_MINING_RADIUS,
        }
    }
}

/// Auto-mining cache and per-mine enable state.
#[derive(Resource, Default)]
pub struct MiningPolicy {
    /// Per-town discovered gold mine slots within policy radius.
    pub discovered_mines: Vec<Vec<usize>>,
    /// Per-gold-mine enabled toggle, keyed by BuildingEntityMap slot.
    pub mine_enabled: HashMap<usize, bool>,
}

// ============================================================================
// DIFFICULTY
// ============================================================================

/// Difficulty preset values for world gen.
pub struct DifficultyPreset {
    pub farms: usize,
    pub ai_towns: usize,
    pub raider_towns: usize,
    pub gold_mines: usize,
    /// Per-job NPC counts (only jobs listed are overridden; unlisted keep current value).
    pub npc_counts: std::collections::BTreeMap<crate::components::Job, usize>,
    pub endless_mode: bool,
    pub endless_strength: f32,
}

/// Game difficulty — scales building costs. Selected on main menu, immutable during play.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Resource, serde::Serialize, serde::Deserialize)]
pub enum Difficulty {
    Easy,
    #[default]
    Normal,
    Hard,
}

impl Difficulty {
    pub const ALL: [Difficulty; 3] = [Difficulty::Easy, Difficulty::Normal, Difficulty::Hard];

    pub fn label(self) -> &'static str {
        match self {
            Difficulty::Easy => "Easy",
            Difficulty::Normal => "Normal",
            Difficulty::Hard => "Hard",
        }
    }

    /// World gen presets. Overrides listed explicitly; unlisted jobs reset to NPC_REGISTRY defaults.
    pub fn presets(self) -> DifficultyPreset {
        use crate::components::Job;
        let (farms, ai_towns, raider_towns, gold_mines, endless_mode, endless_strength, overrides) = match self {
            Difficulty::Easy   => (4, 2, 2, 3, false, 0.5,  vec![(Job::Farmer, 4), (Job::Archer, 8), (Job::Raider, 0)]),
            Difficulty::Normal => (2, 5, 5, 2, true,  0.75, vec![(Job::Farmer, 2), (Job::Archer, 4), (Job::Raider, 1)]),
            Difficulty::Hard   => (1, 10, 10, 1, true, 1.25, vec![(Job::Farmer, 0), (Job::Archer, 2), (Job::Raider, 2)]),
        };
        // Start from registry defaults, then apply preset overrides
        let mut npc_counts: std::collections::BTreeMap<Job, usize> = crate::constants::NPC_REGISTRY
            .iter().map(|d| (d.job, d.default_count as usize)).collect();
        for (job, count) in overrides {
            npc_counts.insert(job, count);
        }
        DifficultyPreset { farms, ai_towns, raider_towns, gold_mines, npc_counts, endless_mode, endless_strength }
    }

    /// Migration group scaling: extra raiders per N player villagers.
    pub fn migration_scaling(self) -> i32 {
        match self {
            Difficulty::Easy => 6,
            Difficulty::Normal => 4,
            Difficulty::Hard => 2,
        }
    }
}

/// Per-town policy settings. Index matches WorldData.towns.
#[derive(Resource)]
pub struct TownPolicies {
    pub policies: Vec<PolicySet>,
}

impl Default for TownPolicies {
    fn default() -> Self {
        Self { policies: vec![PolicySet::default(); 16] }
    }
}

// ============================================================================
// SQUADS
// ============================================================================

/// Who controls a squad — player or an AI town.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, serde::Serialize, serde::Deserialize)]
pub enum SquadOwner {
    #[default]
    Player,
    Town(usize), // town_data_idx
}

/// Returns true if the NPC's town matches the squad owner.
pub fn npc_matches_owner(owner: SquadOwner, npc_town_id: i32, player_town: i32) -> bool {
    match owner {
        SquadOwner::Player => npc_town_id == player_town,
        SquadOwner::Town(tdi) => npc_town_id == tdi as i32,
    }
}

/// A squad of combat units (player-controlled or AI-commanded).
#[derive(Clone)]
pub struct Squad {
    /// NPC slot indices assigned to this squad.
    pub members: Vec<usize>,
    /// Squad target position. None = no target, guards patrol normally.
    pub target: Option<Vec2>,
    /// Desired member count. 0 = manual mode (no auto-recruit).
    pub target_size: usize,
    /// If true, squad members patrol waypoints when no squad target is set.
    pub patrol_enabled: bool,
    /// If true, squad members go home to rest when tired.
    pub rest_when_tired: bool,
    /// Wave state: true while this squad is actively attacking a target.
    pub wave_active: bool,
    /// Member count at wave start, used to detect heavy casualties.
    pub wave_start_count: usize,
    /// Minimum members required before a new wave can start.
    pub wave_min_start: usize,
    /// End wave when alive members drop below this percent of `wave_start_count`.
    pub wave_retreat_below_pct: usize,
    /// Squad owner: Player (indices 0..MAX_SQUADS) or AI Town (appended after).
    pub owner: SquadOwner,
    /// Hold fire: when true, members only attack their ManualTarget (no auto-engage).
    pub hold_fire: bool,
}

impl Squad {
    pub fn is_player(&self) -> bool { self.owner == SquadOwner::Player }
}

impl Default for Squad {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            target: None,
            target_size: 0,
            patrol_enabled: true,
            rest_when_tired: true,
            wave_active: false,
            wave_start_count: 0,
            wave_min_start: 0,
            wave_retreat_below_pct: 50,
            owner: SquadOwner::Player,
            hold_fire: false,
        }
    }
}

/// All squads + UI state. First MAX_SQUADS are player-reserved; AI squads appended after.
#[derive(Resource)]
pub struct SquadState {
    pub squads: Vec<Squad>,
    /// Currently selected squad in UI (-1 = none).
    pub selected: i32,
    /// When true, next left-click sets the selected squad's target.
    pub placing_target: bool,
    /// Box-select drag: world-space start position (None = not dragging).
    pub drag_start: Option<Vec2>,
    /// True while mouse is held and drag exceeds threshold (5px).
    pub box_selecting: bool,
    /// DC NPCs keep fighting after looting instead of returning home.
    pub dc_no_return: bool,
}

impl Default for SquadState {
    fn default() -> Self {
        Self {
            squads: (0..crate::constants::MAX_SQUADS).map(|_| Squad::default()).collect(),
            selected: 0,
            placing_target: false,
            drag_start: None,
            box_selecting: false,
            dc_no_return: false,
        }
    }
}

impl SquadState {
    /// Allocate a new squad with the given owner. Returns the squad index.
    pub fn alloc_squad(&mut self, owner: SquadOwner) -> usize {
        let idx = self.squads.len();
        self.squads.push(Squad { owner, ..Default::default() });
        idx
    }

    /// Iterate squads owned by a specific AI town.
    pub fn squads_for_town(&self, tdi: usize) -> impl Iterator<Item = (usize, &Squad)> {
        self.squads.iter().enumerate()
            .filter(move |(_, s)| s.owner == SquadOwner::Town(tdi))
    }
}

// ============================================================================
// HELP CATALOG
// ============================================================================

/// In-game help tooltips. Flat map of topic key → help text.
/// Single source of truth for all "?" tooltip content.
#[derive(Resource)]
pub struct HelpCatalog(pub HashMap<&'static str, &'static str>);

impl HelpCatalog {
    pub fn new() -> Self {
        let mut m = HashMap::new();

        // Top bar stats
        m.insert("food", "Farmers grow food at farms. Spend it on buildings (right-click green '+' slots) and upgrades (U key). Build more Houses to get more farmers.");
        m.insert("gold", "Gold mines appear between towns. Set your miner count in the Roster tab (R key) using the Miners slider. Miners walk to the nearest mine, dig gold, and bring it back.");
        m.insert("pop", "Living NPCs / spawner buildings. Build Farmer Homes and Archer Homes to grow your town. Dead NPCs respawn after 12 game-hours.");
        m.insert("farmers", "Each Farmer Home spawns 1 farmer who works at the nearest free farm. Build farms first, then Farmer Homes to staff them.");
        m.insert("archers", "Each Archer Home spawns 1 archer who patrols waypoints. Build Waypoints to create a patrol route, then Archer Homes to staff them.");
        m.insert("raiders", "Enemy raiders steal food from your farms. Build archers and waypoints near farms to defend them.");
        m.insert("time", "Space = pause/unpause. +/- = speed up/slow down (0.25x to 128x). Day/Night affects work schedules set in Policies (P key).");

        // Left panel tabs
        m.insert("tab_roster", "Filter, sort, click to inspect. F to follow.");
        m.insert("tab_upgrades", "Spend food and gold on permanent upgrades.");
        m.insert("tab_policies", "Work schedules, off-duty behavior, flee and aggro settings.");
        m.insert("tab_patrols", "Guard post patrol order. Use arrows to reorder.");
        m.insert("tab_squads", "Set squad sizes and map targets. 1-9 hotkeys.");
        m.insert("tab_profiler", "Per-system timings. Enable in ESC > Settings > Debug.");

        // Build menu
        m.insert("build_farm", "Grows food over time. Build a Farmer Home nearby to assign a farmer to harvest it.");
        m.insert("build_farmer_home", "Spawns 1 farmer. Farmer works at the nearest free farm. Build farms first!");
        m.insert("build_archer_home", "Spawns 1 archer. Archer patrols nearby waypoints and fights enemies.");
        m.insert("build_waypoint", "Patrol waypoint for guards. Guards patrol between nearby waypoints and fight enemies.");
        m.insert("build_tent", "Spawns 1 raider. Raiders steal food from enemy farms and bring it back to their town.");
        m.insert("build_miner_home", "Spawns 1 miner. Miner works at the nearest gold mine.");
        m.insert("unlock_slot", "Pay food to unlock this grid slot. Then right-click it again to build.");
        m.insert("destroy", "Remove this building. Its NPC dies and the slot becomes empty.");

        // Inspector (NPC)
        m.insert("npc_state", "What this NPC is currently doing. Working = at their job. Resting = recovering energy at home. Fighting = in combat.");
        m.insert("npc_energy", "Energy drains while active, recovers while resting at home. NPCs go rest when energy drops below 50, resume at 80.");
        m.insert("npc_trait", "Personality trait. 40% of NPCs spawn with one. Brave = never flees. Swift = +25% speed. Hardy = +25% HP.");
        m.insert("npc_level", "Archers level up from kills. +1% all stats per level. XP needed = (level+1)^2 x 100.");

        // Getting started
        m.insert("getting_started", "Welcome! Right-click green '+' slots to build.\n- Build Farms + Farmer Homes for food\n- Build Waypoints + Archer Homes for defense\n- Raiders will attack your farms\nKeys: R=roster, U=upgrades, P=policies, T=patrols, Q=squads, H=help");

        Self(m)
    }
}

// ============================================================================
// TUTORIAL STATE
// ============================================================================

/// Guided tutorial state machine. Step 0 = not started, 1-10 = active, 255 = done.
#[derive(Resource)]
pub struct TutorialState {
    pub step: u8,
    pub initial_farms: usize,
    pub initial_farmer_homes: usize,
    pub initial_waypoints: usize,
    pub initial_archer_homes: usize,
    pub initial_miner_homes: usize,
    pub camera_start: Vec2,
    /// Wall-clock seconds when tutorial started (for 10-minute auto-end).
    pub start_time: f64,
}

impl Default for TutorialState {
    fn default() -> Self {
        Self {
            step: 0,
            initial_farms: 0,
            initial_farmer_homes: 0,
            initial_waypoints: 0,
            initial_archer_homes: 0,
            initial_miner_homes: 0,
            camera_start: Vec2::ZERO,
            start_time: 0.0,
        }
    }
}

// ============================================================================
// MIGRATION STATE
// ============================================================================

/// Active migration group: boat → walk → settle lifecycle.
/// Phase 1 (boat): boat_slot is Some, member_slots empty, town_data_idx None
/// Phase 2 (walk): boat_slot None, member_slots filled, town_data_idx None
/// Phase 3 (settle): town created, NPCs get Home, migration cleared
pub struct MigrationGroup {
    // Boat phase
    pub boat_slot: Option<usize>,
    pub boat_pos: Vec2,
    /// Where the AI wants to settle (picked at boat spawn, far from existing towns).
    pub settle_target: Vec2,
    // Intent (from PendingAiSpawn)
    pub is_raider: bool,
    pub upgrade_levels: Vec<u8>,
    pub starting_food: i32,
    pub starting_gold: i32,
    // Set at disembark
    pub member_slots: Vec<usize>,
    pub faction: i32,
    // Set at settle
    pub town_data_idx: Option<usize>,
    pub grid_idx: usize,
}

/// Tracks dynamic raider town migrations.
#[derive(Resource, Default)]
pub struct MigrationState {
    pub active: Option<MigrationGroup>,
    pub check_timer: f32,
    /// Debug: force-spawn a migration group next frame (ignores cooldown/population checks).
    pub debug_spawn: bool,
}

/// Pending AI respawn queued by endless mode after a town is defeated.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct PendingAiSpawn {
    pub delay_remaining: f32,
    pub is_raider: bool,
    pub upgrade_levels: Vec<u8>,
    pub starting_food: i32,
    pub starting_gold: i32,
}

/// Endless mode: defeated AI enemies are replaced by new ones scaled to player strength.
#[derive(Resource)]
pub struct EndlessMode {
    pub enabled: bool,
    /// Fraction of player strength for replacement AI (0.25–1.5)
    pub strength_fraction: f32,
    pub pending_spawns: Vec<PendingAiSpawn>,
}

impl Default for EndlessMode {
    fn default() -> Self {
        Self { enabled: false, strength_fraction: 0.75, pending_spawns: Vec::new() }
    }
}

/// Pre-computed healing zone per town, indexed by faction for O(1) lookup.
pub struct HealingZone {
    pub center: Vec2,
    pub radius_sq: f32,
    pub heal_rate: f32,
}

/// Faction-indexed healing zone cache. Rebuilt when HealingZonesDirtyMsg is received.
#[derive(Resource, Default)]
pub struct HealingZoneCache {
    pub by_faction: Vec<Vec<HealingZone>>,
}

/// Tracks whether any buildings are damaged and need fountain healing.
/// Separate resource because this is persistent state (stays true while damage exists),
/// unlike the one-shot dirty signals which are now Bevy Messages.
#[derive(Resource, Default)]
pub struct BuildingHealState {
    pub needs_healing: bool,
}

// ============================================================================
// AUDIO
// ============================================================================

/// Runtime audio state — volume levels and loaded track handles.
#[derive(Resource)]
pub struct GameAudio {
    pub music_volume: f32,
    pub sfx_volume: f32,
    pub tracks: Vec<Handle<AudioSource>>,
    pub last_track: Option<usize>,
    pub loop_current: bool,
    /// UI-requested track — set by jukebox dropdown, consumed by jukebox_system.
    pub play_next: Option<usize>,
    /// Playback speed multiplier (0.25-2.0, default 1.0).
    pub music_speed: f32,
}

impl Default for GameAudio {
    fn default() -> Self {
        Self { music_volume: 0.3, sfx_volume: 0.5, tracks: Vec::new(), last_track: None, loop_current: false, play_next: None, music_speed: 1.0 }
    }
}

/// Marker component for the currently playing music entity.
#[derive(Component)]
pub struct MusicTrack;

/// Sound effect categories (scaffold for future SFX).
#[derive(Clone, Copy)]
pub enum SfxKind { ArrowShoot, Hit, Death, Build, Click, Upgrade }

/// Fire-and-forget SFX trigger message.
#[derive(Message, Clone)]
pub struct PlaySfxMsg(pub SfxKind);

// Test12 relocated to src/tests/vertical_slice.rs — uses shared TestState resource.
