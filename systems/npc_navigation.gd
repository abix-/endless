# npc_navigation.gd
# Handles movement with predicted rendering (Factorio-style optimization)
# Logic runs every N frames, rendering interpolates between
extends RefCounted
class_name NPCNavigation

# Bitmask for fast state checks (bit N = state N is stationary)
# IDLE=0, RESTING=1, FARMING=5, OFF_DUTY=6, ON_DUTY=7
const STATIONARY_MASK := (1 << 0) | (1 << 1) | (1 << 5) | (1 << 6) | (1 << 7)

# Logic update intervals by state (in frames)
const LOGIC_INTERVAL_COMBAT := 2    # Combat needs fast updates
const LOGIC_INTERVAL_MOVING := 5    # Walking can be slower
const LOGIC_INTERVAL_IDLE := 30     # Stationary states barely need updates
const DRIFT_THRESHOLD_SQ := 900.0   # 30px - re-evaluate if pushed this far from anchor

var manager: Node
var separation_velocities: PackedVector2Array  # Smooth separation
var cached_sizes: PackedFloat32Array  # Cached size scales to avoid sqrt() in hot loop

# Sub-profiling (in ms)
var profile_loop := 0.0      # Main loop overhead (iteration, state checks)
var profile_sep := 0.0       # Separation calculations
var profile_logic := 0.0     # Logic updates
var profile_render := 0.0    # Render updates

# Separation sub-profiling
var profile_sep_grid := 0.0    # get_nearby call
var profile_sep_loop := 0.0    # inner loop
var sep_neighbor_count := 0    # total neighbors checked
var sep_call_count := 0        # how many NPCs ran separation this frame

signal arrived(npc_index: int)

func _init(npc_manager: Node) -> void:
	manager = npc_manager
	separation_velocities.resize(manager.max_count)
	cached_sizes.resize(manager.max_count)


func update_cached_size(i: int) -> void:
	cached_sizes[i] = manager.get_size_scale(manager.levels[i])


var _profiling := false  # Set each frame from manager

func process(delta: float, profiling: bool = false) -> void:
	_profiling = profiling
	var t_start := 0
	var t_sep := 0
	var t_logic := 0
	var t_render := 0

	if profiling:
		t_start = Time.get_ticks_usec()
		# Reset separation sub-profiling
		profile_sep_grid = 0.0
		profile_sep_loop = 0.0
		sep_neighbor_count = 0
		sep_call_count = 0

	var frame: int = Engine.get_process_frames()

	# Get camera position for LOD
	var cam_pos := Vector2.ZERO
	var camera: Camera2D = manager.get_viewport().get_camera_2d()
	if camera:
		cam_pos = camera.global_position
	var cam_x: float = cam_pos.x
	var cam_y: float = cam_pos.y

	var frame_mod8: int = frame % 8

	# Cache array references as locals (faster than member access)
	var npc_count: int = manager.count
	var healths: PackedFloat32Array = manager.healths
	var states: PackedInt32Array = manager.states
	var positions: PackedVector2Array = manager.positions
	var intended_vels: PackedVector2Array = manager.intended_velocities
	var last_logic: PackedInt32Array = manager.last_logic_frame

	# LOD thresholds
	var lod_near_sq: float = Config.LOD_NEAR_SQ
	var lod_mid_sq: float = Config.LOD_MID_SQ
	var lod_far_sq: float = Config.LOD_FAR_SQ

	# Cache awake array
	var awake: PackedByteArray = manager.awake

	for i in npc_count:
		if healths[i] <= 0.0:
			continue
		if awake[i] == 0:
			continue  # Skip sleeping NPCs entirely

		var state: int = states[i]
		# Inline is_stationary: STATIONARY_MASK = 227
		var is_stationary: bool = (227 & (1 << state)) != 0
		if is_stationary:
			intended_vels[i] = Vector2.ZERO

		# Separation runs on its own stagger (every 8 frames per NPC)
		if i % 8 == frame_mod8:
			if profiling:
				var t0 := Time.get_ticks_usec()
				_calc_separation(i)
				t_sep += Time.get_ticks_usec() - t0
			else:
				_calc_separation(i)

		# Stationary NPCs apply separation but skip logic/movement
		if is_stationary:
			if profiling:
				var t0 := Time.get_ticks_usec()
				_update_render(i, delta)
				t_render += Time.get_ticks_usec() - t0
			else:
				_update_render(i, delta)
			# Drift check: working NPCs pushed off their post walk back
			if state == NPCState.State.FARMING or state == NPCState.State.ON_DUTY:
				var anchor: Vector2 = manager.wander_centers[i]
				var cur: Vector2 = positions[i]
				var ddx: float = cur.x - anchor.x
				var ddy: float = cur.y - anchor.y
				if ddx * ddx + ddy * ddy > DRIFT_THRESHOLD_SQ:
					manager._state.set_state(i, NPCState.State.IDLE)
					manager._decide_what_to_do(i)
			continue

		# LOD: inline distance_squared_to and _get_lod_multiplier
		var pos: Vector2 = positions[i]
		var dx: float = pos.x - cam_x
		var dy: float = pos.y - cam_y
		var dist_sq: float = dx * dx + dy * dy

		var lod_mult: int = 1
		if dist_sq >= lod_far_sq:
			lod_mult = 8
		elif dist_sq >= lod_mid_sq:
			lod_mult = 4
		elif dist_sq >= lod_near_sq:
			lod_mult = 2

		# Inline _get_logic_interval
		var logic_interval: int
		if state == 2 or state == 3:  # FIGHTING, FLEEING
			logic_interval = LOGIC_INTERVAL_COMBAT * lod_mult
		elif state == 4 or state == 8 or state == 9 or state == 10:  # WALKING, PATROLLING, RAIDING, RETURNING
			logic_interval = LOGIC_INTERVAL_MOVING * lod_mult
		else:
			logic_interval = LOGIC_INTERVAL_IDLE * lod_mult

		var frames_since_logic: int = frame - last_logic[i]

		if frames_since_logic >= logic_interval:
			if profiling:
				var t0 := Time.get_ticks_usec()
				_update_logic(i, delta * float(frames_since_logic), state)
				t_logic += Time.get_ticks_usec() - t0
			else:
				_update_logic(i, delta * float(frames_since_logic), state)
			last_logic[i] = frame

		if profiling:
			var t0 := Time.get_ticks_usec()
			_update_render(i, delta)
			t_render += Time.get_ticks_usec() - t0
		else:
			_update_render(i, delta)

	if profiling:
		var t_end := Time.get_ticks_usec()
		var total := t_end - t_start
		profile_loop = (total - t_sep - t_logic - t_render) / 1000.0
		profile_sep = t_sep / 1000.0
		profile_logic = t_logic / 1000.0
		profile_render = t_render / 1000.0



