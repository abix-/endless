//! ECS Resources - Shared state accessible by all systems

pub use crate::entity_map::*;

use crate::constants::{MAX_ENTITIES, MAX_NPC_COUNT, MAX_PROJECTILES};
use bevy::prelude::*;
use bevy::reflect::Reflect;
use serde::{Serialize, Deserialize};
use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

/// CLI flag: skip main menu and start a new game with saved settings.
#[derive(Resource, Default)]
pub struct AutoStart(pub bool);

/// CLI flag: --test [name|all] — run integration tests and exit.
#[derive(Resource, Default)]
pub struct CliTestMode {
    pub active: bool,
    pub filter: Option<String>,
}

/// Profiling resource: frame timing + render-world timing drain + tracing capture.
/// Auto-capture via SystemTimingLayer handles all main-world systems.
/// Render-world timings still use record() via atomic drain in frame_timer_start.
const EMA_ALPHA: f32 = 0.1;

#[derive(Resource)]
pub struct SystemTimings {
    data: Mutex<HashMap<&'static str, f32>>,
    /// Tracing-captured timings (Bevy auto-spans, feature-gated behind `trace`).
    traced: Mutex<HashMap<String, f32>>,
    pub frame_ms: Mutex<f32>,
    /// Rolling peak frame time (resets every PEAK_WINDOW frames).
    frame_peak: Mutex<(f32, u32)>,
    pub enabled: bool,
}

impl Default for SystemTimings {
    fn default() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
            traced: Mutex::new(HashMap::new()),
            frame_ms: Mutex::new(0.0),
            frame_peak: Mutex::new((0.0, 0)),
            enabled: false,
        }
    }
}

impl SystemTimings {
    /// Record true frame time from Bevy's Time::delta (captures render + vsync + everything).
    pub fn record_frame_delta(&self, dt_secs: f32) {
        if self.enabled {
            let ms = dt_secs * 1000.0;
            if let Ok(mut fm) = self.frame_ms.lock() {
                *fm = *fm * (1.0 - EMA_ALPHA) + ms * EMA_ALPHA;
            }
            if let Ok(mut fp) = self.frame_peak.lock() {
                fp.0 = fp.0.max(ms);
                fp.1 += 1;
                if fp.1 >= 120 {
                    fp.0 = ms;
                    fp.1 = 0;
                }
            }
        }
    }

    /// Record a timing value directly (same EMA as scope guard).
    /// Use for accumulated sub-section timings recorded after a loop.
    pub fn record(&self, name: &'static str, ms: f32) {
        if let Ok(mut data) = self.data.lock() {
            let entry = data.entry(name).or_insert(0.0);
            *entry = *entry * (1.0 - EMA_ALPHA) + ms * EMA_ALPHA;
        }
    }

    /// Record a tracing-captured timing (from Bevy auto-spans).
    pub fn record_traced(&self, name: &str, ms: f32) {
        if let Ok(mut traced) = self.traced.lock() {
            let entry = traced.entry(name.to_string()).or_insert(0.0);
            // Already EMA-smoothed by the tracing layer; just copy the latest value.
            *entry = ms;
        }
    }

    pub fn get_timings(&self) -> HashMap<&'static str, f32> {
        self.data.lock().map(|d| d.clone()).unwrap_or_default()
    }

    pub fn get_traced_timings(&self) -> HashMap<String, f32> {
        self.traced.lock().map(|d| d.clone()).unwrap_or_default()
    }

    /// Get per-system peak ms from the tracing layer's rolling window.
    pub fn get_traced_peaks(&self) -> HashMap<String, f32> {
        crate::tracing_layer::TRACING_PEAKS
            .lock()
            .map(|p| p.iter().map(|(k, (peak, _))| (k.clone(), *peak)).collect())
            .unwrap_or_default()
    }

    pub fn get_frame_ms(&self) -> f32 {
        self.frame_ms.lock().map(|f| *f).unwrap_or(0.0)
    }

    pub fn get_frame_peak_ms(&self) -> f32 {
        self.frame_peak.lock().map(|f| f.0).unwrap_or(0.0)
    }
}

/// Delta time for the current frame (seconds).
#[derive(Resource, Default)]
pub struct DeltaTime(pub f32);

/// Monotonically increasing UID allocator. Starts at 1 (0 is reserved as "none").
#[derive(Resource)]
pub struct NextEntityUid(pub u64);

impl Default for NextEntityUid {
    fn default() -> Self {
        Self(1)
    }
}

impl NextEntityUid {
    /// Allocate the next UID. Never returns EntityUid(0).
    pub fn alloc(&mut self) -> crate::components::EntityUid {
        let uid = crate::components::EntityUid(self.0);
        self.0 += 1;
        uid
    }
}

/// NPC decision throttling config. Controls how often non-combat decisions are evaluated.
#[derive(Resource)]
pub struct NpcDecisionConfig {
    pub interval: f32, // seconds between decision evaluations (default 2.0)
    pub max_decisions_per_frame: usize, // max Tier 3 decisions per frame (adaptive bucket floor)
}

impl Default for NpcDecisionConfig {
    fn default() -> Self {
        Self {
            interval: 2.0,
            max_decisions_per_frame: 300,
        }
    }
}

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
            npc_counts: crate::constants::NPC_REGISTRY
                .iter()
                .map(|d| (d.job, d.default_count as i32))
                .collect(),
            spawn_interval_hours: 4,
            food_per_work_hour: 1,
        }
    }
}

/// Game time tracking - Bevy-owned, uses PhysicsDelta from godot-bevy.
/// Only total_seconds is mutable. Day/hour/minute are derived on demand.
#[derive(Resource, Reflect)]
#[reflect(Resource)]
pub struct GameTime {
    pub total_seconds: f32, // Only mutable state - accumulates from PhysicsDelta
    pub seconds_per_hour: f32, // Game speed: 5.0 = 1 game-hour per 5 real seconds
    pub start_hour: i32,    // Hour at game start (6 = 6am)
    pub time_scale: f32,    // 1.0 = normal, 2.0 = 2x speed
    pub paused: bool,
    pub last_hour: i32,    // Previous hour (for detecting hour ticks)
    pub hour_ticked: bool, // True if hour just changed this frame
}

