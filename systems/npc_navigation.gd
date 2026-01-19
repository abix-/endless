# npc_navigation.gd
# Handles movement with predicted rendering (Factorio-style optimization)
# Logic runs every N frames, rendering interpolates between
extends RefCounted
class_name NPCNavigation

const STATIONARY_STATES := [NPCState.State.IDLE, NPCState.State.RESTING, NPCState.State.FARMING, NPCState.State.OFF_DUTY, NPCState.State.ON_DUTY]
const MOVING_STATES := [NPCState.State.WALKING, NPCState.State.PATROLLING, NPCState.State.RETURNING, NPCState.State.RAIDING, NPCState.State.FIGHTING, NPCState.State.FLEEING]

# Logic update intervals by state (in frames)
const LOGIC_INTERVAL_COMBAT := 2    # Combat needs fast updates
const LOGIC_INTERVAL_MOVING := 5    # Walking can be slower
const LOGIC_INTERVAL_IDLE := 30     # Stationary states barely need updates

var manager: Node
var separation_velocities: PackedVector2Array  # Smooth separation

signal arrived(npc_index: int)

func _init(npc_manager: Node) -> void:
	manager = npc_manager
	separation_velocities.resize(manager.max_count)


func process(delta: float) -> void:
	var frame: int = Engine.get_process_frames()

	# Get camera position for LOD
	var cam_pos := Vector2.ZERO
	var camera: Camera2D = manager.get_viewport().get_camera_2d()
	if camera:
		cam_pos = camera.global_position

	for i in manager.count:
		if manager.healths[i] <= 0:
			continue

		var state: int = manager.states[i]

		# Stationary NPCs: skip entirely (huge savings)
		if state in STATIONARY_STATES:
			manager.intended_velocities[i] = Vector2.ZERO
			continue

		# LOD: skip distant NPCs on some frames
		var dist_sq: float = manager.positions[i].distance_squared_to(cam_pos)
		var lod_mult: int = _get_lod_multiplier(dist_sq)

		# Determine if we should run logic this frame
		var logic_interval: int = _get_logic_interval(state) * lod_mult
		var frames_since_logic: int = frame - manager.last_logic_frame[i]

		# Separation runs on its own stagger (every 4 frames per NPC)
		# This is independent of logic updates to ensure smooth collision avoidance
		if i % 4 == frame % 4:
			_calc_separation(i)

		if frames_since_logic >= logic_interval:
			# LOGIC UPDATE: expensive calculations
			_update_logic(i, delta * float(frames_since_logic), state)
			manager.last_logic_frame[i] = frame

		# RENDER UPDATE: always apply stored velocity (cheap)
		_update_render(i, delta)


func _get_logic_interval(state: int) -> int:
	match state:
		NPCState.State.FIGHTING, NPCState.State.FLEEING:
			return LOGIC_INTERVAL_COMBAT
		NPCState.State.WALKING, NPCState.State.PATROLLING, NPCState.State.RETURNING, NPCState.State.RAIDING:
			return LOGIC_INTERVAL_MOVING
		_:
			return LOGIC_INTERVAL_IDLE


func _get_lod_multiplier(dist_sq: float) -> int:
	if dist_sq < Config.LOD_NEAR_SQ:
		return 1
	elif dist_sq < Config.LOD_MID_SQ:
		return 2
	elif dist_sq < Config.LOD_FAR_SQ:
		return 4
	else:
		return 8


func _update_logic(i: int, accumulated_delta: float, state: int) -> void:
	# Calculate intended velocity based on state
	match state:
		NPCState.State.WALKING, NPCState.State.PATROLLING, NPCState.State.RETURNING, NPCState.State.RAIDING:
			_calc_move_toward_target(i)
		NPCState.State.FIGHTING:
			_calc_move_toward_enemy(i)
		NPCState.State.FLEEING:
			_calc_move_toward_flee_target(i)


func _update_render(i: int, delta: float) -> void:
	# Apply stored velocity (cheap - just position += velocity * delta)
	var velocity: Vector2 = manager.intended_velocities[i]
	var sep_vel: Vector2 = separation_velocities[i]

	if velocity.length_squared() < 0.01 and sep_vel.length_squared() < 0.01:
		return

	var my_pos: Vector2 = manager.positions[i]
	var new_pos: Vector2 = my_pos + (velocity + sep_vel) * delta

	# Check for arrival (only if moving toward target)
	var target: Vector2 = manager.targets[i]
	var arrival_radius: float = manager.arrival_radii[i]
	var state: int = manager.states[i]

	if state in [NPCState.State.WALKING, NPCState.State.PATROLLING, NPCState.State.RETURNING, NPCState.State.RAIDING]:
		var dist_to_target: float = my_pos.distance_to(target)
		var move_dist: float = velocity.length() * delta

		if dist_to_target < arrival_radius or move_dist >= dist_to_target:
			# Arrived - snap to target, clear velocity, emit signal
			manager.positions[i] = target
			manager.intended_velocities[i] = Vector2.ZERO
			manager.last_logic_frame[i] = 0  # Force logic update next frame
			arrived.emit(i)
			return

	manager.positions[i] = new_pos


