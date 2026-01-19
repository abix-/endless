# npc_manager.gd
# Orchestrates NPC systems, owns data arrays
extends Node2D

# Re-export enums for external access (main.gd uses these)
enum State { IDLE, WALKING, SLEEPING, WORKING, RESTING, WANDERING, FIGHTING, FLEEING }
enum Faction { VILLAGER, RAIDER }
enum Job { FARMER, GUARD, RAIDER }

signal npc_leveled_up(npc_index: int, job: int, new_level: int)

const MAX_LEVEL := 9999

# Scaling functions
static func get_stat_scale(level: int) -> float:
	return sqrt(float(level))  # Level 1 = 1x, Level 9999 = 100x

static func get_size_scale(level: int) -> float:
	# Level 1 = 1x, Level 9999 = 50x
	return 1.0 + (sqrt(float(level)) - 1.0) * 0.495

static func get_xp_for_next_level(level: int) -> int:
	return level  # Need 'level' XP to go from level to level+1

# World info (set by main.gd)
var village_center := Vector2.ZERO
var farm_positions: Array[Vector2] = []

# Data arrays
var count := 0
var max_count := Config.MAX_NPC_COUNT

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
var death_times: PackedInt32Array

var states: PackedInt32Array
var factions: PackedInt32Array
var jobs: PackedInt32Array
var current_targets: PackedInt32Array
var will_flee: PackedInt32Array
var works_at_night: PackedInt32Array
var health_dirty: PackedInt32Array
var last_rendered: PackedInt32Array
var flash_timers: PackedFloat32Array

var levels: PackedInt32Array
var xp: PackedInt32Array
var carrying_food: PackedInt32Array  # Raiders carrying stolen food

var home_positions: PackedVector2Array
var work_positions: PackedVector2Array
var spawn_positions: PackedVector2Array

# Selection
var selected_npc := -1

# Rendering
@onready var multimesh_instance: MultiMeshInstance2D = $MultiMeshInstance2D
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


func set_projectile_manager(pm: Node) -> void:
	_projectiles = pm


func _ready() -> void:
	add_to_group("npc_manager")
	_init_arrays()
	_init_systems()
	WorldClock.time_tick.connect(_on_time_tick)


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
	death_times.resize(max_count)

	states.resize(max_count)
	factions.resize(max_count)
	jobs.resize(max_count)
	current_targets.resize(max_count)
	will_flee.resize(max_count)
	works_at_night.resize(max_count)
	health_dirty.resize(max_count)
	last_rendered.resize(max_count)
	flash_timers.resize(max_count)
	levels.resize(max_count)
	xp.resize(max_count)
	carrying_food.resize(max_count)

	for i in max_count:
		death_times[i] = -1


func _init_systems() -> void:
	_grid = NPCGrid.new(self)
	_renderer = NPCRenderer.new(self, multimesh_instance)
	_state = NPCState.new(self)
	_nav = NPCNavigation.new(self)
	_combat = NPCCombat.new(self)
	_needs = NPCNeeds.new(self)

	_nav.arrived.connect(_on_npc_arrived)


# ============================================================
# MAIN LOOP
# ============================================================

func _process(delta: float) -> void:
	var t1 := Time.get_ticks_usec()

	_grid.rebuild()

	_combat.process_scanning(delta)
	_combat.process(delta)
	_nav.process(delta)

	if _projectiles:
		_projectiles.process(delta)

	_renderer.update(delta)

	var t2 := Time.get_ticks_usec()
	last_loop_time = (t2 - t1) / 1000.0

	_update_selection()

	if Engine.get_process_frames() % 30 == 0:
		_update_counts()


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

func spawn_npc(job: int, faction: int, pos: Vector2, home_pos: Vector2, work_pos: Vector2, night_worker: bool, flee: bool, hp: float, damage: float, attack_range: float) -> int:
	if count >= max_count:
		return -1

	var i: int = count
	count += 1

	positions[i] = pos
	velocities[i] = Vector2.ZERO
	targets[i] = pos
	wander_centers[i] = pos
	home_positions[i] = home_pos
	work_positions[i] = work_pos
	spawn_positions[i] = pos

	healths[i] = hp
	max_healths[i] = hp
	energies[i] = Config.ENERGY_MAX
	attack_damages[i] = damage
	attack_ranges[i] = attack_range
	attack_timers[i] = 0.0
	scan_timers[i] = randf() * Config.SCAN_INTERVAL
	death_times[i] = -1

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

	match job:
		Job.FARMER: total_farmers += 1
		Job.GUARD: total_guards += 1
		Job.RAIDER: total_raiders += 1

	_renderer.set_npc_sprite(i, job)
	_renderer.set_visible_count(count)

	_decide_what_to_do(i)

	return i


