# npc_manager.gd
# Orchestrates NPC systems, owns data arrays
extends Node2D

# Re-export enums for external access (main.gd uses these)
enum State { IDLE, WALKING, SLEEPING, WORKING, RESTING, WANDERING, FIGHTING, FLEEING }
enum Faction { VILLAGER, RAIDER }
enum Job { FARMER, GUARD, RAIDER }

# Constants
const MOVE_SPEED := 50.0
const ATTACK_RANGE := 30.0
const ATTACK_COOLDOWN := 1.0
const SCAN_INTERVAL := 0.2
const RESPAWN_MINUTES := 720  # 12 hours = 720 minutes

# Data arrays
var count := 0
var max_count := 3000

var positions: PackedVector2Array
var velocities: PackedVector2Array
var targets: PackedVector2Array
var wander_centers: PackedVector2Array

var healths: PackedFloat32Array
var max_healths: PackedFloat32Array
var energies: PackedFloat32Array
var attack_damages: PackedFloat32Array
var attack_timers: PackedFloat32Array
var scan_timers: PackedFloat32Array
var death_times: PackedInt32Array  # When NPC died (in total minutes), -1 if alive

var states: PackedInt32Array
var factions: PackedInt32Array
var jobs: PackedInt32Array
var current_targets: PackedInt32Array
var will_flee: PackedInt32Array
var works_at_night: PackedInt32Array
var health_dirty: PackedInt32Array
var last_rendered: PackedInt32Array

var home_positions: PackedVector2Array
var work_positions: PackedVector2Array
var spawn_positions: PackedVector2Array  # Original spawn position for raiders

# Spatial grid - flat typed arrays
var grid_cells: PackedInt32Array
var grid_cell_counts: PackedInt32Array
var grid_cell_starts: PackedInt32Array
const GRID_SIZE := 64
const GRID_CELL_CAPACITY := 64
var cell_size := 100.0

# Selection
var selected_npc := -1

# Rendering
@onready var multimesh_instance: MultiMeshInstance2D = $MultiMeshInstance2D
@onready var info_label: Label = $InfoLabel
var multimesh: MultiMesh

# Stats - alive counts
var alive_farmers := 0
var alive_guards := 0
var alive_raiders := 0

# Stats - totals (for "X / Y" display)
var total_farmers := 0
var total_guards := 0
var total_raiders := 0

# Stats - kills
var villager_kills := 0  # Villagers killed by raiders
var raider_kills := 0    # Raiders killed by villagers

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


func _ready() -> void:
	add_to_group("npc_manager")
	_init_arrays()
	_init_grid()
	_init_multimesh()
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
	
	# Initialize death_times to -1 (alive)
	for i in max_count:
		death_times[i] = -1


func _init_grid() -> void:
	var total_cells: int = GRID_SIZE * GRID_SIZE
	grid_cells.resize(total_cells * GRID_CELL_CAPACITY)
	grid_cell_counts.resize(total_cells)
	grid_cell_starts.resize(total_cells)
	
	for i in total_cells:
		grid_cell_starts[i] = i * GRID_CELL_CAPACITY
		grid_cell_counts[i] = 0


func _init_multimesh() -> void:
	multimesh = MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_2D
	multimesh.use_colors = true
	multimesh.use_custom_data = true
	multimesh.instance_count = max_count
	multimesh.visible_instance_count = 0
	
	var quad := QuadMesh.new()
	quad.size = Vector2(16, 16)
	multimesh.mesh = quad
	
	multimesh_instance.multimesh = multimesh


func _init_systems() -> void:
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
	
	_grid_rebuild()
	
	_combat.process_scanning(delta)
	_combat.process(delta)
	_nav.process(delta)
	
	_update_rendering()
	
	var t2 := Time.get_ticks_usec()
	last_loop_time = (t2 - t1) / 1000.0
	
	_update_selection()
	
	if Engine.get_process_frames() % 30 == 0:
		_update_counts()


