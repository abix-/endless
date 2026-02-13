//! ECS Resources - Shared state accessible by all systems

use bevy::prelude::*;
use bevy::render::extract_resource::ExtractResource;
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use crate::constants::MAX_NPC_COUNT;

/// Per-system profiling (Factorio-style). RAII guard pattern: `let _t = timings.scope("name");`
/// Uses Res<SystemTimings> (not ResMut) with internal Mutex so parallel systems don't serialize.
#[derive(Resource)]
pub struct SystemTimings {
    data: Mutex<HashMap<&'static str, f32>>,
    frame_start: Mutex<Option<std::time::Instant>>,
    pub frame_ms: Mutex<f32>,
    pub enabled: bool,
}

impl Default for SystemTimings {
    fn default() -> Self {
        Self {
            data: Mutex::new(HashMap::new()),
            frame_start: Mutex::new(None),
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

    pub fn begin_frame(&self) {
        if self.enabled {
            if let Ok(mut start) = self.frame_start.lock() {
                *start = Some(std::time::Instant::now());
            }
        }
    }

    pub fn end_frame(&self) {
        if self.enabled {
            if let Ok(start) = self.frame_start.lock() {
                if let Some(s) = *start {
                    let ms = s.elapsed().as_secs_f64() as f32 * 1000.0;
                    if let Ok(mut fm) = self.frame_ms.lock() {
                        *fm = *fm * 0.95 + ms * 0.05;
                    }
                }
            }
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
            farmers_per_town: 2,
            guards_per_town: 500,
            raiders_per_camp: 500,
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

/// Currently selected building (grid cell). `active = false` means no building selected.
#[derive(Resource, Default)]
pub struct SelectedBuilding {
    pub col: usize,
    pub row: usize,
    pub active: bool,
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
    /// High-water mark: max slot index ever allocated. Use for GPU dispatch bounds.
    pub fn count(&self) -> usize { self.next }
    /// Currently alive: allocated minus freed. Use for UI display counts.
    pub fn alive(&self) -> usize { self.next - self.free.len() }
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

/// GPU readback state. Populated by ReadbackComplete observers, read by Bevy systems.
/// Clone + ExtractResource so render world can access positions for instanced rendering.
#[derive(Resource, Clone, ExtractResource)]
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

/// GPU→CPU readback of projectile hit results. Each entry is [npc_idx, processed].
/// Populated by ReadbackComplete observer, read by process_proj_hits.
#[derive(Resource, Default)]
pub struct ProjHitState(pub Vec<[i32; 2]>);

/// GPU→CPU readback of projectile positions. [x0, y0, x1, y1, ...] flattened.
/// Populated by ReadbackComplete observer, read by prepare_proj_buffers (render world).
#[derive(Resource, Default, Clone, ExtractResource)]
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

/// Per-mine gold tracking. Mirrors FarmStates pattern.
#[derive(Resource, Default, Clone)]
pub struct MineStates {
    pub gold: Vec<f32>,      // Current gold in each mine
    pub max_gold: Vec<f32>,  // Max capacity per mine
    pub positions: Vec<Vec2>, // World positions (for render/lookup)
}

impl MineStates {
    pub fn push_mine(&mut self, pos: Vec2, max_gold: f32) {
        self.gold.push(max_gold);
        self.max_gold.push(max_gold);
        self.positions.push(pos);
    }
}

/// Farm growth state.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub enum FarmGrowthState {
    #[default]
    Growing,  // Crops growing, progress accumulating
    Ready,    // Ready to harvest, shows food icon
}

/// Per-farm growth tracking. Extracted to render world for instanced farm sprites.
#[derive(Resource, Default, Clone, ExtractResource)]
pub struct FarmStates {
    pub states: Vec<FarmGrowthState>,  // Per-farm state
    pub progress: Vec<f32>,            // Growth progress 0.0-1.0
    pub positions: Vec<Vec2>,          // World positions (for render)
}

impl FarmStates {
    pub fn push_farm(&mut self, pos: Vec2) {
        self.states.push(FarmGrowthState::Growing);
        self.progress.push(0.0);
        self.positions.push(pos);
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
    Intel,
    Profiler,
}

/// Which UI panels are open. Toggled by keyboard shortcuts and HUD buttons.
#[derive(Resource)]
pub struct UiState {
    pub build_menu_open: bool,
    pub pause_menu_open: bool,
    pub left_panel_open: bool,
    pub left_panel_tab: LeftPanelTab,
    pub combat_log_visible: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            build_menu_open: false,
            pause_menu_open: false,
            left_panel_open: false,
            left_panel_tab: LeftPanelTab::default(),
            combat_log_visible: true,
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

/// Context for the build menu popup — populated by slot_right_click_system.
#[derive(Resource, Default)]
pub struct BuildMenuContext {
    /// Which villager town grid (index into TownGrids.grids).
    pub grid_idx: Option<usize>,
    /// Which town in WorldData.towns.
    pub town_data_idx: Option<usize>,
    /// Grid slot (row, col) relative to town center.
    pub slot: Option<(i32, i32)>,
    /// World position of the slot center.
    pub slot_world_pos: Vec2,
    /// Screen position where right-click occurred (for menu placement).
    pub screen_pos: [f32; 2],
    /// True if the slot is locked (show Unlock button).
    pub is_locked: bool,
    /// True if the slot already has a building.
    pub has_building: bool,
    /// True if the slot is the fountain (indestructible).
    pub is_fountain: bool,
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
}

/// A single combat log entry.
#[derive(Clone)]
pub struct CombatLogEntry {
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
    pub kind: CombatEventKind,
    pub message: String,
}

const COMBAT_LOG_MAX: usize = 200;

/// Global combat event log. Ring buffer, newest at back.
#[derive(Resource, Default)]
pub struct CombatLog {
    pub entries: VecDeque<CombatLogEntry>,
}

impl CombatLog {
    pub fn push(&mut self, kind: CombatEventKind, day: i32, hour: i32, minute: i32, message: String) {
        if self.entries.len() >= COMBAT_LOG_MAX {
            self.entries.pop_front();
        }
        self.entries.push_back(CombatLogEntry { day, hour, minute, kind, message });
    }
}

// ============================================================================
// GUARD POST TURRET STATE
// ============================================================================

/// Per-guard-post turret state. Length auto-syncs with WorldData.guard_posts.
#[derive(Resource, Default)]
pub struct GuardPostState {
    /// Cooldown timer per post (seconds remaining).
    pub timers: Vec<f32>,
    /// Whether auto-attack is enabled per post.
    pub attack_enabled: Vec<bool>,
}

// ============================================================================
// BUILDING SPAWNERS
// ============================================================================

/// Tracks one building spawner (House, Barracks, or Tent) and its linked NPC.
#[derive(Clone, Default)]
pub struct SpawnerEntry {
    pub building_kind: i32,   // 0=House (farmer), 1=Barracks (guard), 2=Tent (raider)
    pub town_idx: i32,        // town data index (villager or raider camp)
    pub position: Vec2,       // building world position
    pub npc_slot: i32,        // linked NPC slot (-1 = no NPC alive)
    pub respawn_timer: f32,   // game hours remaining (-1 = not respawning)
}

/// All building spawners in the world. Each House/Barracks gets one entry.
#[derive(Resource, Default)]
pub struct SpawnerState(pub Vec<SpawnerEntry>);

/// Per-town auto-upgrade flags. When enabled, upgrades are purchased automatically
/// once per game hour whenever the town has enough food.
#[derive(Resource)]
pub struct AutoUpgrade {
    pub flags: Vec<[bool; crate::systems::stats::UPGRADE_COUNT]>,
}

impl Default for AutoUpgrade {
    fn default() -> Self {
        Self { flags: vec![[false; crate::systems::stats::UPGRADE_COUNT]; 16] }
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

/// Per-town behavior configuration. Controls flee thresholds, work schedules, off-duty behavior.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct PolicySet {
    pub eat_food: bool,
    pub guard_aggressive: bool,
    pub guard_leash: bool,
    pub farmer_fight_back: bool,
    pub prioritize_healing: bool,
    pub farmer_flee_hp: f32,     // 0.0-1.0 percentage
    pub guard_flee_hp: f32,
    pub recovery_hp: f32,        // 0.0-1.0 — go rest/heal when below this
    pub farmer_schedule: WorkSchedule,
    pub guard_schedule: WorkSchedule,
    pub farmer_off_duty: OffDutyBehavior,
    pub guard_off_duty: OffDutyBehavior,
    pub mining_pct: f32, // 0.0-1.0 — fraction of idle farmers that choose mining over farming
}

impl Default for PolicySet {
    fn default() -> Self {
        Self {
            eat_food: true,
            guard_aggressive: false,
            guard_leash: true,
            farmer_fight_back: false,
            prioritize_healing: true,
            farmer_flee_hp: 0.30,
            guard_flee_hp: 0.15,
            recovery_hp: 0.80,
            farmer_schedule: WorkSchedule::Both,
            guard_schedule: WorkSchedule::Both,
            farmer_off_duty: OffDutyBehavior::GoToBed,
            guard_off_duty: OffDutyBehavior::GoToBed,
            mining_pct: 0.0,
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

/// A player-controlled squad of guards.
#[derive(Clone, Default)]
pub struct Squad {
    /// NPC slot indices of guards in this squad.
    pub members: Vec<usize>,
    /// Squad target position. None = no target, guards patrol normally.
    pub target: Option<Vec2>,
}

/// All squads + UI state. 10 squads, pre-initialized.
#[derive(Resource)]
pub struct SquadState {
    pub squads: Vec<Squad>,
    /// Currently selected squad in UI (-1 = none).
    pub selected: i32,
    /// When true, next left-click sets the selected squad's target.
    pub placing_target: bool,
}

impl Default for SquadState {
    fn default() -> Self {
        Self {
            squads: (0..crate::constants::MAX_SQUADS).map(|_| Squad::default()).collect(),
            selected: -1,
            placing_target: false,
        }
    }
}

// Test12 relocated to src/tests/vertical_slice.rs — uses shared TestState resource.
