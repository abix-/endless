# npc_manager.gd
# Orchestrates NPC systems, owns data arrays
extends Node2D

# Re-export enums for external access (main.gd uses these)
enum State { IDLE, WALKING, RESTING, WORKING, WANDERING, FIGHTING, FLEEING }
enum Faction { VILLAGER, RAIDER }
enum Job { FARMER, GUARD, RAIDER }

signal npc_leveled_up(npc_index: int, job: int, old_level: int, new_level: int)
@warning_ignore("unused_signal")  # Emitted from npc_needs.gd
signal raider_delivered_food(town_idx: int)
@warning_ignore("unused_signal")  # Emitted from npc_needs.gd
signal npc_ate_food(npc_index: int, town_idx: int, job: int, hp_before: float, energy_before: float, hp_after: float)
@warning_ignore("unused_signal")  # Emitted from npc_combat.gd
signal npc_died(npc_index: int, job: int, level: int, town_idx: int, killer_job: int, killer_level: int)
@warning_ignore("unused_signal")  # Emitted from main.gd
signal npc_spawned(npc_index: int, job: int, town_idx: int)

const Location = preload("res://world/location.gd")
const MAX_LEVEL := 9999

# Arrival radii - edge-based, for entering sprite boundary (cached at load)
var _arrival_farm: float
var _arrival_home: float
var _arrival_camp: float
var _arrival_guard_post: float

# Scaling functions
static func get_stat_scale(level: int) -> float:
	return sqrt(float(level))  # Level 1 = 1x, Level 9999 = 100x

static func get_size_scale(level: int) -> float:
	# Level 1 = 1x, Level 9999 = 50x
	return 1.0 + (sqrt(float(level)) - 1.0) * 0.495


func get_npc_size_scale(i: int) -> float:
	return get_size_scale(levels[i]) * (1.0 + size_bonuses[i])


static func get_xp_for_next_level(level: int) -> int:
	return level  # Need 'level' XP to go from level to level+1

# World info (set by main.gd)
var village_center := Vector2.ZERO
var farm_positions: Array[Vector2] = []
var guard_posts_by_town: Array[Array] = []  # Per-town arrays of guard post positions
var town_centers: Array[Vector2] = []  # Fountain at center of each town
var town_upgrades: Array = []  # Reference to main's upgrade data
var town_policies: Array = []  # Reference to main's policy data
var town_food: PackedInt32Array  # Reference to main's town food (set by main.gd)
var beds_by_town: Array[Array] = []  # Per-town arrays of bed positions
var bed_occupants: Array[PackedInt32Array] = []  # Per-town: bed index -> NPC index (-1 = free)
var farms_by_town: Array[Array] = []  # Per-town arrays of farm positions
var farm_occupant_counts: Array[PackedInt32Array] = []  # Per-town: farm index -> count of farmers (0-4)
var camp_food: PackedInt32Array  # Reference to main's camp food (set by main.gd)

# Data arrays
var count := 0
var max_count := Config.MAX_NPC_COUNT
var _free_slots: Array[int] = []  # Reusable slots from dead NPCs

var positions: PackedVector2Array
var velocities: PackedVector2Array
var targets: PackedVector2Array
var wander_centers: PackedVector2Array

var healths: PackedFloat32Array
var max_healths: PackedFloat32Array
var energies: PackedFloat32Array
var attack_damages: PackedFloat32Array
var attack_ranges: PackedFloat32Array
var attack_timers: PackedFloat32Array
var scan_timers: PackedFloat32Array
var arrival_radii: PackedFloat32Array

var states: PackedInt32Array
var factions: PackedInt32Array
var jobs: PackedInt32Array
var current_targets: PackedInt32Array
var will_flee: PackedInt32Array
var recovering: PackedInt32Array  # NPC is healing after fleeing, stays until 75% HP
var works_at_night: PackedInt32Array
var health_dirty: PackedInt32Array
var last_rendered: PackedInt32Array
var flash_timers: PackedFloat32Array

var levels: PackedInt32Array
var xp: PackedInt32Array
var carrying_food: PackedInt32Array  # Raiders carrying stolen food
var town_indices: PackedInt32Array  # Which town/camp this NPC belongs to
var size_bonuses: PackedFloat32Array  # Size bonus from upgrades
var current_bed_idx: PackedInt32Array  # Which bed this NPC is using (-1 = none)
var current_farm_idx: PackedInt32Array  # Which farm this farmer is assigned to (-1 = none)

var home_positions: PackedVector2Array
var work_positions: PackedVector2Array
var spawn_positions: PackedVector2Array

# Guard patrol data
var patrol_target_idx: PackedInt32Array   # Current target post index in town's guard_posts
var patrol_last_idx: PackedInt32Array     # Last visited post index (-1 = none)
var patrol_timer: PackedInt32Array        # Minutes waited at current post