func _update_logic(i: int, _accumulated_delta: float, state: int) -> void:
	# Calculate intended velocity based on state
	# WALKING=4, PATROLLING=8, RETURNING=10, RAIDING=9
	if state == 4 or state == 8 or state == 9 or state == 10:
		_calc_move_toward_target(i)
	elif state == 2:  # FIGHTING
		_calc_move_toward_enemy(i)
	elif state == 3:  # FLEEING
		_calc_move_toward_flee_target(i)


func _update_render(i: int, delta: float) -> void:
	var vel: Vector2 = manager.intended_velocities[i]
	var sep: Vector2 = separation_velocities[i]
	var vx: float = vel.x + sep.x
	var vy: float = vel.y + sep.y

	if vx * vx + vy * vy < 0.01:
		return

	var pos: Vector2 = manager.positions[i]
	var new_x: float = pos.x + vx * delta
	var new_y: float = pos.y + vy * delta

	# Check for arrival (WALKING=4, PATROLLING=8, RETURNING=10, RAIDING=9)
	var state: int = manager.states[i]
	if state == 4 or state == 8 or state == 9 or state == 10:
		var target: Vector2 = manager.targets[i]
		var dx: float = target.x - pos.x
		var dy: float = target.y - pos.y
		var dist_sq: float = dx * dx + dy * dy
		var arrival_r: float = manager.arrival_radii[i]
		var move_sq: float = (vel.x * vel.x + vel.y * vel.y) * delta * delta

		if dist_sq < arrival_r * arrival_r or move_sq >= dist_sq:
			# Don't snap to target - stay at current position (separation spreads them out)
			manager.intended_velocities[i] = Vector2.ZERO
			manager.last_logic_frame[i] = 0
			arrived.emit(i)
			return

	manager.positions[i] = Vector2(new_x, new_y)


