# projectile_manager.gd
# Manages projectiles with MultiMesh rendering
extends Node2D

var npc_manager: Node

# Data arrays
var count := 0
var max_count := Config.MAX_PROJECTILES

var positions: PackedVector2Array
var velocities: PackedVector2Array
var damages: PackedFloat32Array
var factions: PackedInt32Array
var shooters: PackedInt32Array
var lifetimes: PackedFloat32Array
var active: PackedInt32Array

# Pool of free indices
var free_indices: Array[int] = []

# Rendering
@onready var multimesh_instance: MultiMeshInstance2D = $MultiMeshInstance2D
var multimesh: MultiMesh


func _ready() -> void:
	_init_arrays()
	_init_multimesh()


func set_npc_manager(manager: Node) -> void:
	npc_manager = manager


func _init_arrays() -> void:
	positions.resize(max_count)
	velocities.resize(max_count)
	damages.resize(max_count)
	factions.resize(max_count)
	shooters.resize(max_count)
	lifetimes.resize(max_count)
	active.resize(max_count)

	for i in max_count:
		active[i] = 0
		free_indices.append(i)


func _init_multimesh() -> void:
	multimesh = MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_2D
	multimesh.use_colors = true
	multimesh.instance_count = max_count
	multimesh.visible_instance_count = max_count

	var quad := QuadMesh.new()
	quad.size = Vector2(Config.PROJECTILE_SIZE * 2, Config.PROJECTILE_SIZE)
	multimesh.mesh = quad

	multimesh_instance.multimesh = multimesh

	# Hide all initially
	for i in max_count:
		multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))


func fire(from_pos: Vector2, target_pos: Vector2, damage: float, faction: int, shooter: int) -> void:
	if free_indices.is_empty():
		return

	var i: int = free_indices.pop_back()

	var direction: Vector2 = from_pos.direction_to(target_pos)
	var velocity: Vector2 = direction * Config.PROJECTILE_SPEED

	positions[i] = from_pos
	velocities[i] = velocity
	damages[i] = damage
	factions[i] = faction
	shooters[i] = shooter
	lifetimes[i] = Config.PROJECTILE_LIFETIME
	active[i] = 1

	# Set color based on faction
	var color: Color
	if faction == 0:  # Villager
		color = Color.CYAN
	else:  # Raider
		color = Color.ORANGE
	multimesh.set_instance_color(i, color)

	count += 1


func process(delta: float) -> void:
	if npc_manager == null:
		return

	for i in max_count:
		if active[i] == 0:
			continue

		# Update lifetime
		lifetimes[i] -= delta
		if lifetimes[i] <= 0:
			_deactivate(i)
			continue

		# Move projectile
		positions[i] += velocities[i] * delta

		# Check collision with NPCs
		var hit_idx := _check_collision(i)
		if hit_idx >= 0:
			_on_hit(i, hit_idx)
			_deactivate(i)
			continue

		# Update rendering
		var angle: float = velocities[i].angle()
		var transform := Transform2D(angle, positions[i])
		multimesh.set_instance_transform_2d(i, transform)


func _check_collision(proj_idx: int) -> int:
	var pos: Vector2 = positions[proj_idx]
	var faction: int = factions[proj_idx]
	var hit_radius_sq: float = Config.PROJECTILE_HIT_RADIUS * Config.PROJECTILE_HIT_RADIUS

	var nearby: Array = npc_manager._grid.get_nearby(pos)

	for npc_idx in nearby:
		if npc_manager.healths[npc_idx] <= 0:
			continue

		# Check if enemy faction
		var npc_faction: int = npc_manager.factions[npc_idx]
		if npc_faction == faction:
			continue

		var npc_pos: Vector2 = npc_manager.positions[npc_idx]
		var dist_sq: float = pos.distance_squared_to(npc_pos)

		if dist_sq < hit_radius_sq:
			return npc_idx

	return -1


func _on_hit(proj_idx: int, npc_idx: int) -> void:
	var damage: float = damages[proj_idx]
	var shooter: int = shooters[proj_idx]

	npc_manager.healths[npc_idx] -= damage
	npc_manager.mark_health_dirty(npc_idx)
	npc_manager._renderer.trigger_flash(npc_idx)

	if npc_manager.healths[npc_idx] <= 0:
		npc_manager._combat._die(npc_idx)
	else:
		npc_manager._combat._aggro_victim(shooter, npc_idx)


func _deactivate(i: int) -> void:
	active[i] = 0
	free_indices.append(i)
	count -= 1
	multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))