# Identity
var npc_names: Array[String] = []         # NPC names for attachment/display
var traits: PackedInt32Array              # NPCState.Trait values

# Predicted movement (Factorio-style optimization)
var last_logic_frame: PackedInt32Array    # Frame when logic was last calculated
var intended_velocities: PackedVector2Array  # Cached velocity from last logic update

# Entity sleeping (Factorio-style optimization)
var awake: PackedByteArray  # 1 = awake, 0 = sleeping
var sleep_timers: PackedFloat32Array  # Delay before sleeping after leaving active zone
const SLEEP_DELAY := 2.0  # Seconds before NPC sleeps
const ACTIVE_RADIUS := 1500.0  # Pixels from camera center
const ACTIVE_RADIUS_SQ := ACTIVE_RADIUS * ACTIVE_RADIUS

# Parallel processing (double-buffer pattern)
var use_parallel := true  # Toggle for fallback
var use_gpu_separation := false  # GPU compute for separation (experimental)
var next_positions: PackedVector2Array  # Write buffer for positions
var pending_damage: PackedFloat32Array  # Accumulated damage per NPC
var _death_queue: Array[int] = []  # NPCs to process death after parallel phase
var _damage_mutex: Mutex  # Protects pending_damage writes
var _cached_delta: float = 0.0  # Delta cached for parallel tasks
var _cached_frame: int = 0  # Frame counter for staggering
var _gpu_separation: GPUSeparation  # GPU compute shader manager

# Selection
var selected_npc := -1

# Rendering
@onready var multimesh_instance: MultiMeshInstance2D = $MultiMeshInstance2D
@onready var loot_icon_instance: MultiMeshInstance2D = $LootIconMultiMesh
@onready var halo_instance: MultiMeshInstance2D = $HaloMultiMesh
@onready var sleep_icon_instance: MultiMeshInstance2D = $SleepIconMultiMesh
@onready var info_label: Label = $InfoLabel

# Stats - alive counts
var alive_farmers := 0
var alive_guards := 0
var alive_raiders := 0

# Stats - totals
var total_farmers := 0
var total_guards := 0
var total_raiders := 0

# Stats - kills
var villager_kills := 0
var raider_kills := 0

# Stats - dead awaiting respawn
var dead_farmers := 0
var dead_guards := 0
var dead_raiders := 0

# Performance tracking
var last_loop_time := 0.0

# Systems
var _state: NPCState
var _nav: NPCNavigation
var _combat: NPCCombat
var _needs: NPCNeeds
var _grid: NPCGrid
var _renderer: NPCRenderer
var _projectiles: Node  # Set by main.gd
var _guard_post_combat: GuardPostCombat  # Set by set_main_reference


func set_projectile_manager(pm: Node) -> void:
	_projectiles = pm


func set_main_reference(main_node: Node) -> void:
	_guard_post_combat = GuardPostCombat.new(self, main_node)


func _ready() -> void:
	add_to_group("npc_manager")
	_damage_mutex = Mutex.new()
	_init_radii()
	_init_arrays()
	_init_systems()
	WorldClock.time_tick.connect(_on_time_tick)


func _init_radii() -> void:
	_arrival_farm = Location.get_arrival_radius("field")
	_arrival_home = Location.get_arrival_radius("home")
	_arrival_camp = Location.get_arrival_radius("camp")
	_arrival_guard_post = Location.get_arrival_radius("guard_post")


func _init_arrays() -> void:
	positions.resize(max_count)
	velocities.resize(max_count)
	targets.resize(max_count)
	wander_centers.resize(max_count)
	home_positions.resize(max_count)
	work_positions.resize(max_count)
	spawn_positions.resize(max_count)

	healths.resize(max_count)
	max_healths.resize(max_count)
	energies.resize(max_count)
	attack_damages.resize(max_count)
	attack_ranges.resize(max_count)
	attack_timers.resize(max_count)
	scan_timers.resize(max_count)
	arrival_radii.resize(max_count)

	states.resize(max_count)
	factions.resize(max_count)
	jobs.resize(max_count)
	current_targets.resize(max_count)
	will_flee.resize(max_count)
	recovering.resize(max_count)
	works_at_night.resize(max_count)
	health_dirty.resize(max_count)
	last_rendered.resize(max_count)
	flash_timers.resize(max_count)
	levels.resize(max_count)
	xp.resize(max_count)
	carrying_food.resize(max_count)
	town_indices.resize(max_count)
	size_bonuses.resize(max_count)
	current_bed_idx.resize(max_count)
	current_farm_idx.resize(max_count)
	patrol_target_idx.resize(max_count)
	patrol_last_idx.resize(max_count)
	patrol_timer.resize(max_count)
	last_logic_frame.resize(max_count)
	intended_velocities.resize(max_count)
	npc_names.resize(max_count)
	traits.resize(max_count)
	awake.resize(max_count)
	sleep_timers.resize(max_count)
	next_positions.resize(max_count)
	pending_damage.resize(max_count)

	for i in max_count:
		awake[i] = 1  # Start awake
		sleep_timers[i] = SLEEP_DELAY
		patrol_last_idx[i] = -1
		current_bed_idx[i] = -1
		current_farm_idx[i] = -1
		npc_names[i] = ""
		traits[i] = NPCState.Trait.NONE