func spawn_farmer(pos: Vector2, home_pos: Vector2, work_pos: Vector2) -> int:
	return spawn_npc(Job.FARMER, Faction.VILLAGER, pos, home_pos, work_pos, false, true, Config.FARMER_HP, Config.FARMER_DAMAGE, Config.FARMER_RANGE)


func spawn_guard(pos: Vector2, home_pos: Vector2, work_pos: Vector2, night_worker: bool) -> int:
	return spawn_npc(Job.GUARD, Faction.VILLAGER, pos, home_pos, work_pos, night_worker, false, Config.GUARD_HP, Config.GUARD_DAMAGE, Config.GUARD_RANGE)


func spawn_raider(pos: Vector2, camp_pos: Vector2) -> int:
	return spawn_npc(Job.RAIDER, Faction.RAIDER, pos, camp_pos, camp_pos, false, false, Config.RAIDER_HP, Config.RAIDER_DAMAGE, Config.RAIDER_RANGE)


# ============================================================
# RESPAWNING
# ============================================================

func _check_respawns() -> void:
	var current_time: int = WorldClock.get_total_minutes()

	for i in count:
		if healths[i] > 0:
			continue
		if death_times[i] < 0:
			continue

		var time_dead: int = current_time - death_times[i]
		if time_dead >= Config.RESPAWN_MINUTES:
			_respawn(i)


func _respawn(i: int) -> void:
	# All NPCs respawn at home (camp for raiders)
	positions[i] = home_positions[i]
	wander_centers[i] = home_positions[i]

	healths[i] = max_healths[i]
	energies[i] = Config.ENERGY_MAX
	attack_timers[i] = 0.0
	scan_timers[i] = randf() * Config.SCAN_INTERVAL
	death_times[i] = -1

	states[i] = State.IDLE
	current_targets[i] = -1
	health_dirty[i] = 1
	last_rendered[i] = 0
	carrying_food[i] = 0

	_renderer.show_npc(i, positions[i])
	_renderer.set_npc_health_display(i, 1.0)

	_decide_what_to_do(i)


func record_death(i: int) -> void:
	death_times[i] = WorldClock.get_total_minutes()


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
	var search_radius := Config.GRID_CELL_SIZE * 2
	while search_radius <= Config.GRID_CELL_SIZE * Config.GRID_SIZE:
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
# CALLBACKS
# ============================================================

func _on_time_tick(_hour: int, _minute: int) -> void:
	_needs.on_time_tick(_hour, _minute)
	_check_respawns()


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

	# Check for level ups
	while xp[i] >= get_xp_for_next_level(levels[i]) and levels[i] < MAX_LEVEL:
		xp[i] -= get_xp_for_next_level(levels[i])
		var old_scale: float = get_stat_scale(levels[i])
		levels[i] += 1
		var new_scale: float = get_stat_scale(levels[i])
		# Heal proportionally to new max HP
		healths[i] = healths[i] * new_scale / old_scale
		health_dirty[i] = 1  # Trigger size/health bar update
		npc_leveled_up.emit(i, jobs[i], levels[i])


func get_scaled_damage(i: int) -> float:
	return attack_damages[i] * get_stat_scale(levels[i])


func get_scaled_max_health(i: int) -> float:
	return max_healths[i] * get_stat_scale(levels[i])


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
		var status: String = _state.get_state_name(state)
		if job == Job.RAIDER and carrying_food[selected_npc] == 1:
			status = "Loot"
		info_label.text = "%s Lv.%d | H:%.0f E:%.0f | %s" % [
			_state.get_job_name(job),
			lvl,
			healths[selected_npc],
			energies[selected_npc],
			status
		]
	else:
		info_label.visible = false