func _get_move_speed(i: int) -> float:
	var speed: float = Config.MOVE_SPEED
	# Apply move speed upgrade for guards
	if manager.jobs[i] == 1:  # GUARD
		var town_idx: int = manager.town_indices[i]
		if town_idx >= 0 and town_idx < manager.town_upgrades.size():
			var move_level: int = manager.town_upgrades[town_idx].guard_move_speed
			if move_level > 0:
				speed *= 1.0 + (move_level * Config.UPGRADE_GUARD_MOVE_SPEED)
	# Apply Swift trait
	if manager.traits[i] == NPCState.Trait.SWIFT:
		speed *= 1.25
	return speed


func _calc_move_toward_target(i: int) -> void:
	var pos: Vector2 = manager.positions[i]
	var target: Vector2 = manager.targets[i]
	var dx: float = target.x - pos.x
	var dy: float = target.y - pos.y
	var dist_sq: float = dx * dx + dy * dy
	var arrival_r: float = manager.arrival_radii[i]

	if dist_sq < arrival_r * arrival_r:
		manager.intended_velocities[i] = Vector2.ZERO
	else:
		var dist: float = sqrt(dist_sq)
		var speed: float = _get_move_speed(i)
		manager.intended_velocities[i] = Vector2(dx / dist * speed, dy / dist * speed)


func _calc_move_toward_enemy(i: int) -> void:
	var target_idx: int = manager.current_targets[i]

	if target_idx < 0 or manager.healths[target_idx] <= 0.0:
		manager.intended_velocities[i] = Vector2.ZERO
		return

	var pos: Vector2 = manager.positions[i]
	var enemy: Vector2 = manager.positions[target_idx]
	var dx: float = enemy.x - pos.x
	var dy: float = enemy.y - pos.y
	var dist_sq: float = dx * dx + dy * dy
	var attack_range: float = manager.attack_ranges[i]

	if dist_sq > attack_range * attack_range:
		var dist: float = sqrt(dist_sq)
		var speed: float = _get_move_speed(i)
		manager.intended_velocities[i] = Vector2(dx / dist * speed, dy / dist * speed)
	else:
		manager.intended_velocities[i] = Vector2.ZERO


func _calc_move_toward_flee_target(i: int) -> void:
	var pos: Vector2 = manager.positions[i]
	var job: int = manager.jobs[i]

	var flee_target: Vector2
	if job == 2:  # RAIDER
		flee_target = manager.home_positions[i]
	else:
		var town_idx: int = manager.town_indices[i]
		if town_idx >= 0 and town_idx < manager.town_centers.size():
			flee_target = manager.town_centers[town_idx]
		else:
			flee_target = manager.home_positions[i]

	var dx: float = flee_target.x - pos.x
	var dy: float = flee_target.y - pos.y
	var dist: float = sqrt(dx * dx + dy * dy)
	if dist > 0.001:
		var speed: float = _get_move_speed(i) * 1.2  # Flee faster
		manager.intended_velocities[i] = Vector2(dx / dist * speed, dy / dist * speed)