func _init_systems() -> void:
	_grid = NPCGrid.new(self)
	_renderer = NPCRenderer.new(self, multimesh_instance, loot_icon_instance, halo_instance, sleep_icon_instance)
	_state = NPCState.new(self)
	_nav = NPCNavigation.new(self)
	_combat = NPCCombat.new(self)
	_needs = NPCNeeds.new(self)

	# Initialize GPU compute (optional, may fail on incompatible systems)
	_gpu_separation = GPUSeparation.new()
	if _gpu_separation.initialize():
		print("GPU separation initialized successfully")
	else:
		print("GPU separation unavailable, using CPU")
		use_gpu_separation = false

	_nav.arrived.connect(_on_npc_arrived)


func _exit_tree() -> void:
	if _gpu_separation:
		_gpu_separation.cleanup()


# ============================================================
# ENTITY SLEEPING
# ============================================================

func _update_sleep_status(delta: float) -> void:
	var cam_pos := Vector2.ZERO
	var camera: Camera2D = get_viewport().get_camera_2d()
	if camera:
		cam_pos = camera.global_position

	for i in count:
		if healths[i] <= 0:
			continue

		var should_stay_awake := _should_stay_awake(i)
		var pos: Vector2 = positions[i]
		var dx: float = pos.x - cam_pos.x
		var dy: float = pos.y - cam_pos.y
		var in_range: bool = (dx * dx + dy * dy) < ACTIVE_RADIUS_SQ

		if should_stay_awake or in_range:
			awake[i] = 1
			sleep_timers[i] = SLEEP_DELAY
		else:
			sleep_timers[i] -= delta
			if sleep_timers[i] <= 0:
				awake[i] = 0


func _should_stay_awake(i: int) -> bool:
	var state: int = states[i]
	# Combat states always awake
	if state == NPCState.State.FIGHTING or state == NPCState.State.FLEEING:
		return true
	# Active raider states always awake
	if state == NPCState.State.RAIDING or state == NPCState.State.RETURNING:
		return true
	return false


func wake_npc(i: int) -> void:
	awake[i] = 1
	sleep_timers[i] = SLEEP_DELAY


# ============================================================
# PARALLEL PROCESSING HELPERS
# ============================================================

func add_pending_damage(target: int, damage: float) -> void:
	_damage_mutex.lock()
	pending_damage[target] += damage
	_damage_mutex.unlock()


func queue_death(i: int) -> void:
	_damage_mutex.lock()
	if i not in _death_queue:
		_death_queue.append(i)
	_damage_mutex.unlock()


func apply_deferred_changes() -> void:
	# Apply pending damage and check for deaths
	for i in count:
		if pending_damage[i] > 0:
			healths[i] -= pending_damage[i]
			flash_timers[i] = 0.15
			health_dirty[i] = 1
			pending_damage[i] = 0.0

			if healths[i] <= 0:
				healths[i] = 0
				if i not in _death_queue:
					_death_queue.append(i)

	# Process deaths (single-threaded, safe)
	for i in _death_queue:
		if healths[i] <= 0:
			_combat._die(i, -1)
	_death_queue.clear()

	# Copy next_positions to positions (swap buffers)
	for i in count:
		if healths[i] > 0 and awake[i] == 1:
			positions[i] = next_positions[i]


# ============================================================
# GPU COMPUTE
# ============================================================

func _kick_gpu_separation() -> void:
	_grid.rebuild_gpu_grid()
	_gpu_separation.kick(
		positions,
		_nav.cached_sizes,
		healths,
		states,
		targets,
		_grid.gpu_grid_counts,
		_grid.gpu_grid_data,
		count,
		_grid.gpu_grid_width,
		_grid.gpu_grid_height,
		NPCGrid.GPU_MAX_PER_CELL,
		NPCGrid.CELL_SIZE,
		Config.SEPARATION_RADIUS,
		Config.SEPARATION_STRENGTH
	)


func _apply_gpu_result() -> void:
	var result: PackedVector2Array = _gpu_separation.get_result()
	var apply_count: int = mini(result.size(), count)
	for i in apply_count:
		_nav.separation_velocities[i] = result[i]