impl GameTime {
    /// True when gameplay should be frozen.
    /// `time_scale <= 0` is treated the same as paused.
    pub fn is_paused(&self) -> bool {
        self.paused || self.time_scale <= 0.0
    }

    /// Gameplay-scaled delta. Zero when paused, multiplied by time_scale otherwise.
    pub fn delta(&self, time: &Time) -> f32 {
        if self.is_paused() {
            0.0
        } else {
            time.delta_secs() * self.time_scale
        }
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
        (6..20).contains(&h)
    }
}

impl Default for GameTime {
    fn default() -> Self {
        Self {
            total_seconds: 5.0 * 55.0 / 60.0, // Start at 6:55am
            seconds_per_hour: 5.0,
            start_hour: 6,
            time_scale: 1.0,
            last_hour: 0,
            hour_ticked: false,
            paused: false,
        }
    }
}

/// Tracks actual updates-per-second (UPS). Incremented each FixedUpdate tick,
/// sampled by the HUD once per wall-clock second.
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct UpsCounter {
    pub ticks_this_second: u32,
    pub display_ups: u32,
}

// ============================================================================
// UI STATE RESOURCES
// ============================================================================

/// Kill statistics for UI display.
#[derive(Resource, Clone, Default, Reflect)]
#[reflect(Resource)]
pub struct KillStats {
    pub archer_kills: i32,   // Raiders killed by archers
    pub villager_kills: i32, // Villagers (farmers/archers) killed by raiders
}