func _calc_separation(i: int) -> void:
	var t0 := 0
	if _profiling:
		t0 = Time.get_ticks_usec()
		sep_call_count += 1

	# Cache array references as locals (faster access)
	var positions: PackedVector2Array = manager.positions
	var targets: PackedVector2Array = manager.targets
	var healths: PackedFloat32Array = manager.healths
	var states: PackedInt32Array = manager.states
	var sizes: PackedFloat32Array = cached_sizes

	var my_pos: Vector2 = positions[i]
	var my_size: float = sizes[i]
	if my_size <= 0.0:
		my_size = 1.0
	var nearby: Array = manager._grid.get_nearby(my_pos)
	var nearby_count: int = nearby.size()

	var t1 := 0
	if _profiling:
		t1 = Time.get_ticks_usec()
		profile_sep_grid += (t1 - t0) / 1000.0
		sep_neighbor_count += nearby_count

	if nearby_count <= 1:
		var prev_vel: Vector2 = separation_velocities[i]
		separation_velocities[i] = prev_vel * 0.6
		return

	var sep_x := 0.0
	var sep_y := 0.0
	var dodge_x := 0.0
	var dodge_y := 0.0
	var my_radius: float = Config.SEPARATION_RADIUS * my_size
	var target_pos: Vector2 = targets[i]
	var to_target: Vector2 = target_pos - my_pos
	var to_target_len: float = sqrt(to_target.x * to_target.x + to_target.y * to_target.y)
	var my_dir_x := 0.0
	var my_dir_y := 0.0
	if to_target_len > 0.001:
		my_dir_x = to_target.x / to_target_len
		my_dir_y = to_target.y / to_target_len

	var sep_radius: float = Config.SEPARATION_RADIUS

	for j in nearby_count:
		var other_idx: int = nearby[j]
		if other_idx == i:
			continue
		if healths[other_idx] <= 0.0:
			continue

		var other_pos: Vector2 = positions[other_idx]
		var diff_x: float = my_pos.x - other_pos.x
		var diff_y: float = my_pos.y - other_pos.y
		var dist_sq: float = diff_x * diff_x + diff_y * diff_y
		if dist_sq <= 0.0:
			continue

		var other_size: float = sizes[other_idx]
		if other_size <= 0.0:
			other_size = 1.0
		var combined_radius: float = (my_radius + sep_radius * other_size) * 0.5
		var combined_radius_sq: float = combined_radius * combined_radius

		if dist_sq < combined_radius_sq:
			var other_state: int = states[other_idx]
			# Inline is_stationary: STATIONARY_MASK = 227 (bits 0,1,5,6,7)
			var other_stationary: bool = (227 & (1 << other_state)) != 0

			var push_strength: float = other_size / my_size
			if other_stationary:
				push_strength *= 3.0
			elif to_target_len > 0.001:
				# Reduce separation for co-moving NPCs (same target direction)
				var other_target: Vector2 = targets[other_idx]
				var otx: float = other_target.x - other_pos.x
				var oty: float = other_target.y - other_pos.y
				var ot_len_sq: float = otx * otx + oty * oty
				if ot_len_sq > 0.001:
					var ot_len: float = sqrt(ot_len_sq)
					var dot: float = my_dir_x * (otx / ot_len) + my_dir_y * (oty / ot_len)
					if dot > 0.5:
						push_strength *= 0.25

			var inv_dist: float = 1.0 / sqrt(dist_sq)
			var factor: float = inv_dist * inv_dist * push_strength
			sep_x += diff_x * factor
			sep_y += diff_y * factor

		# TCP-like collision avoidance
		var approach_radius_sq: float = combined_radius_sq * 4.0
		if dist_sq < approach_radius_sq:
			var other_state: int = states[other_idx]
			# Inline: not is_stationary
			if (227 & (1 << other_state)) == 0:
				var inv_dist: float = 1.0 / sqrt(dist_sq)
				var to_other_x: float = -diff_x * inv_dist
				var to_other_y: float = -diff_y * inv_dist

				var i_approach: float = my_dir_x * to_other_x + my_dir_y * to_other_y
				if i_approach > 0.3:
					# Get other's direction
					var other_target: Vector2 = targets[other_idx]
					var otx: float = other_target.x - other_pos.x
					var oty: float = other_target.y - other_pos.y
					var ot_len: float = sqrt(otx * otx + oty * oty)
					if ot_len > 0.001:
						var other_dir_x: float = otx / ot_len
						var other_dir_y: float = oty / ot_len
						var they_approach: float = -(other_dir_x * to_other_x + other_dir_y * to_other_y)

						var perp_x: float = -my_dir_y
						var perp_y: float = my_dir_x
						var dodge_strength: float = 0.4

						if they_approach > 0.3:
							dodge_strength = 0.5
						elif they_approach < -0.3:
							dodge_strength = 0.3

						if i < other_idx:
							dodge_x += perp_x * dodge_strength
							dodge_y += perp_y * dodge_strength
						else:
							dodge_x -= perp_x * dodge_strength
							dodge_y -= perp_y * dodge_strength

	var final_x := 0.0
	var final_y := 0.0
	var sep_len_sq: float = sep_x * sep_x + sep_y * sep_y
	if sep_len_sq > 0.0:
		var sep_len: float = sqrt(sep_len_sq)
		var sep_strength: float = Config.SEPARATION_STRENGTH
		final_x = (sep_x / sep_len) * sep_strength
		final_y = (sep_y / sep_len) * sep_strength

	var dodge_len_sq: float = dodge_x * dodge_x + dodge_y * dodge_y
	if dodge_len_sq > 0.0:
		var dodge_len: float = sqrt(dodge_len_sq)
		var dodge_strength: float = Config.SEPARATION_STRENGTH * 0.7
		final_x += (dodge_x / dodge_len) * dodge_strength
		final_y += (dodge_y / dodge_len) * dodge_strength

	# Dampen: lerp toward new value to prevent oscillation
	var prev: Vector2 = separation_velocities[i]
	separation_velocities[i] = Vector2(
		prev.x + (final_x - prev.x) * 0.4,
		prev.y + (final_y - prev.y) * 0.4
	)

	if _profiling:
		profile_sep_loop += (Time.get_ticks_usec() - t1) / 1000.0