# ============================================================
# UNIFIED PARALLEL PROCESSING
# ============================================================

func _process_unified_parallel() -> void:
	_cached_frame = Engine.get_process_frames()
	var task_id = WorkerThreadPool.add_group_task(_process_single_npc_unified, count)
	WorkerThreadPool.wait_for_group_task_completion(task_id)


func _process_single_npc_unified(i: int) -> void:
	if healths[i] <= 0.0:
		return
	if awake[i] == 0:
		return

	# Phase 1: Scan for targets (staggered every 16 frames)
	_combat._scan_single_npc_internal(i, _cached_frame)

	# Phase 2: Combat processing
	_combat._process_single_npc_combat_internal(i, _cached_delta, _cached_frame)

	# Phase 3: Navigation
	_nav._process_single_npc_nav_internal(i, _cached_delta, _cached_frame)


# ============================================================
# MAIN LOOP
# ============================================================

var profile_sleep := 0.0
var profile_grid := 0.0
var profile_gpu_sep := 0.0
var profile_scan := 0.0
var profile_combat := 0.0
var profile_nav := 0.0
var profile_projectiles := 0.0
var profile_render := 0.0

func _process(delta: float) -> void:
	var t1 := Time.get_ticks_usec()
	var profiling: bool = UserSettings.perf_metrics
	var t := t1

	_cached_delta = delta

	_update_sleep_status(delta)
	if profiling:
		var t2 := Time.get_ticks_usec()
		profile_sleep = (t2 - t) / 1000.0
		t = t2

	_grid.rebuild()
	if profiling:
		var t2 := Time.get_ticks_usec()
		profile_grid = (t2 - t) / 1000.0
		t = t2

	# GPU separation: apply last frame's result, kick new computation
	if use_gpu_separation and _gpu_separation.is_initialized:
		_apply_gpu_result()
		_kick_gpu_separation()
		if profiling:
			var t2 := Time.get_ticks_usec()
			profile_gpu_sep = (t2 - t) / 1000.0
			t = t2
	else:
		profile_gpu_sep = 0.0

	if use_parallel:
		# Unified parallel processing - single pass for scan+combat+nav
		_process_unified_parallel()
		if profiling:
			var t2 := Time.get_ticks_usec()
			var total_parallel := (t2 - t) / 1000.0
			# Split evenly for display (actual work is combined)
			profile_scan = total_parallel * 0.15
			profile_combat = total_parallel * 0.25
			profile_nav = total_parallel * 0.60
			t = t2

		# Apply deferred changes (damage, deaths, position swap)
		apply_deferred_changes()

		# Fire queued projectiles from parallel combat
		_combat.fire_queued_projectiles()

		# Handle state changes for NPCs that found targets during parallel scan
		_process_pending_state_changes()
	else:
		# Single-threaded path (fallback)
		_combat.process_scanning(delta)
		if profiling:
			var t2 := Time.get_ticks_usec()
			profile_scan = (t2 - t) / 1000.0
			t = t2

		_combat.process(delta)
		if profiling:
			var t2 := Time.get_ticks_usec()
			profile_combat = (t2 - t) / 1000.0
			t = t2

		_nav.process(delta, profiling)
		if profiling:
			var t2 := Time.get_ticks_usec()
			profile_nav = (t2 - t) / 1000.0
			t = t2

	if _projectiles:
		_projectiles.process(delta)
	if profiling:
		var t2 := Time.get_ticks_usec()
		profile_projectiles = (t2 - t) / 1000.0
		t = t2

	if _guard_post_combat:
		_guard_post_combat.process(delta)

	_renderer.update(delta)

	var t_end := Time.get_ticks_usec()
	if profiling:
		profile_render = (t_end - t) / 1000.0
	last_loop_time = (t_end - t1) / 1000.0

	_update_selection()

	if Engine.get_process_frames() % 30 == 0:
		_update_counts()


func _process_pending_state_changes() -> void:
	# Handle NPCs that found targets during parallel scan
	# (parallel scan only sets current_targets, doesn't change state)
	for i in count:
		if healths[i] <= 0:
			continue
		if current_targets[i] < 0:
			continue
		# Has target but not in combat state - needs state change
		var state: int = states[i]
		if state != NPCState.State.FIGHTING and state != NPCState.State.FLEEING:
			var job: int = jobs[i]
			if job == NPCState.Job.FARMER:
				_state.set_state(i, NPCState.State.FLEEING)
			else:
				_state.set_state(i, NPCState.State.FIGHTING)
			_nav.force_logic_update(i)


