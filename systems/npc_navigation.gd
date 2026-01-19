# npc_navigation.gd
# Handles movement toward targets, fleeing, and arrival
extends RefCounted
class_name NPCNavigation

const STATIONARY_STATES := [NPCState.State.WORKING, NPCState.State.SLEEPING, NPCState.State.RESTING, NPCState.State.IDLE]

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

		# LOD: skip distant NPCs on some frames
		var dist_sq: float = manager.positions[i].distance_squared_to(cam_pos)
		if not _should_update(i, frame, dist_sq):
			continue

		var state: int = manager.states[i]
		var lod_delta: float = _get_lod_delta(delta, dist_sq)

		match state:
			NPCState.State.WALKING, NPCState.State.WANDERING:
				_move_toward_target(i, lod_delta)
			NPCState.State.FIGHTING:
				_move_toward_enemy(i, lod_delta)
			NPCState.State.FLEEING:
				_move_away_from_enemy(i, lod_delta)

		# Apply separation smoothly every frame
		manager.positions[i] += separation_velocities[i] * lod_delta

		# Recalculate separation velocity (staggered)
		if i % 4 == frame % 4:
			_calc_separation(i)


func _should_update(i: int, frame: int, dist_sq: float) -> bool:
	if dist_sq < Config.LOD_NEAR_SQ:
		return true  # Every frame
	elif dist_sq < Config.LOD_MID_SQ:
		return i % 2 == frame % 2  # Every 2 frames
	elif dist_sq < Config.LOD_FAR_SQ:
		return i % 4 == frame % 4  # Every 4 frames
	else:
		return i % 8 == frame % 8  # Every 8 frames


func _get_lod_delta(delta: float, dist_sq: float) -> float:
	# Compensate for skipped frames so movement speed stays correct
	if dist_sq < Config.LOD_NEAR_SQ:
		return delta
	elif dist_sq < Config.LOD_MID_SQ:
		return delta * 2.0
	elif dist_sq < Config.LOD_FAR_SQ:
		return delta * 4.0
	else:
		return delta * 8.0


func _move_toward_target(i: int, delta: float) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var target_pos: Vector2 = manager.targets[i]
	var dist: float = my_pos.distance_to(target_pos)
	var arrival_radius: float = manager.arrival_radii[i]

	if dist < arrival_radius:
		arrived.emit(i)
	else:
		var move_dist: float = minf(Config.MOVE_SPEED * delta, dist)
		var dir: Vector2 = my_pos.direction_to(target_pos)
		manager.positions[i] = my_pos + dir * move_dist


func _move_toward_enemy(i: int, delta: float) -> void:
	var target_idx: int = manager.current_targets[i]

	if target_idx < 0 or manager.healths[target_idx] <= 0:
		return

	var my_pos: Vector2 = manager.positions[i]
	var enemy_pos: Vector2 = manager.positions[target_idx]
	var dist: float = my_pos.distance_to(enemy_pos)
	var attack_range: float = manager.attack_ranges[i]

	if dist > attack_range:
		var move_dist: float = minf(Config.MOVE_SPEED * delta, dist - attack_range)
		var dir: Vector2 = my_pos.direction_to(enemy_pos)
		manager.positions[i] = my_pos + dir * move_dist


func _move_away_from_enemy(i: int, delta: float) -> void:
	var target_idx: int = manager.current_targets[i]

	if target_idx < 0 or manager.healths[target_idx] <= 0:
		return

	var my_pos: Vector2 = manager.positions[i]
	var enemy_pos: Vector2 = manager.positions[target_idx]
	var dir: Vector2 = enemy_pos.direction_to(my_pos)
	manager.positions[i] = my_pos + dir * Config.MOVE_SPEED * 1.2 * delta


func _calc_separation(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var my_state: int = manager.states[i]
	var my_size: float = manager.get_size_scale(manager.levels[i])
	var nearby: Array = manager._grid.get_nearby(my_pos)
	var separation := Vector2.ZERO

	# Scale separation radius by size
	var my_radius: float = Config.SEPARATION_RADIUS * my_size

	var i_am_moving: bool = my_state not in STATIONARY_STATES

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

			# Push strength based on relative size (bigger pushes smaller)
			var push_strength: float = other_size / my_size
			if i_am_moving and other_stationary:
				push_strength *= 3.0  # Push harder past workers

			separation += diff.normalized() / sqrt(dist_sq) * push_strength

	# Store as velocity for smooth per-frame application
	if separation.length_squared() > 0:
		separation_velocities[i] = separation.normalized() * Config.SEPARATION_STRENGTH
	else:
		separation_velocities[i] = Vector2.ZERO