func _update_rendering() -> void:
	var camera: Camera2D = get_viewport().get_camera_2d()
	if not camera:
		for i in count:
			if healths[i] <= 0:
				continue
			multimesh.set_instance_transform_2d(i, Transform2D(0, positions[i]))
			if health_dirty[i] == 1:
				var health_pct: float = healths[i] / max_healths[i]
				multimesh.set_instance_custom_data(i, Color(health_pct, 0, 0, 0))
				health_dirty[i] = 0
		return
	
	var cam_pos: Vector2 = camera.global_position
	var view_size: Vector2 = get_viewport_rect().size / camera.zoom
	var margin := 100.0
	
	var min_x: float = cam_pos.x - view_size.x / 2 - margin
	var max_x: float = cam_pos.x + view_size.x / 2 + margin
	var min_y: float = cam_pos.y - view_size.y / 2 - margin
	var max_y: float = cam_pos.y + view_size.y / 2 + margin
	
	var visible_cells: PackedInt32Array = _get_cells_in_rect(min_x, max_x, min_y, max_y)
	
	# Hide NPCs that were rendered last frame but aren't visible now
	for i in count:
		if last_rendered[i] == 1:
			last_rendered[i] = 0
			multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))
	
	# Render only NPCs in visible cells
	for cell_idx in visible_cells:
		var start: int = grid_cell_starts[cell_idx]
		var cell_count: int = grid_cell_counts[cell_idx]
		
		for j in cell_count:
			var i: int = grid_cells[start + j]
			
			if healths[i] <= 0:
				continue
			
			var pos: Vector2 = positions[i]
			multimesh.set_instance_transform_2d(i, Transform2D(0, pos))
			last_rendered[i] = 1
			
			if health_dirty[i] == 1:
				var health_pct: float = healths[i] / max_healths[i]
				multimesh.set_instance_custom_data(i, Color(health_pct, 0, 0, 0))
				health_dirty[i] = 0