func _update_counts() -> void:
	alive_farmers = 0
	alive_guards = 0
	alive_raiders = 0
	dead_farmers = 0
	dead_guards = 0
	dead_raiders = 0

	for i in count:
		var job: int = jobs[i]
		if healths[i] > 0:
			match job:
				Job.FARMER: alive_farmers += 1
				Job.GUARD: alive_guards += 1
				Job.RAIDER: alive_raiders += 1
		else:
			match job:
				Job.FARMER: dead_farmers += 1
				Job.GUARD: dead_guards += 1
				Job.RAIDER: dead_raiders += 1


# ============================================================
# SPAWNING
# ============================================================

func spawn_npc(job: int, faction: int, pos: Vector2, home_pos: Vector2, work_pos: Vector2, night_worker: bool, flee: bool, hp: float, damage: float, attack_range: float, town_idx: int = -1) -> int:
	var i: int
	if not _free_slots.is_empty():
		# Reuse dead slot
		i = _free_slots.pop_back()
		_nav.reset_slot(i)
	else:
		# Append new slot
		if count >= max_count:
			return -1
		i = count
		count += 1

	positions[i] = pos
	velocities[i] = Vector2.ZERO
	intended_velocities[i] = Vector2.ZERO
	targets[i] = pos
	wander_centers[i] = pos
	home_positions[i] = home_pos
	work_positions[i] = work_pos
	spawn_positions[i] = pos

	# Set initial arrival radius (home building)
	match job:
		Job.FARMER, Job.GUARD:
			arrival_radii[i] = _arrival_home
		Job.RAIDER:
			arrival_radii[i] = _arrival_camp

	healths[i] = hp
	max_healths[i] = hp
	energies[i] = Config.ENERGY_MAX
	attack_damages[i] = damage
	attack_ranges[i] = attack_range
	attack_timers[i] = 0.0
	scan_timers[i] = randf() * Config.SCAN_INTERVAL
	flash_timers[i] = 0.0

	states[i] = State.IDLE
	factions[i] = faction
	jobs[i] = job
	current_targets[i] = -1
	will_flee[i] = 1 if flee else 0
	works_at_night[i] = 1 if night_worker else 0
	health_dirty[i] = 1
	last_rendered[i] = 0
	levels[i] = 1
	xp[i] = 0
	carrying_food[i] = 0
	town_indices[i] = town_idx
	size_bonuses[i] = 0.0
	recovering[i] = 0

	# Guard patrol initialization
	patrol_target_idx[i] = 0
	patrol_last_idx[i] = -1
	patrol_timer[i] = 0

	# Assign name and trait
	var first: String = NPCState.FIRST_NAMES[randi() % NPCState.FIRST_NAMES.size()]
	var last: String = NPCState.LAST_NAMES[randi() % NPCState.LAST_NAMES.size()]
	npc_names[i] = "%s %s" % [first, last]
	traits[i] = _roll_trait()

	# Apply trait effects
	if traits[i] == NPCState.Trait.HARDY:
		max_healths[i] *= 1.25
		healths[i] = max_healths[i]
	elif traits[i] == NPCState.Trait.SHARPSHOT:
		attack_ranges[i] *= 1.25

	# Apply town upgrades for guards
	if job == Job.GUARD and town_idx >= 0 and town_idx < town_upgrades.size():
		var upgrades: Dictionary = town_upgrades[town_idx]
		if upgrades.guard_health > 0:
			var bonus: float = Config.UPGRADE_GUARD_HEALTH_BONUS * upgrades.guard_health
			max_healths[i] = Config.GUARD_HP * (1.0 + bonus)
			healths[i] = max_healths[i]
		if upgrades.guard_attack > 0:
			var bonus: float = Config.UPGRADE_GUARD_ATTACK_BONUS * upgrades.guard_attack
			attack_damages[i] = Config.GUARD_DAMAGE * (1.0 + bonus)
		if upgrades.guard_range > 0:
			var bonus: float = Config.UPGRADE_GUARD_RANGE_BONUS * upgrades.guard_range
			attack_ranges[i] = Config.GUARD_RANGE * (1.0 + bonus)
		if upgrades.guard_size > 0:
			size_bonuses[i] = Config.UPGRADE_GUARD_SIZE_BONUS * upgrades.guard_size

	match job:
		Job.FARMER: total_farmers += 1
		Job.GUARD: total_guards += 1
		Job.RAIDER: total_raiders += 1

	_renderer.set_npc_sprite(i, job)
	_renderer.set_visible_count(count)
	_nav.update_cached_size(i)

	_decide_what_to_do(i)

	return i


func spawn_farmer(pos: Vector2, home_pos: Vector2, work_pos: Vector2, town_idx: int) -> int:
	return spawn_npc(Job.FARMER, Faction.VILLAGER, pos, home_pos, work_pos, false, true, Config.FARMER_HP, Config.FARMER_DAMAGE, Config.FARMER_RANGE, town_idx)


