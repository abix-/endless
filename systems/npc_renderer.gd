# npc_renderer.gd
# Handles MultiMesh rendering with camera culling
extends RefCounted
class_name NPCRenderer

var manager: Node
var multimesh: MultiMesh
var multimesh_instance: MultiMeshInstance2D

const FLASH_DECAY := 8.0  # Flash fades in ~0.12 seconds


func _init(npc_manager: Node, mm_instance: MultiMeshInstance2D) -> void:
	manager = npc_manager
	multimesh_instance = mm_instance
	_init_multimesh()


func _init_multimesh() -> void:
	multimesh = MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_2D
	multimesh.use_colors = true
	multimesh.use_custom_data = true
	multimesh.instance_count = manager.max_count
	multimesh.visible_instance_count = 0

	var quad := QuadMesh.new()
	quad.size = Vector2(Config.NPC_SPRITE_SIZE, Config.NPC_SPRITE_SIZE)
	multimesh.mesh = quad

	multimesh_instance.multimesh = multimesh


func update(delta: float) -> void:
	# Decay flash timers
	for i in manager.count:
		if manager.flash_timers[i] > 0:
			manager.flash_timers[i] = maxf(0.0, manager.flash_timers[i] - delta * FLASH_DECAY)
			manager.health_dirty[i] = 1  # Force custom_data update

	var camera: Camera2D = manager.get_viewport().get_camera_2d()
	if not camera:
		_update_all()
		return

	var cam_pos: Vector2 = camera.global_position
	var view_size: Vector2 = manager.get_viewport_rect().size / camera.zoom
	var margin := Config.RENDER_MARGIN

	var min_x: float = cam_pos.x - view_size.x / 2 - margin
	var max_x: float = cam_pos.x + view_size.x / 2 + margin
	var min_y: float = cam_pos.y - view_size.y / 2 - margin
	var max_y: float = cam_pos.y + view_size.y / 2 + margin

	var visible_cells: PackedInt32Array = manager._grid.get_cells_in_rect(min_x, max_x, min_y, max_y)

	# Hide previously rendered NPCs
	for i in manager.count:
		if manager.last_rendered[i] == 1:
			manager.last_rendered[i] = 0
			multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))

	# Render visible NPCs
	for cell_idx in visible_cells:
		var start: int = manager._grid.grid_cell_starts[cell_idx]
		var cell_count: int = manager._grid.grid_cell_counts[cell_idx]

		for j in cell_count:
			var i: int = manager._grid.grid_cells[start + j]

			if manager.healths[i] <= 0:
				continue

			var pos: Vector2 = manager.positions[i]
			multimesh.set_instance_transform_2d(i, Transform2D(0, pos))
			manager.last_rendered[i] = 1

			if manager.health_dirty[i] == 1:
				var health_pct: float = manager.healths[i] / manager.max_healths[i]
				var flash: float = manager.flash_timers[i]
				multimesh.set_instance_custom_data(i, Color(health_pct, flash, 0, 0))
				manager.health_dirty[i] = 0


func _update_all() -> void:
	for i in manager.count:
		if manager.healths[i] <= 0:
			continue
		multimesh.set_instance_transform_2d(i, Transform2D(0, manager.positions[i]))
		if manager.health_dirty[i] == 1:
			var health_pct: float = manager.healths[i] / manager.max_healths[i]
			var flash: float = manager.flash_timers[i]
			multimesh.set_instance_custom_data(i, Color(health_pct, flash, 0, 0))
			manager.health_dirty[i] = 0


func set_npc_color(i: int, color: Color) -> void:
	multimesh.set_instance_color(i, color)


func set_npc_health_display(i: int, health_pct: float) -> void:
	var flash: float = manager.flash_timers[i]
	multimesh.set_instance_custom_data(i, Color(health_pct, flash, 0, 0))


func trigger_flash(i: int) -> void:
	manager.flash_timers[i] = 1.0
	manager.health_dirty[i] = 1


func set_visible_count(new_count: int) -> void:
	multimesh.visible_instance_count = new_count


func hide_npc(i: int) -> void:
	multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))


func show_npc(i: int, pos: Vector2) -> void:
	multimesh.set_instance_transform_2d(i, Transform2D(0, pos))