func _get_cells_in_rect(min_x: float, max_x: float, min_y: float, max_y: float) -> PackedInt32Array:
	var result: PackedInt32Array = []
	
	@warning_ignore("narrowing_conversion")
	var x1: int = clampi(int(min_x / cell_size), 0, GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var x2: int = clampi(int(max_x / cell_size), 0, GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var y1: int = clampi(int(min_y / cell_size), 0, GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var y2: int = clampi(int(max_y / cell_size), 0, GRID_SIZE - 1)
	
	for y in range(y1, y2 + 1):
		for x in range(x1, x2 + 1):
			result.append(y * GRID_SIZE + x)
	
	return result


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

func spawn_npc(job: int, faction: int, pos: Vector2, home_pos: Vector2, work_pos: Vector2, night_worker: bool, flee: bool, hp: float, damage: float) -> int:
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
	spawn_positions[i] = pos  # Remember original spawn
	
	healths[i] = hp
	max_healths[i] = hp
	energies[i] = 100.0
	attack_damages[i] = damage
	attack_timers[i] = 0.0
	scan_timers[i] = randf() * SCAN_INTERVAL
	death_times[i] = -1  # Alive
	
	states[i] = State.IDLE
	factions[i] = faction
	jobs[i] = job
	current_targets[i] = -1
	will_flee[i] = 1 if flee else 0
	works_at_night[i] = 1 if night_worker else 0
	health_dirty[i] = 1
	last_rendered[i] = 0
	
	# Track totals
	match job:
		Job.FARMER: total_farmers += 1
		Job.GUARD: total_guards += 1
		Job.RAIDER: total_raiders += 1
	
	var color: Color
	match job:
		Job.FARMER: color = Color.GREEN
		Job.GUARD: color = Color.BLUE
		Job.RAIDER: color = Color.RED
		_: color = Color.WHITE
	
	multimesh.set_instance_color(i, color)
	multimesh.set_instance_custom_data(i, Color(1, 0, 0, 0))
	multimesh.visible_instance_count = count
	
	_decide_what_to_do(i)
	
	return i


func spawn_farmer(pos: Vector2, home_pos: Vector2, work_pos: Vector2) -> int:
	return spawn_npc(Job.FARMER, Faction.VILLAGER, pos, home_pos, work_pos, false, true, 50.0, 5.0)


func spawn_guard(pos: Vector2, home_pos: Vector2, work_pos: Vector2, night_worker: bool) -> int:
	return spawn_npc(Job.GUARD, Faction.VILLAGER, pos, home_pos, work_pos, night_worker, false, 150.0, 15.0)


func spawn_raider(pos: Vector2) -> int:
	return spawn_npc(Job.RAIDER, Faction.RAIDER, pos, pos, pos, false, false, 100.0, 12.0)


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
		if time_dead >= RESPAWN_MINUTES:
			_respawn(i)


func _respawn(i: int) -> void:
	var job: int = jobs[i]
	
	# Reset position based on job type
	if job == Job.RAIDER:
		positions[i] = spawn_positions[i]
		wander_centers[i] = spawn_positions[i]
	else:
		positions[i] = home_positions[i]
		wander_centers[i] = home_positions[i]
	
	# Reset stats
	healths[i] = max_healths[i]
	energies[i] = 100.0
	attack_timers[i] = 0.0
	scan_timers[i] = randf() * SCAN_INTERVAL
	death_times[i] = -1  # Alive again
	
	states[i] = State.IDLE
	current_targets[i] = -1
	health_dirty[i] = 1
	last_rendered[i] = 0
	
	# Reset multimesh
	multimesh.set_instance_transform_2d(i, Transform2D(0, positions[i]))
	multimesh.set_instance_custom_data(i, Color(1, 0, 0, 0))
	
	_decide_what_to_do(i)


func record_death(i: int) -> void:
	death_times[i] = WorldClock.get_total_minutes()


# ============================================================
# CALLBACKS
# ============================================================

func _on_time_tick(hour: int, minute: int) -> void:
	_needs.on_time_tick(hour, minute)
	
	# Check respawns every game minute
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


# ============================================================
# SPATIAL GRID
# ============================================================

func _grid_cell_index(pos: Vector2) -> int:
	@warning_ignore("narrowing_conversion")
	var x: int = clampi(int(pos.x / cell_size), 0, GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var y: int = clampi(int(pos.y / cell_size), 0, GRID_SIZE - 1)
	return y * GRID_SIZE + x


func _grid_rebuild() -> void:
	for i in grid_cell_counts.size():
		grid_cell_counts[i] = 0
	
	for i in count:
		if healths[i] <= 0:
			continue
		
		var cell_idx: int = _grid_cell_index(positions[i])
		var cell_count: int = grid_cell_counts[cell_idx]
		
		if cell_count < GRID_CELL_CAPACITY:
			var slot: int = grid_cell_starts[cell_idx] + cell_count
			grid_cells[slot] = i
			grid_cell_counts[cell_idx] = cell_count + 1


func _grid_get_nearby(pos: Vector2) -> Array:
	var results := []
	
	@warning_ignore("narrowing_conversion")
	var cx: int = clampi(int(pos.x / cell_size), 0, GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var cy: int = clampi(int(pos.y / cell_size), 0, GRID_SIZE - 1)
	
	for dy in range(-1, 2):
		var ny: int = cy + dy
		if ny < 0 or ny >= GRID_SIZE:
			continue
		for dx in range(-1, 2):
			var nx: int = cx + dx
			if nx < 0 or nx >= GRID_SIZE:
				continue
			
			var cell_idx: int = ny * GRID_SIZE + nx
			var start: int = grid_cell_starts[cell_idx]
			var cell_count: int = grid_cell_counts[cell_idx]
			
			for j in cell_count:
				results.append(grid_cells[start + j])
	
	return results


# ============================================================
# SELECTION / UI
# ============================================================

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		selected_npc = _get_npc_at_mouse()


func _get_npc_at_mouse() -> int:
	var mouse_pos: Vector2 = get_global_mouse_position()
	var nearby: Array = _grid_get_nearby(mouse_pos)
	
	for i in nearby:
		if healths[i] <= 0:
			continue
		var pos: Vector2 = positions[i]
		if pos.distance_to(mouse_pos) < 16:
			return i
	
	return -1


func _update_selection() -> void:
	if selected_npc >= 0 and healths[selected_npc] > 0:
		info_label.visible = true
		var pos: Vector2 = positions[selected_npc]
		info_label.global_position = pos + Vector2(-40, -40)
		var job: int = jobs[selected_npc]
		var state: int = states[selected_npc]
		info_label.text = "%s | H:%.0f E:%.0f | %s" % [
			_state.get_job_name(job),
			healths[selected_npc],
			energies[selected_npc],
			_state.get_state_name(state)
		]
	else:
		info_label.visible = false