func _calc_move_toward_target(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var target_pos: Vector2 = manager.targets[i]
	var dist: float = my_pos.distance_to(target_pos)

	if dist < manager.arrival_radii[i]:
		manager.intended_velocities[i] = Vector2.ZERO
	else:
		var dir: Vector2 = my_pos.direction_to(target_pos)
		manager.intended_velocities[i] = dir * Config.MOVE_SPEED


func _calc_move_toward_enemy(i: int) -> void:
	var target_idx: int = manager.current_targets[i]

	if target_idx < 0 or manager.healths[target_idx] <= 0:
		manager.intended_velocities[i] = Vector2.ZERO
		return

	var my_pos: Vector2 = manager.positions[i]
	var enemy_pos: Vector2 = manager.positions[target_idx]
	var dist: float = my_pos.distance_to(enemy_pos)
	var attack_range: float = manager.attack_ranges[i]

	if dist > attack_range:
		var dir: Vector2 = my_pos.direction_to(enemy_pos)
		manager.intended_velocities[i] = dir * Config.MOVE_SPEED
	else:
		manager.intended_velocities[i] = Vector2.ZERO


func _calc_move_toward_flee_target(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var job: int = manager.jobs[i]

	# Determine flee destination
	var flee_target: Vector2
	if job == NPCState.Job.RAIDER:
		flee_target = manager.home_positions[i]
	else:
		var town_idx: int = manager.town_indices[i]
		if town_idx >= 0 and town_idx < manager.town_centers.size():
			flee_target = manager.town_centers[town_idx]
		else:
			flee_target = manager.home_positions[i]

	var dir: Vector2 = my_pos.direction_to(flee_target)
	manager.intended_velocities[i] = dir * Config.MOVE_SPEED * 1.2  # Flee faster


func _calc_separation(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var my_state: int = manager.states[i]
	var my_size: float = manager.get_size_scale(manager.levels[i])
	var nearby: Array = manager._grid.get_nearby(my_pos)
	var separation := Vector2.ZERO
	var dodge := Vector2.ZERO

	# Scale separation radius by size
	var my_radius: float = Config.SEPARATION_RADIUS * my_size

	var i_am_moving: bool = my_state not in STATIONARY_STATES
	var my_dir := Vector2.ZERO
	if i_am_moving:
		my_dir = my_pos.direction_to(manager.targets[i])

	for other_idx in nearby:
		if other_idx == i:
			continue
		if manager.healths[other_idx] <= 0:
			continue

		var other_pos: Vector2 = manager.positions[other_idx]
		var other_size: float = manager.get_size_scale(manager.levels[other_idx])
		var combined_radius: float = (my_radius + Config.SEPARATION_RADIUS * other_size) * 0.5
		var combined_radius_sq: float = combined_radius * combined_radius

		var diff: Vector2 = my_pos - other_pos
		var dist_sq: float = diff.length_squared()

		if dist_sq > 0 and dist_sq < combined_radius_sq:
			var other_state: int = manager.states[other_idx]
			var other_stationary: bool = other_state in STATIONARY_STATES

			# Push strength based on relative size
			var push_strength: float = other_size / my_size
			if i_am_moving and other_stationary:
				push_strength *= 3.0

			separation += diff.normalized() / sqrt(dist_sq) * push_strength

		# TCP-like collision avoidance
		var approach_radius_sq: float = combined_radius_sq * 4.0
		if i_am_moving and dist_sq > 0 and dist_sq < approach_radius_sq:
			var other_state: int = manager.states[other_idx]
			var other_moving: bool = other_state not in STATIONARY_STATES

			if other_moving:
				var to_other: Vector2 = diff.normalized() * -1.0
				var other_dir: Vector2 = other_pos.direction_to(manager.targets[other_idx])

				var i_approach: float = my_dir.dot(to_other)
				var they_approach: float = other_dir.dot(-to_other)

				if i_approach > 0.3:
					var perp: Vector2 = Vector2(-my_dir.y, my_dir.x)
					var dodge_strength: float = 0.0

					if they_approach > 0.3:
						dodge_strength = 0.5  # Head-on
					elif they_approach < -0.3:
						dodge_strength = 0.3  # Overtaking
					else:
						dodge_strength = 0.4  # Crossing

					if i < other_idx:
						dodge += perp * dodge_strength
					else:
						dodge -= perp * dodge_strength

	var final_vel := Vector2.ZERO
	if separation.length_squared() > 0:
		final_vel = separation.normalized() * Config.SEPARATION_STRENGTH
	if dodge.length_squared() > 0:
		final_vel += dodge.normalized() * Config.SEPARATION_STRENGTH * 0.7

	separation_velocities[i] = final_vel


# Force immediate logic update (call when state changes, damage taken, etc.)
func force_logic_update(i: int) -> void:
	manager.last_logic_frame[i] = 0