# ============================================================
# PARALLEL PROCESSING
# ============================================================

var _parallel_delta: float = 0.0
var _parallel_frame: int = 0

func process_parallel(delta: float) -> void:
	_parallel_delta = delta
	_parallel_frame = Engine.get_process_frames()

	# No pre-copy needed - each thread copies its own NPC's position
	var task_id = WorkerThreadPool.add_group_task(_process_single_npc_nav, manager.count)
	WorkerThreadPool.wait_for_group_task_completion(task_id)


func _process_single_npc_nav(i: int) -> void:
	if manager.healths[i] <= 0.0:
		return
	if manager.awake[i] == 0:
		return

	# Copy current position as base (each thread handles its own NPC)
	manager.next_positions[i] = manager.positions[i]

	var state: int = manager.states[i]
	# Inline is_stationary: STATIONARY_MASK = 227
	var is_stationary: bool = (227 & (1 << state)) != 0

	if is_stationary:
		manager.intended_velocities[i] = Vector2.ZERO

	# Separation (staggered every 16 frames per NPC)
	if i % 16 == _parallel_frame % 16:
		_calc_separation_parallel(i)

	if is_stationary:
		_update_render_parallel(i)
		return

	# Logic update (staggered based on state and distance)
	var frames_since_logic: int = _parallel_frame - manager.last_logic_frame[i]
	var logic_interval: int = _get_logic_interval_for_state(state)

	if frames_since_logic >= logic_interval:
		_update_logic(i, _parallel_delta * float(frames_since_logic), state)
		manager.last_logic_frame[i] = _parallel_frame

	_update_render_parallel(i)


func _get_logic_interval_for_state(state: int) -> int:
	if state == 2 or state == 3:  # FIGHTING, FLEEING
		return LOGIC_INTERVAL_COMBAT
	elif state == 4 or state == 8 or state == 9 or state == 10:  # WALKING, PATROLLING, RAIDING, RETURNING
		return LOGIC_INTERVAL_MOVING
	else:
		return LOGIC_INTERVAL_IDLE


func _update_render_parallel(i: int) -> void:
	var vel: Vector2 = manager.intended_velocities[i]
	var sep: Vector2 = separation_velocities[i]
	var vx: float = vel.x + sep.x
	var vy: float = vel.y + sep.y

	if vx * vx + vy * vy < 0.01:
		return

	# Read from next_positions (already copied from positions in _process_single_npc_nav)
	var pos: Vector2 = manager.next_positions[i]
	manager.next_positions[i] = Vector2(pos.x + vx * _parallel_delta, pos.y + vy * _parallel_delta)