func spawn_guard(pos: Vector2, home_pos: Vector2, work_pos: Vector2, night_worker: bool, town_idx: int) -> int:
	return spawn_npc(Job.GUARD, Faction.VILLAGER, pos, home_pos, work_pos, night_worker, false, Config.GUARD_HP, Config.GUARD_DAMAGE, Config.GUARD_RANGE, town_idx)


func spawn_raider(pos: Vector2, camp_pos: Vector2, town_idx: int) -> int:
	return spawn_npc(Job.RAIDER, Faction.RAIDER, pos, camp_pos, camp_pos, false, false, Config.RAIDER_HP, Config.RAIDER_DAMAGE, Config.RAIDER_RANGE, town_idx)


# ============================================================
# TRAIT ROLLING
# ============================================================

func _roll_trait() -> int:
	# 60% no trait, 40% get a trait
	if randf() < 0.6:
		return NPCState.Trait.NONE
	# Equal chance among traits (excluding NONE)
	var trait_pool := [
		NPCState.Trait.BRAVE,
		NPCState.Trait.COWARD,
		NPCState.Trait.EFFICIENT,
		NPCState.Trait.HARDY,
		NPCState.Trait.LAZY,
		NPCState.Trait.STRONG,
		NPCState.Trait.SWIFT,
		NPCState.Trait.SHARPSHOT,
		NPCState.Trait.BERSERKER,
	]
	return trait_pool[randi() % trait_pool.size()]


# ============================================================
# SLOT MANAGEMENT
# ============================================================

func free_slot(i: int) -> void:
	# Add dead NPC's slot to free list for reuse
	_free_slots.append(i)


# ============================================================
# POPULATION COUNTING
# ============================================================

func count_alive_by_job_and_town(job: int, town_idx: int) -> int:
	var found := 0
	for i in count:
		if healths[i] <= 0:
			continue
		if jobs[i] != job:
			continue
		if town_indices[i] != town_idx:
			continue
		found += 1
	return found


# ============================================================
# RAIDER HELPERS
# ============================================================

func count_nearby_raiders(i: int, radius: float = 200.0) -> int:
	var my_pos: Vector2 = positions[i]
	var nearby: Array = _grid.get_nearby(my_pos)
	var radius_sq: float = radius * radius
	var count_found := 0

	for other_idx in nearby:
		if other_idx == i:
			continue
		if healths[other_idx] <= 0:
			continue
		if jobs[other_idx] != Job.RAIDER:
			continue

		var dist_sq: float = my_pos.distance_squared_to(positions[other_idx])
		if dist_sq < radius_sq:
			count_found += 1

	return count_found


func find_nearest_raider(i: int) -> int:
	var my_pos: Vector2 = positions[i]
	var nearest := -1
	var nearest_dist_sq := INF

	# Expanding radius search using grid
	var search_radius := NPCGrid.CELL_SIZE * 2
	while search_radius <= Config.world_width:
		var nearby: Array = _grid.get_nearby_in_radius(my_pos, search_radius)

		for other_idx in nearby:
			if other_idx == i:
				continue
			if healths[other_idx] <= 0:
				continue
			if jobs[other_idx] != Job.RAIDER:
				continue

			var dist_sq: float = my_pos.distance_squared_to(positions[other_idx])
			if dist_sq < nearest_dist_sq:
				nearest_dist_sq = dist_sq
				nearest = other_idx

		if nearest >= 0:
			break
		search_radius *= 2

	return nearest


func get_raider_group_center(i: int, radius: float = 200.0) -> Vector2:
	var my_pos: Vector2 = positions[i]
	var nearby: Array = _grid.get_nearby(my_pos)
	var radius_sq: float = radius * radius
	var center := my_pos
	var count_found := 1

	for other_idx in nearby:
		if other_idx == i:
			continue
		if healths[other_idx] <= 0:
			continue
		if jobs[other_idx] != Job.RAIDER:
			continue

		var dist_sq: float = my_pos.distance_squared_to(positions[other_idx])
		if dist_sq < radius_sq:
			center += positions[other_idx]
			count_found += 1

	return center / count_found


func find_nearest_farm(pos: Vector2) -> Vector2:
	var nearest := Vector2.ZERO
	var nearest_dist_sq := INF

	for farm_pos in farm_positions:
		var dist_sq: float = pos.distance_squared_to(farm_pos)
		if dist_sq < nearest_dist_sq:
			nearest_dist_sq = dist_sq
			nearest = farm_pos

	return nearest


# ============================================================
# BED MANAGEMENT
# ============================================================