/// Currently selected NPC index (-1 = none).
#[derive(Resource)]
pub struct SelectedNpc(pub i32);
impl Default for SelectedNpc {
    fn default() -> Self {
        Self(-1)
    }
}

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
#[derive(Default)]
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
    pub healing_active_count: usize,
    pub healing_enter_checks: usize,
    pub healing_exits: usize,
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
            self.target_changes_this_minute[idx] =
                self.target_changes_this_minute[idx].saturating_add(1);
            if self.prev_target_q[idx] == q {
                self.ping_pong_this_minute[idx] = self.ping_pong_this_minute[idx].saturating_add(1);
            }
        }
        self.prev_target_q[idx] = last_q;
        self.last_target_q[idx] = q;

        if self.last_reason[idx] != reason {
            if !self.last_reason[idx].is_empty() {
                self.reason_flips_this_minute[idx] =
                    self.reason_flips_this_minute[idx].saturating_add(1);
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
                self.sink_target_changes_this_minute[idx] =
                    self.sink_target_changes_this_minute[idx].saturating_add(1);
                let prev = self.sink_prev_target[idx];
                if Self::target_delta_sq(prev, curr) <= 0.01 {
                    self.sink_ping_pong_this_minute[idx] =
                        self.sink_ping_pong_this_minute[idx].saturating_add(1);
                }
            }
            self.sink_prev_target[idx] = last;
        } else {
            self.sink_has_target[idx] = true;
        }
        self.sink_last_target[idx] = curr;
    }

    pub fn top_offenders(&self, top_n: usize) -> Vec<(usize, u16, u16, u16, u16, &str)> {
        let mut rows: Vec<(usize, u16, u16, u16, u16, &str)> = self
            .sink_target_changes_this_minute
            .iter()
            .enumerate()
            .filter_map(|(idx, &sink_changes)| {
                if sink_changes == 0 {
                    return None;
                }
                let reason_flips = self.reason_flips_this_minute.get(idx).copied().unwrap_or(0);
                let ping_pong = self
                    .sink_ping_pong_this_minute
                    .get(idx)
                    .copied()
                    .unwrap_or(0);
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
// PATHFINDING + MOVEMENT INTENT
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

/// Source of a pathfinding request — used for merge priority when deduplicating.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum PathSource {
    /// From resolve_movement_system — has a fresh goal.
    Movement,
    /// From invalidate_paths_on_building_change — goal may be stale.
    Invalidation,
}

/// A queued pathfinding request.
pub struct PathRequest {
    pub entity: Entity,
    pub slot: usize,
    pub start: IVec2,
    pub goal: IVec2,
    pub goal_world: Vec2,
    pub priority: u8, // 0=urgent, 1=normal, 2=low
    pub source: PathSource,
}

/// Unified movement + pathfinding queue.
/// World-space intents (from behavior/combat/health) are submitted via `submit()`,
/// then resolved to grid-space PathRequests in resolve_movement_system.
/// Invalidation feeds `enqueue()` directly with grid-space requests.
/// 3 priority buckets (0=urgent, 1=normal, 2=low) with per-bucket entity dedup.
#[derive(Resource, Default)]
pub struct PathRequestQueue {
    /// World-space intents awaiting grid conversion. Priority-wins-per-entity dedup.
    pending_intents: HashMap<Entity, MovementIntent>,
    /// Grid-space path requests in 3 priority buckets.
    buckets: [HashMap<Entity, PathRequest>; 3],
}

impl PathRequestQueue {
    /// Submit a movement intent (world-space). Keeps highest priority per entity.
    #[inline]
    pub fn submit(
        &mut self,
        entity: Entity,
        target: Vec2,
        priority: MovementPriority,
        source: &'static str,
    ) {
        use std::collections::hash_map::Entry;
        match self.pending_intents.entry(entity) {
            Entry::Occupied(mut e) => {
                if priority > e.get().priority {
                    *e.get_mut() = MovementIntent { target, priority, source };
                }
            }
            Entry::Vacant(e) => {
                e.insert(MovementIntent { target, priority, source });
            }
        }
    }

    /// Drain all pending world-space intents for resolution.
    pub fn drain_intents(&mut self) -> std::collections::hash_map::Drain<'_, Entity, MovementIntent> {
        self.pending_intents.drain()
    }

    /// Insert or merge a grid-space path request. Per-entity dedupe within priority bucket.
    /// Movement source has fresher goal — prefer it over Invalidation.
    pub fn enqueue(&mut self, req: PathRequest) {
        use std::collections::hash_map::Entry;
        let bucket_idx = (req.priority as usize).min(2);
        let bucket = &mut self.buckets[bucket_idx];
        match bucket.entry(req.entity) {
            Entry::Vacant(e) => {
                e.insert(req);
            }
            Entry::Occupied(mut e) => {
                let existing = e.get_mut();
                if req.source == PathSource::Movement {
                    existing.start = req.start;
                    existing.goal = req.goal;
                    existing.goal_world = req.goal_world;
                    existing.slot = req.slot;
                    existing.source = PathSource::Movement;
                }
            }
        }
    }

    pub fn is_empty(&self) -> bool {
        self.buckets.iter().all(|b| b.is_empty())
    }

    pub fn total_len(&self) -> usize {
        self.buckets.iter().map(|b| b.len()).sum()
    }

    /// Drain up to `max` requests in priority order. O(max), not O(total).
    /// Sorted by slot within batch for determinism.
    pub fn drain_budget(&mut self, max: usize) -> Vec<PathRequest> {
        let mut result = Vec::with_capacity(max);
        for bucket in &mut self.buckets {
            if result.len() >= max {
                break;
            }
            let remaining = max - result.len();
            if bucket.len() <= remaining {
                result.extend(bucket.drain().map(|(_, r)| r));
            } else {
                let keys: Vec<Entity> = bucket.keys().take(remaining).copied().collect();
                for key in keys {
                    if let Some(req) = bucket.remove(&key) {
                        result.push(req);
                    }
                }
            }
        }
        result.sort_unstable_by_key(|r| r.slot);
        result
    }
}

/// Tuning constants for the pathfinding budget system.
#[derive(Resource)]
pub struct PathfindConfig {
    /// Max A* calls per FixedUpdate tick.
    pub max_per_frame: usize,
    /// Manhattan distance threshold (in tiles) — below this, use direct LOS instead of A*.
    pub short_distance_tiles: i32,
    /// Max nodes A* visits before early termination (prevents worst-case spikes).
    pub max_nodes: usize,
    /// Frames without position progress before re-queuing a path request.
    pub stuck_repath_frames: u32,
    /// Max milliseconds per tick for A* processing (early break guard).
    pub max_time_budget_ms: f32,
}

impl Default for PathfindConfig {
    fn default() -> Self {
        Self {
            max_per_frame: 200,
            short_distance_tiles: 12,
            max_nodes: 5000,
            stuck_repath_frames: 30,
            max_time_budget_ms: 2.0,
        }
    }
}

/// Live A* pathfinding metrics for profiler display. Updated every frame by resolve_movement_system.
#[derive(Resource)]
pub struct PathfindStats {
    /// EMA-smoothed: paths processed per frame
    pub processed: f32,
    /// EMA-smoothed: LOS bypasses per frame
    pub los_bypass: f32,
    /// EMA-smoothed: full A* calls per frame
    pub astar_calls: f32,
    /// EMA-smoothed: A* failures (no path / node limit) per frame
    pub astar_fails: f32,
    /// EMA-smoothed: elapsed ms per frame
    pub elapsed_ms: f32,
    /// Snapshot: queue depth after processing
    pub queue_remaining: usize,
    /// Last limit reason ("count" or "time")
    pub limit_reason: &'static str,
}

impl Default for PathfindStats {
    fn default() -> Self {
        Self {
            processed: 0.0,
            los_bypass: 0.0,
            astar_calls: 0.0,
            astar_fails: 0.0,
            elapsed_ms: 0.0,
            queue_remaining: 0,
            limit_reason: "count",
        }
    }
}

const PATHFIND_EMA: f32 = 0.1;

impl PathfindStats {
    pub fn update(
        &mut self,
        processed: usize,
        los_bypass: usize,
        astar_calls: usize,
        astar_fails: usize,
        elapsed_ms: f32,
        queue_remaining: usize,
        limit_reason: &'static str,
    ) {
        let a = PATHFIND_EMA;
        self.processed = self.processed * (1.0 - a) + processed as f32 * a;
        self.los_bypass = self.los_bypass * (1.0 - a) + los_bypass as f32 * a;
        self.astar_calls = self.astar_calls * (1.0 - a) + astar_calls as f32 * a;
        self.astar_fails = self.astar_fails * (1.0 - a) + astar_fails as f32 * a;
        self.elapsed_ms = self.elapsed_ms * (1.0 - a) + elapsed_ms * a;
        self.queue_remaining = queue_remaining;
        self.limit_reason = limit_reason;
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
    pub trait_display: String,
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

/// Per-NPC activity logs. Indexed by slot. 500 entries max per NPC.
#[derive(Resource)]
pub struct NpcLogCache {
    pub logs: Vec<VecDeque<NpcLogEntry>>,
    /// Filtering mode (synced from UserSettings each frame).
    pub mode: crate::settings::NpcLogMode,
    /// Currently selected NPC slot (-1 = none).
    pub selected: i32,
    /// Player faction id (for Faction mode filtering).
    pub player_faction: i32,
    /// Per-slot faction cache (set from decision_system iteration).
    slot_factions: Vec<i32>,
}

impl Default for NpcLogCache {
    fn default() -> Self {
        Self {
            logs: (0..MAX_NPC_COUNT).map(|_| VecDeque::new()).collect(),
            mode: crate::settings::NpcLogMode::SelectedOnly,
            selected: -1,
            player_faction: 0,
            slot_factions: vec![-1; MAX_NPC_COUNT],
        }
    }
}

impl NpcLogCache {
    /// Record a slot's faction (called during decision_system iteration).
    #[inline]
    pub fn set_slot_faction(&mut self, idx: usize, faction: i32) {
        if idx < self.slot_factions.len() {
            self.slot_factions[idx] = faction;
        }
    }

    /// Update selected NPC, clearing stale logs from previously selected NPC.
    pub fn update_selected(&mut self, new_selected: i32) {
        if new_selected != self.selected {
            // Clear previous selection's log when in SelectedOnly mode
            if self.mode == crate::settings::NpcLogMode::SelectedOnly {
                let old = self.selected as usize;
                if old < self.logs.len() {
                    self.logs[old].clear();
                }
            }
            self.selected = new_selected;
        }
    }

    /// Push a log message for an NPC with timestamp.
    /// Filtered by current mode — early-returns for NPCs outside the active scope.
    pub fn push(
        &mut self,
        idx: usize,
        day: i32,
        hour: i32,
        minute: i32,
        message: impl Into<Cow<'static, str>>,
    ) {
        if idx >= MAX_NPC_COUNT {
            return;
        }

        // Gate by mode
        match self.mode {
            crate::settings::NpcLogMode::SelectedOnly => {
                if self.selected < 0 || idx != self.selected as usize {
                    return;
                }
            }
            crate::settings::NpcLogMode::Faction => {
                if idx < self.slot_factions.len() && self.slot_factions[idx] != self.player_faction
                {
                    return;
                }
            }
            crate::settings::NpcLogMode::All => {}
        }

        let entry = NpcLogEntry {
            day,
            hour,
            minute,
            message: message.into(),
        };
        if let Some(log) = self.logs.get_mut(idx) {
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
    pub fn new(max: usize) -> Self {
        Self {
            next: 0,
            max,
            free: Vec::new(),
        }
    }
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
    pub fn free(&mut self, slot: usize) {
        self.free.push(slot);
    }
    /// High-water mark: max slot index ever allocated. Use for GPU dispatch bounds.
    pub fn count(&self) -> usize {
        self.next
    }
    /// Currently alive: allocated minus freed. Use for UI display counts.
    pub fn alive(&self) -> usize {
        self.next - self.free.len()
    }
    pub fn reset(&mut self) {
        self.next = 0;
        self.free.clear();
    }
}

/// Unified entity slot allocator. NPCs and buildings share the same slot namespace.
/// Slot = GPU index (no offset arithmetic). Manages 0..MAX_ENTITIES with free list.
/// Every allocation queues a GPU state reset (drained by `populate_gpu_state`).
#[derive(Resource)]
pub struct GpuSlotPool {
    pool: SlotPool,
    pending_resets: Vec<usize>,
    pending_frees: Vec<usize>,
}

impl Default for GpuSlotPool {
    fn default() -> Self {
        Self {
            pool: SlotPool::new(MAX_ENTITIES),
            pending_resets: Vec::new(),
            pending_frees: Vec::new(),
        }
    }
}

impl GpuSlotPool {
    /// Allocate a slot and queue a full GPU state reset (prevents stale data from previous occupant).
    pub fn alloc_reset(&mut self) -> Option<usize> {
        let slot = self.pool.alloc()?;
        self.pending_resets.push(slot);
        Some(slot)
    }
    pub fn free(&mut self, slot: usize) {
        self.pool.free(slot);
        self.pending_frees.push(slot);
    }
    /// High-water mark: max slot index ever allocated.
    pub fn count(&self) -> usize {
        self.pool.count()
    }
    /// Currently alive: allocated minus freed.
    pub fn alive(&self) -> usize {
        self.pool.alive()
    }
    pub fn reset(&mut self) {
        self.pool.reset();
        self.pending_resets.clear();
        self.pending_frees.clear();
    }
    /// Drain slots needing GPU state reset. Called by `populate_gpu_state`.
    pub fn take_pending_resets(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.pending_resets)
    }
    /// Drain slots needing GPU hide cleanup. Called by `populate_gpu_state`.
    pub fn take_pending_frees(&mut self) -> Vec<usize> {
        std::mem::take(&mut self.pending_frees)
    }
    /// Direct access for save/load that rebuilds allocator state.
    pub fn set_next(&mut self, n: usize) {
        self.pool.next = n;
    }
    /// Direct access to free list for save/load.
    pub fn free_list_mut(&mut self) -> &mut Vec<usize> {
        &mut self.pool.free
    }
    /// Read-only access to free list for debug display.
    pub fn free_list(&self) -> &[usize] {
        &self.pool.free
    }
    /// High-water mark (alias for debug display).
    pub fn next(&self) -> usize {
        self.pool.next
    }
}

/// Projectile slot allocator. Wraps SlotPool like GpuSlotPool.
#[derive(Resource)]
pub struct ProjSlotAllocator(pub SlotPool);

impl Default for ProjSlotAllocator {
    fn default() -> Self {
        Self(SlotPool::new(MAX_PROJECTILES))
    }
}

impl std::ops::Deref for ProjSlotAllocator {
    type Target = SlotPool;
    fn deref(&self) -> &SlotPool {
        &self.0
    }
}

impl std::ops::DerefMut for ProjSlotAllocator {
    fn deref_mut(&mut self) -> &mut SlotPool {
        &mut self.0
    }
}

/// GPU readback state. Populated by ReadbackComplete observers, read by main-world Bevy systems.
#[derive(Resource)]
#[derive(Default)]
pub struct GpuReadState {
    pub positions: Vec<f32>,      // [x0, y0, x1, y1, ...]
    pub combat_targets: Vec<i32>, // target index per NPC (-1 = none)
    pub health: Vec<f32>,
    pub factions: Vec<i32>,
    pub threat_counts: Vec<u32>, // packed (enemies << 16 | allies) per NPC
    pub npc_count: usize,
}


/// GPU→CPU readback of projectile hit results. Each entry is [npc_idx, processed].
/// Populated by ReadbackComplete observer, read by process_proj_hits.
#[derive(Resource, Default)]
pub struct ProjHitState(pub Vec<[i32; 2]>);

/// GPU→CPU readback of projectile positions. [x0, y0, x1, y1, ...] flattened.
/// Populated by ReadbackComplete observer, read by extract_proj_data (ExtractSchedule).
#[derive(Resource, Default)]
pub struct ProjPositionState(pub Vec<f32>);

/// O(1) lookup from town_idx → Bevy Entity for town ECS entities.
#[derive(Resource, Default)]
pub struct TownIndex(pub HashMap<i32, Entity>);


/// Monotonic counter for unique loot item IDs.
#[derive(Resource, Default)]
pub struct NextLootItemId {
    pub next: u64,
}

impl NextLootItemId {
    pub fn alloc(&mut self) -> u64 {
        let id = self.next;
        self.next += 1;
        id
    }
}


/// Per-town merchant stock (items for sale + refresh timer).
#[derive(Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MerchantStock {
    pub items: Vec<crate::constants::LootItem>,
    /// Game-hours until next stock refresh. <=0 means needs refresh.
    pub refresh_timer: f32,
}

/// Merchant inventory per town.
#[derive(Resource, Default, Clone, serde::Serialize, serde::Deserialize)]
pub struct MerchantInventory {
    pub stocks: Vec<MerchantStock>,
}

impl MerchantInventory {
    pub fn init(&mut self, town_count: usize) {
        self.stocks.resize_with(town_count, MerchantStock::default);
    }

    pub fn refresh(&mut self, town_idx: usize, next_id: &mut NextLootItemId) {
        if town_idx >= self.stocks.len() {
            self.stocks.resize_with(town_idx + 1, MerchantStock::default);
        }
        let stock = &mut self.stocks[town_idx];
        stock.items.clear();
        let count = 4 + (next_id.next as usize % 3); // 4-6 items
        for _ in 0..count {
            let id = next_id.alloc();
            stock.items.push(crate::constants::roll_loot_item(id, id as u32));
        }
        stock.refresh_timer = 12.0; // 12 game-hours
    }

    pub fn remove(&mut self, town_idx: usize, item_id: u64) -> Option<crate::constants::LootItem> {
        let stock = self.stocks.get_mut(town_idx)?;
        let pos = stock.items.iter().position(|i| i.id == item_id)?;
        Some(stock.items.swap_remove(pos))
    }
}

/// What kind of faction this is — determines AI behavior and UI treatment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Reflect)]
pub enum FactionKind {
    Neutral,
    Player,
    AiBuilder,
    AiRaider,
}

/// A faction in the game. Owns towns, buildings, and NPCs.
#[derive(Clone, Debug, Serialize, Deserialize, Reflect)]
pub struct FactionData {
    pub kind: FactionKind,
    pub name: String,
    /// Town indices owned by this faction (most factions own exactly 1 town).
    pub towns: Vec<usize>,
}

/// All factions. Index 0 = Neutral, 1 = Player, 2+ = AI.
#[derive(Resource, Default, Serialize, Deserialize, Reflect)]
#[reflect(Resource)]
pub struct FactionList {
    pub factions: Vec<FactionData>,
}

impl FactionList {
    pub fn is_player(&self, faction_idx: i32) -> bool {
        self.factions.get(faction_idx as usize).is_some_and(|f| f.kind == FactionKind::Player)
    }

    pub fn is_neutral(&self, faction_idx: i32) -> bool {
        faction_idx == 0 || self.factions.get(faction_idx as usize).is_some_and(|f| f.kind == FactionKind::Neutral)
    }

    pub fn player_faction(&self) -> Option<usize> {
        self.factions.iter().position(|f| f.kind == FactionKind::Player)
    }

    pub fn player_town(&self) -> Option<usize> {
        self.factions.iter().find(|f| f.kind == FactionKind::Player)
            .and_then(|f| f.towns.first().copied())
    }
}

/// Per-faction statistics.
#[derive(Clone, Default, Reflect)]
pub struct FactionStat {
    pub alive: i32,
    pub dead: i32,
    pub kills: i32,
}

/// Stats for all factions. Indexed by faction ID (0=Neutral, 1=Player, 2+=AI).
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct FactionStats {
    pub stats: Vec<FactionStat>,
}

/// Faction-vs-faction reputation matrix. values[a][b] = how faction a feels about faction b.
/// 0.0 = neutral. Negative = hostile (they killed our NPCs). Range -9999..9999.
#[derive(Resource, Default)]
pub struct Reputation {
    pub values: Vec<Vec<f32>>,
}

impl Reputation {
    pub fn init(&mut self, count: usize) {
        self.values = vec![vec![0.0; count]; count];
    }

    /// Ensure matrix is at least NxN (for dynamic faction additions).
    pub fn ensure_size(&mut self, count: usize) {
        while self.values.len() < count {
            self.values.push(vec![0.0; count]);
        }
        for row in &mut self.values {
            row.resize(count, 0.0);
        }
    }

    /// Faction `victim_faction` loses reputation toward `killer_faction`. -1 per kill.
    pub fn on_kill(&mut self, killer_faction: i32, victim_faction: i32) {
        if killer_faction == victim_faction { return; }
        if let Some(row) = self.values.get_mut(victim_faction as usize) {
            if let Some(val) = row.get_mut(killer_faction as usize) {
                *val = (*val - 1.0).clamp(-9999.0, 9999.0);
            }
        }
    }

    /// Get faction a's opinion of faction b.
    pub fn get(&self, a: i32, b: i32) -> f32 {
        self.values.get(a as usize)
            .and_then(|row| row.get(b as usize))
            .copied()
            .unwrap_or(0.0)
    }
}

/// Towns that the LLM player is allowed to control via BRP write endpoints.
/// Populated from main menu AI slot config. Empty = no restrictions (legacy/debug).
#[derive(Resource, Default, Reflect)]
#[reflect(Resource)]
pub struct RemoteAllowedTowns {
    pub towns: Vec<usize>,
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

    /// Get raider index from faction (faction 2 = index 0, etc).
    /// AI factions start at index 2 (after Neutral=0, Player=1).
    pub fn faction_to_idx(faction: i32) -> Option<usize> {
        if faction > crate::constants::FACTION_PLAYER {
            Some((faction - 2) as usize)
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
    Inventory,
    Factions,
    Profiler,
    Help,
}

/// Active category in the pause-menu settings panel.
#[derive(Default, Clone, Copy, PartialEq, Eq)]
pub enum PauseSettingsTab {
    #[default]
    Interface,
    Video,
    Camera,
    Controls,
    Audio,
    Logs,
    Debug,
    LlmPlayer,
    SaveGame,
    LoadGame,
}

impl PauseSettingsTab {
    pub fn label(self) -> &'static str {
        match self {
            Self::Interface => "Interface",
            Self::Video => "Video",
            Self::Camera => "Camera",
            Self::Controls => "Controls",
            Self::Audio => "Audio",
            Self::Logs => "Logs",
            Self::Debug => "Debug",
            Self::LlmPlayer => "LLM Player",
            Self::SaveGame => "Save Game",
            Self::LoadGame => "Load Game",
        }
    }

    pub fn title_subtitle(self) -> (&'static str, &'static str) {
        match self {
            Self::Interface => ("Interface", "UI size, text readability, and display behavior."),
            Self::Video => ("Video", "Window resolution, vsync, and display behavior."),
            Self::Camera => ("Camera", "Panning, zoom speed, and sprite-detail transitions."),
            Self::Controls => ("Controls", "View and rebind keyboard shortcuts."),
            Self::Audio => ("Audio", "Music and sound effect levels."),
            Self::Logs => ("Logs", "Control what gets written to combat and activity logs."),
            Self::Debug => ("Debug", "Developer visibility and diagnostics toggles."),
            Self::LlmPlayer => ("LLM Player", "Claude command interval and payload inspector."),
            Self::SaveGame => ("Save Game", "Quicksave instantly or save manually by filename."),
            Self::LoadGame => ("Load Game", "Quickload or load a named/manual save file."),
        }
    }
}

/// Which UI panels are open. Toggled by keyboard shortcuts and HUD buttons.
#[derive(Resource)]
pub struct UiState {
    pub build_menu_open: bool,
    pub pause_menu_open: bool,
    pub pause_settings_tab: PauseSettingsTab,
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
    /// True when the player's fountain has been destroyed — shows lose screen.
    pub game_over: bool,
    /// Tower upgrade popup — Some(slot) when open for a specific tower.
    pub tower_upgrade_slot: Option<usize>,
    /// Casino blackjack popup open.
    pub casino_open: bool,
    /// Inventory slot filter — bitfield of enabled EquipmentSlot variants (all on by default).
    pub inv_slot_filter: u16,
    /// Inventory view mode: 0=Unequipped, 1=Equipped, 2=All.
    pub inv_view_mode: u8,
    /// Tech tree window open.
    pub tech_tree_open: bool,
    /// Tech tree active branch tab index.
    pub tech_tree_tab: usize,
    /// Inspector window currently visible (NPC or building selected).
    pub inspector_visible: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            build_menu_open: false,
            pause_menu_open: false,
            pause_settings_tab: PauseSettingsTab::default(),
            left_panel_open: false,
            left_panel_tab: LeftPanelTab::default(),
            combat_log_visible: true,
            assigning_mine: None,
            factions_overlay_faction: None,
            inspector_prefer_npc: true,
            inspector_click_seq: 0,
            game_over: false,
            tower_upgrade_slot: None,
            casino_open: false,
            inv_slot_filter: 0x1FF, // all 9 slots enabled
            inv_view_mode: 0,
            tech_tree_open: false,
            tech_tree_tab: 0,
            inspector_visible: false,
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
    /// Drag-line start slot in world grid coordinates (col, row).
    pub drag_start_slot: Option<(usize, usize)>,
    /// Drag-line current/end slot in world grid coordinates (col, row).
    pub drag_current_slot: Option<(usize, usize)>,
    /// Show the mouse-follow build hint sprite (hidden when snapped over a valid build slot).
    pub show_cursor_hint: bool,
    /// Bevy image handles for ghost preview sprites (populated by build_menu init).
    pub ghost_sprites: std::collections::HashMap<crate::world::BuildingKind, Handle<Image>>,
    /// Active build menu category tab.
    pub build_tab: crate::constants::DisplayCategory,
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
            build_tab: crate::constants::DisplayCategory::Economy,
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
    Llm,
    Chat,
}

impl CombatEventKind {
    const COUNT: usize = 10;

    fn index(self) -> usize {
        match self {
            Self::Kill => 0,
            Self::Spawn => 1,
            Self::Raid => 2,
            Self::Harvest => 3,
            Self::LevelUp => 4,
            Self::Ai => 5,
            Self::BuildingDamage => 6,
            Self::Loot => 7,
            Self::Llm => 8,
            Self::Chat => 9,
        }
    }
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

const COMBAT_LOG_PER_KIND: usize = 200;

/// Global combat event log. Per-kind ring buffers (200 each), newest at back.
#[derive(Resource)]
pub struct CombatLog {
    buffers: [VecDeque<CombatLogEntry>; CombatEventKind::COUNT],
}

impl Default for CombatLog {
    fn default() -> Self {
        Self {
            buffers: std::array::from_fn(|_| VecDeque::new()),
        }
    }
}

impl CombatLog {
    pub fn push(
        &mut self,
        kind: CombatEventKind,
        faction: i32,
        day: i32,
        hour: i32,
        minute: i32,
        message: String,
    ) {
        self.push_at(kind, faction, day, hour, minute, message, None);
    }

    pub fn push_at(
        &mut self,
        kind: CombatEventKind,
        faction: i32,
        day: i32,
        hour: i32,
        minute: i32,
        message: String,
        location: Option<bevy::math::Vec2>,
    ) {
        let buf = &mut self.buffers[kind.index()];
        if buf.len() >= COMBAT_LOG_PER_KIND {
            buf.pop_front();
        }
        buf.push_back(CombatLogEntry {
            day,
            hour,
            minute,
            kind,
            faction,
            message,
            location,
        });
    }

    pub fn iter_all(&self) -> impl Iterator<Item = &CombatLogEntry> {
        self.buffers.iter().flat_map(|b| b.iter())
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
    /// Per-slot cooldown for player/AI-built Tower buildings.
    pub tower_cooldowns: std::collections::HashMap<usize, f32>,
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
        Self {
            flags: vec![vec![false; count]; 16],
        }
    }
}

// ============================================================================
// TOWN POLICIES
// ============================================================================

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub enum WorkSchedule {
    #[default]
    Both,
    DayOnly,
    NightOnly,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Reflect, serde::Serialize, serde::Deserialize)]
pub enum OffDutyBehavior {
    #[default]
    GoToBed,
    StayAtFountain,
    WanderTown,
}

fn default_policy_mining_radius() -> f32 {
    crate::constants::DEFAULT_MINING_RADIUS
}

/// Per-town behavior configuration. Controls flee thresholds, work schedules, off-duty behavior.
#[derive(Clone, Debug, PartialEq, Reflect, serde::Serialize, serde::Deserialize)]
pub struct PolicySet {
    pub eat_food: bool,
    #[serde(alias = "guard_aggressive")]
    pub archer_aggressive: bool,
    #[serde(alias = "guard_leash")]
    pub archer_leash: bool,
    pub farmer_fight_back: bool,
    pub prioritize_healing: bool,
    pub farmer_flee_hp: f32, // 0.0-1.0 percentage
    #[serde(alias = "guard_flee_hp")]
    pub archer_flee_hp: f32,
    pub recovery_hp: f32, // 0.0-1.0 — go rest/heal when below this
    pub farmer_schedule: WorkSchedule,
    #[serde(alias = "guard_schedule")]
    pub archer_schedule: WorkSchedule,
    pub farmer_off_duty: OffDutyBehavior,
    #[serde(alias = "guard_off_duty")]
    pub archer_off_duty: OffDutyBehavior,
    #[serde(default = "default_policy_mining_radius")]
    pub mining_radius: f32,
    /// AI manager won't spend food below this amount.
    #[serde(default)]
    pub reserve_food: i32,
    /// AI manager won't spend gold below this amount.
    #[serde(default)]
    pub reserve_gold: i32,
    /// Equipment count that triggers NPC return-home to deposit loot.
    #[serde(default = "default_loot_threshold")]
    pub loot_threshold: usize,
}

fn default_loot_threshold() -> usize { 3 }

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
            reserve_food: 0,
            reserve_gold: 0,
            loot_threshold: 3,
        }
    }
}

/// Auto-mining cache and per-mine enable state.
#[derive(Resource, Default)]
pub struct MiningPolicy {
    /// Per-town discovered gold mine slots within policy radius.
    pub discovered_mines: Vec<Vec<usize>>,
    /// Per-gold-mine enabled toggle, keyed by EntityMap slot.
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
    pub raider_forage_hours: f32,
}

/// Game difficulty — scales building costs. Selected on main menu, immutable during play.
#[derive(
    Clone, Copy, PartialEq, Eq, Debug, Default, Resource, Reflect, serde::Serialize, serde::Deserialize,
)]
#[reflect(Resource)]
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
        let (farms, ai_towns, raider_towns, gold_mines, endless_mode, endless_strength, raider_forage_hours, overrides) =
            match self {
                Difficulty::Easy => (
                    4,
                    2,
                    2,
                    3,
                    true,
                    0.5,
                    12.0,
                    vec![(Job::Farmer, 4), (Job::Archer, 8), (Job::Raider, 0)],
                ),
                Difficulty::Normal => (
                    2,
                    5,
                    5,
                    2,
                    true,
                    0.75,
                    6.0,
                    vec![(Job::Farmer, 2), (Job::Archer, 4), (Job::Raider, 1)],
                ),
                Difficulty::Hard => (
                    1,
                    20,
                    20,
                    1,
                    true,
                    1.25,
                    3.0,
                    vec![(Job::Farmer, 0), (Job::Archer, 2), (Job::Raider, 2)],
                ),
            };
        // Start from registry defaults, then apply preset overrides
        let mut npc_counts: std::collections::BTreeMap<Job, usize> = crate::constants::NPC_REGISTRY
            .iter()
            .map(|d| (d.job, d.default_count))
            .collect();
        for (job, count) in overrides {
            npc_counts.insert(job, count);
        }
        DifficultyPreset {
            farms,
            ai_towns,
            raider_towns,
            gold_mines,
            npc_counts,
            endless_mode,
            endless_strength,
            raider_forage_hours,
        }
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
    /// NPC UIDs assigned to this squad (stable across slot reuse).
    pub members: Vec<crate::components::EntityUid>,
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
    pub fn is_player(&self) -> bool {
        self.owner == SquadOwner::Player
    }
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
            squads: (0..crate::constants::MAX_SQUADS)
                .map(|_| Squad::default())
                .collect(),
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
        self.squads.push(Squad {
            owner,
            ..Default::default()
        });
        idx
    }

    /// Iterate squads owned by a specific AI town.
    pub fn squads_for_town(&self, tdi: usize) -> impl Iterator<Item = (usize, &Squad)> {
        self.squads
            .iter()
            .enumerate()
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

impl Default for HelpCatalog {
    fn default() -> Self {
        Self::new()
    }
}

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
        m.insert("time", "Default: Space = pause/unpause. +/- = speed up/slow down (0x, 0.25x to 128x). 0x behaves as pause. Rebind in ESC > Settings > Controls.");

        // Left panel tabs
        m.insert("tab_roster", "Filter, sort, click to inspect. F to follow.");
        m.insert("tab_upgrades", "Spend food and gold on permanent upgrades.");
        m.insert(
            "tab_policies",
            "Work schedules, off-duty behavior, flee and aggro settings.",
        );
        m.insert(
            "tab_patrols",
            "Guard post patrol order. Use arrows to reorder.",
        );
        m.insert("tab_squads", "Set squad sizes and map targets. Default hotkeys are 1-9/0 (rebind in ESC > Settings > Controls).");
        m.insert("tab_inventory", "Equipment from defeated raiders. Select a military NPC, then click Equip.");
        m.insert(
            "tab_profiler",
            "Per-system timings. Enable in ESC > Settings > Debug.",
        );

        // Build menu
        m.insert(
            "build_farm",
            "Grows food over time. Build a Farmer Home nearby to assign a farmer to harvest it.",
        );
        m.insert(
            "build_farmer_home",
            "Spawns 1 farmer. Farmer works at the nearest free farm. Build farms first!",
        );
        m.insert(
            "build_archer_home",
            "Spawns 1 archer. Archer patrols nearby waypoints and fights enemies.",
        );
        m.insert(
            "build_waypoint",
            "Patrol waypoint for guards. Guards patrol between nearby waypoints and fight enemies.",
        );
        m.insert(
            "build_tent",
            "Spawns 1 raider. Raiders steal food from enemy farms and bring it back to their town.",
        );
        m.insert(
            "build_miner_home",
            "Spawns 1 miner. Miner works at the nearest gold mine.",
        );
        m.insert(
            "unlock_slot",
            "Pay food to unlock this grid slot. Then right-click it again to build.",
        );
        m.insert(
            "destroy",
            "Remove this building. Its NPC dies and the slot becomes empty.",
        );

        // Inspector (NPC)
        m.insert("npc_state", "What this NPC is currently doing. Working = at their job. Resting = recovering energy at home. Fighting = in combat.");
        m.insert("npc_energy", "Energy drains while active, recovers while resting at home. NPCs go rest when energy drops below 50, resume at 80.");
        m.insert("npc_trait", "Personality trait. 40% of NPCs spawn with one. Brave = never flees. Swift = +25% speed. Hardy = +25% HP.");
        m.insert(
            "npc_level",
            "Archers level up from kills. +1% all stats per level. XP needed = (level+1)^2 x 100.",
        );

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
        Self {
            enabled: false,
            strength_fraction: 0.75,
            pending_spawns: Vec::new(),
        }
    }
}