func _calc_separation_parallel(i: int) -> void:
	# Thread-safe separation using grid lookup
	var positions: PackedVector2Array = manager.positions
	var targets: PackedVector2Array = manager.targets
	var healths: PackedFloat32Array = manager.healths
	var sizes: PackedFloat32Array = cached_sizes

	var my_pos: Vector2 = positions[i]
	var my_size: float = sizes[i]
	if my_size <= 0.0:
		my_size = 1.0

	var nearby: Array = manager._grid.get_nearby(my_pos)
	var nearby_count: int = nearby.size()
	if nearby_count <= 1:
		var prev_vel: Vector2 = separation_velocities[i]
		separation_velocities[i] = prev_vel * 0.6
		return

	# Compute my direction to target for co-movement check
	var my_target: Vector2 = targets[i]
	var mtx: float = my_target.x - my_pos.x
	var mty: float = my_target.y - my_pos.y
	var mt_len_sq: float = mtx * mtx + mty * mty
	var my_dir_x := 0.0
	var my_dir_y := 0.0
	if mt_len_sq > 0.001:
		var mt_len: float = sqrt(mt_len_sq)
		my_dir_x = mtx / mt_len
		my_dir_y = mty / mt_len

	var sep_x := 0.0
	var sep_y := 0.0
	var my_radius: float = Config.SEPARATION_RADIUS * my_size
	var sep_radius: float = Config.SEPARATION_RADIUS

	for j in nearby_count:
		var other_idx: int = nearby[j]
		if other_idx == i:
			continue
		if healths[other_idx] <= 0.0:
			continue

		var other_pos: Vector2 = positions[other_idx]
		var diff_x: float = my_pos.x - other_pos.x
		var diff_y: float = my_pos.y - other_pos.y
		var dist_sq: float = diff_x * diff_x + diff_y * diff_y
		if dist_sq <= 0.0:
			continue

		var other_size: float = sizes[other_idx]
		if other_size <= 0.0:
			other_size = 1.0
		var combined_radius: float = (my_radius + sep_radius * other_size) * 0.5
		var combined_radius_sq: float = combined_radius * combined_radius

		if dist_sq < combined_radius_sq:
			var dist: float = sqrt(dist_sq)
			var overlap: float = combined_radius - dist
			var push_strength: float = overlap / combined_radius

			# Reduce separation for co-moving NPCs
			if mt_len_sq > 0.001:
				var other_target: Vector2 = targets[other_idx]
				var otx: float = other_target.x - other_pos.x
				var oty: float = other_target.y - other_pos.y
				var ot_len_sq: float = otx * otx + oty * oty
				if ot_len_sq > 0.001:
					var ot_len: float = sqrt(ot_len_sq)
					var dot: float = my_dir_x * (otx / ot_len) + my_dir_y * (oty / ot_len)
					if dot > 0.5:
						push_strength *= 0.25

			sep_x += (diff_x / dist) * push_strength
			sep_y += (diff_y / dist) * push_strength

	var final_x := 0.0
	var final_y := 0.0
	var sep_len_sq: float = sep_x * sep_x + sep_y * sep_y
	if sep_len_sq > 0.0:
		var sep_len: float = sqrt(sep_len_sq)
		var sep_strength: float = Config.SEPARATION_STRENGTH
		final_x = (sep_x / sep_len) * sep_strength
		final_y = (sep_y / sep_len) * sep_strength

	# Dampen: lerp toward new value to prevent oscillation
	var prev: Vector2 = separation_velocities[i]
	separation_velocities[i] = Vector2(
		prev.x + (final_x - prev.x) * 0.4,
		prev.y + (final_y - prev.y) * 0.4
	)


# Internal version for unified parallel processing (called from npc_manager)
func _process_single_npc_nav_internal(i: int, delta: float, frame: int) -> void:
	# Copy current position as base
	manager.next_positions[i] = manager.positions[i]

	var state: int = manager.states[i]
	var is_stationary: bool = (227 & (1 << state)) != 0

	if is_stationary:
		manager.intended_velocities[i] = Vector2.ZERO

	# Separation (skip if GPU already computed it)
	if not manager.use_gpu_separation:
		if i % 16 == frame % 16:
			_calc_separation_parallel(i)

	if is_stationary:
		_update_render_internal(i, delta)
		return

	# Logic update (staggered based on state)
	var frames_since_logic: int = frame - manager.last_logic_frame[i]
	var logic_interval: int = _get_logic_interval_for_state(state)

	if frames_since_logic >= logic_interval:
		_update_logic(i, delta * float(frames_since_logic), state)
		manager.last_logic_frame[i] = frame

	_update_render_internal(i, delta)


func _update_render_internal(i: int, delta: float) -> void:
	var vel: Vector2 = manager.intended_velocities[i]
	var sep: Vector2 = separation_velocities[i]
	var vx: float = vel.x + sep.x
	var vy: float = vel.y + sep.y

	if vx * vx + vy * vy < 0.01:
		return

	var pos: Vector2 = manager.next_positions[i]

	# Check for arrival (WALKING=4, PATROLLING=8, RETURNING=10, RAIDING=9)
	var state: int = manager.states[i]
	if state == 4 or state == 8 or state == 9 or state == 10:
		var target: Vector2 = manager.targets[i]
		var dx: float = target.x - pos.x
		var dy: float = target.y - pos.y
		var dist_sq: float = dx * dx + dy * dy
		var arrival_r: float = manager.arrival_radii[i]
		var move_sq: float = (vel.x * vel.x + vel.y * vel.y) * delta * delta

		if dist_sq < arrival_r * arrival_r or move_sq >= dist_sq:
			manager.intended_velocities[i] = Vector2.ZERO
			manager.last_logic_frame[i] = 0
			manager.pending_arrivals[i] = 1
			return

	manager.next_positions[i] = Vector2(pos.x + vx * delta, pos.y + vy * delta)


# Force immediate logic update (call when state changes, damage taken, etc.)
func force_logic_update(i: int) -> void:
	manager.last_logic_frame[i] = 0


# Reset cached velocities for a slot (call when reusing dead NPC slot)
func reset_slot(i: int) -> void:
	separation_velocities[i] = Vector2.ZERO
	cached_sizes[i] = 1.0