func find_closest_free_bed(town_idx: int, pos: Vector2) -> int:
	if town_idx < 0 or town_idx >= beds_by_town.size():
		return -1

	var beds: Array = beds_by_town[town_idx]
	var occupants: PackedInt32Array = bed_occupants[town_idx]

	var best_idx := -1
	var best_dist_sq := INF

	for bed_idx in beds.size():
		if occupants[bed_idx] >= 0:
			continue  # Bed occupied
		var bed_pos: Vector2 = beds[bed_idx]
		var dist_sq: float = pos.distance_squared_to(bed_pos)
		if dist_sq < best_dist_sq:
			best_dist_sq = dist_sq
			best_idx = bed_idx

	return best_idx


func reserve_bed(town_idx: int, bed_idx: int, npc_idx: int) -> void:
	if town_idx < 0 or town_idx >= bed_occupants.size():
		return
	if bed_idx < 0 or bed_idx >= bed_occupants[town_idx].size():
		return

	bed_occupants[town_idx][bed_idx] = npc_idx
	current_bed_idx[npc_idx] = bed_idx


func release_bed(npc_idx: int) -> void:
	var bed_idx: int = current_bed_idx[npc_idx]
	if bed_idx < 0:
		return

	var town_idx: int = town_indices[npc_idx]
	if town_idx >= 0 and town_idx < bed_occupants.size():
		if bed_idx < bed_occupants[town_idx].size():
			bed_occupants[town_idx][bed_idx] = -1

	current_bed_idx[npc_idx] = -1


func get_bed_position(town_idx: int, bed_idx: int) -> Vector2:
	if town_idx < 0 or town_idx >= beds_by_town.size():
		return Vector2.ZERO
	if bed_idx < 0 or bed_idx >= beds_by_town[town_idx].size():
		return Vector2.ZERO
	return beds_by_town[town_idx][bed_idx]


func get_free_bed_count(town_idx: int) -> int:
	if town_idx < 0 or town_idx >= bed_occupants.size():
		return 0
	var free := 0
	for occupant in bed_occupants[town_idx]:
		if occupant < 0:
			free += 1
	return free


func get_total_bed_count(town_idx: int) -> int:
	if town_idx < 0 or town_idx >= beds_by_town.size():
		return 0
	return beds_by_town[town_idx].size()


# ============================================================
# FARM MANAGEMENT
# ============================================================

const MAX_FARMERS_PER_FARM := 4

func find_closest_free_farm(town_idx: int, pos: Vector2) -> int:
	if town_idx < 0 or town_idx >= farms_by_town.size():
		return -1

	var farms: Array = farms_by_town[town_idx]
	var counts: PackedInt32Array = farm_occupant_counts[town_idx]

	var best_idx := -1
	var best_dist_sq := INF

	for farm_idx in farms.size():
		if counts[farm_idx] >= MAX_FARMERS_PER_FARM:
			continue  # Farm full
		var farm_pos: Vector2 = farms[farm_idx]
		var dist_sq: float = pos.distance_squared_to(farm_pos)
		if dist_sq < best_dist_sq:
			best_dist_sq = dist_sq
			best_idx = farm_idx

	return best_idx


func reserve_farm(town_idx: int, farm_idx: int, npc_idx: int) -> void:
	if town_idx < 0 or town_idx >= farm_occupant_counts.size():
		return
	if farm_idx < 0 or farm_idx >= farm_occupant_counts[town_idx].size():
		return

	farm_occupant_counts[town_idx][farm_idx] += 1
	current_farm_idx[npc_idx] = farm_idx


func release_farm(npc_idx: int) -> void:
	var farm_idx: int = current_farm_idx[npc_idx]
	if farm_idx < 0:
		return

	var town_idx: int = town_indices[npc_idx]
	if town_idx >= 0 and town_idx < farm_occupant_counts.size():
		if farm_idx < farm_occupant_counts[town_idx].size():
			farm_occupant_counts[town_idx][farm_idx] = maxi(0, farm_occupant_counts[town_idx][farm_idx] - 1)

	current_farm_idx[npc_idx] = -1


func get_farm_position(town_idx: int, farm_idx: int) -> Vector2:
	if town_idx < 0 or town_idx >= farms_by_town.size():
		return Vector2.ZERO
	if farm_idx < 0 or farm_idx >= farms_by_town[town_idx].size():
		return Vector2.ZERO
	return farms_by_town[town_idx][farm_idx]


# ============================================================
# CALLBACKS
# ============================================================

func _on_time_tick(_hour: int, _minute: int) -> void:
	_needs.on_time_tick(_hour, _minute)


func _on_npc_arrived(i: int) -> void:
	_needs.on_arrival(i)


func _decide_what_to_do(i: int) -> void:
	_needs.decide_what_to_do(i)


func mark_health_dirty(i: int) -> void:
	health_dirty[i] = 1