/// Pre-computed healing zone per town, indexed by faction for O(1) lookup.
pub struct HealingZone {
    pub center: Vec2,
    pub enter_radius_sq: f32,
    pub exit_radius_sq: f32,
    pub heal_rate: f32,
    pub town_idx: usize,
    pub faction: i32,
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

/// Tracks NPC slots currently in a healing zone. Sustain-check iterates only these.
#[derive(Resource)]
pub struct ActiveHealingSlots {
    pub slots: Vec<usize>,
    pub mark: Vec<u8>,
}

impl Default for ActiveHealingSlots {
    fn default() -> Self {
        Self {
            slots: Vec::new(),
            mark: vec![0u8; crate::constants::MAX_ENTITIES],
        }
    }
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
    /// SFX variant handles keyed by kind — multiple variants per kind for random selection.
    pub sfx_handles: std::collections::HashMap<SfxKind, Vec<Handle<AudioSource>>>,
    /// Whether arrow shoot SFX plays (disabled by default — the sound is rough).
    pub sfx_shoot_enabled: bool,
}

impl Default for GameAudio {
    fn default() -> Self {
        Self {
            music_volume: 0.3,
            sfx_volume: 0.15,
            tracks: Vec::new(),
            last_track: None,
            loop_current: false,
            play_next: None,
            music_speed: 1.0,
            sfx_handles: std::collections::HashMap::new(),
            sfx_shoot_enabled: false,
        }
    }
}

/// Marker component for the currently playing music entity.
#[derive(Component)]
pub struct MusicTrack;

/// Sound effect categories.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum SfxKind {
    ArrowShoot,
    Death,
    Build,
    Click,
    Upgrade,
}

/// Fire-and-forget SFX trigger message. Position enables spatial culling (None = always play).
#[derive(Message, Clone)]
pub struct PlaySfxMsg {
    pub kind: SfxKind,
    pub position: Option<Vec2>,
}

// ============================================================================
// CHAT INBOX (LLM ↔ Player messaging)
// ============================================================================

pub struct ChatMessage {
    pub from_town: usize,
    pub to_town: usize,
    pub text: String,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    /// Whether this message has been included in an LLM state payload.
    pub sent_to_llm: bool,
    /// Whether a reply exists for this message.
    pub has_reply: bool,
}

const CHAT_CAPACITY: usize = 200;

#[derive(Resource, Default)]
pub struct ChatInbox {
    pub messages: std::collections::VecDeque<ChatMessage>,
}

impl ChatInbox {
    pub fn push(&mut self, msg: ChatMessage) {
        if self.messages.len() >= CHAT_CAPACITY {
            self.messages.pop_front();
        }
        self.messages.push_back(msg);
    }
}

// Test12 relocated to src/tests/vertical_slice.rs — uses shared TestState resource.