func record_kill(victim_faction: int) -> void:
	if victim_faction == Faction.VILLAGER:
		villager_kills += 1
	else:
		raider_kills += 1


func grant_xp(i: int, amount: int) -> void:
	if levels[i] >= MAX_LEVEL:
		return

	xp[i] += amount
	var start_level: int = levels[i]

	# Check for level ups
	while xp[i] >= get_xp_for_next_level(levels[i]) and levels[i] < MAX_LEVEL:
		xp[i] -= get_xp_for_next_level(levels[i])
		var old_scale: float = get_stat_scale(levels[i])
		levels[i] += 1
		var new_scale: float = get_stat_scale(levels[i])
		# Heal proportionally to new max HP
		healths[i] = healths[i] * new_scale / old_scale
		health_dirty[i] = 1

	# Emit once with start and end level
	if levels[i] > start_level:
		_nav.update_cached_size(i)
		npc_leveled_up.emit(i, jobs[i], start_level, levels[i])


func get_scaled_damage(i: int) -> float:
	return attack_damages[i] * get_stat_scale(levels[i])


func get_scaled_max_health(i: int) -> float:
	return max_healths[i] * get_stat_scale(levels[i])


func apply_town_upgrade(town_idx: int, upgrade_type: String, new_level: int) -> void:
	var bonus: float = 0.0
	match upgrade_type:
		"guard_health":
			bonus = Config.UPGRADE_GUARD_HEALTH_BONUS * new_level
		"guard_attack":
			bonus = Config.UPGRADE_GUARD_ATTACK_BONUS * new_level
		"guard_range":
			bonus = Config.UPGRADE_GUARD_RANGE_BONUS * new_level
		"guard_size":
			bonus = Config.UPGRADE_GUARD_SIZE_BONUS * new_level
		"farmer_hp":
			bonus = Config.UPGRADE_FARMER_HP_BONUS * new_level

	# Apply to NPCs in this town
	for i in count:
		if healths[i] <= 0:
			continue
		if town_indices[i] != town_idx:
			continue

		match upgrade_type:
			"guard_health":
				if jobs[i] != Job.GUARD:
					continue
				var old_max: float = max_healths[i]
				max_healths[i] = Config.GUARD_HP * (1.0 + bonus)
				healths[i] = healths[i] * max_healths[i] / old_max
				health_dirty[i] = 1
			"guard_attack":
				if jobs[i] != Job.GUARD:
					continue
				attack_damages[i] = Config.GUARD_DAMAGE * (1.0 + bonus)
			"guard_range":
				if jobs[i] != Job.GUARD:
					continue
				attack_ranges[i] = Config.GUARD_RANGE * (1.0 + bonus)
			"guard_size":
				if jobs[i] != Job.GUARD:
					continue
				size_bonuses[i] = bonus
			"farmer_hp":
				if jobs[i] != Job.FARMER:
					continue
				var old_max: float = max_healths[i]
				max_healths[i] = Config.FARMER_HP * (1.0 + bonus)
				healths[i] = healths[i] * max_healths[i] / old_max
				health_dirty[i] = 1


# ============================================================
# GRID ACCESS (for other systems)
# ============================================================

func _grid_get_nearby(pos: Vector2) -> Array:
	return _grid.get_nearby(pos)


# ============================================================
# SELECTION / UI
# ============================================================

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		selected_npc = _get_npc_at_mouse()


func _get_npc_at_mouse() -> int:
	var mouse_pos: Vector2 = get_global_mouse_position()
	var nearby: Array = _grid.get_nearby(mouse_pos)

	for i in nearby:
		if healths[i] <= 0:
			continue
		var pos: Vector2 = positions[i]
		if pos.distance_to(mouse_pos) < Config.NPC_CLICK_RADIUS:
			return i

	return -1


func _update_selection() -> void:
	if selected_npc >= 0 and healths[selected_npc] > 0:
		info_label.visible = true
		var pos: Vector2 = positions[selected_npc]
		info_label.global_position = pos + Vector2(-40, -40)
		var job: int = jobs[selected_npc]
		var state: int = states[selected_npc]
		var lvl: int = levels[selected_npc]
		var npc_name: String = npc_names[selected_npc]
		var job_name: String = _state.get_job_name(job)
		var npc_trait: int = traits[selected_npc]
		var trait_name: String = NPCState.TRAIT_NAMES.get(npc_trait, "")
		var status: String = _state.get_state_name(state)
		if job == Job.RAIDER and carrying_food[selected_npc] == 1:
			status = "Looting"
		var display := "%s - %s Lv.%d" % [npc_name, job_name, lvl]
		if not trait_name.is_empty():
			display += " (%s)" % trait_name
		info_label.text = "%s | %s" % [display, status]
	else:
		info_label.visible = false
