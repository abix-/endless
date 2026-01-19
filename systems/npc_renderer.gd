# npc_renderer.gd
# Handles MultiMesh rendering with camera culling
extends RefCounted
class_name NPCRenderer

var manager: Node
var multimesh: MultiMesh
var multimesh_instance: MultiMeshInstance2D
var rendered_npcs: PackedInt32Array  # Track which NPCs were rendered last frame

const FLASH_DECAY := 8.0  # Flash fades in ~0.12 seconds

# Sprite frames (column, row) in the character sheet
const SPRITE_FARMER := Vector2i(0, 5)
const SPRITE_GUARD := Vector2i(0, 2)
const SPRITE_RAIDER := Vector2i(0, 7)

# Job tint colors
const COLOR_FARMER := Color(1.0, 1.0, 1.0)        # White (neutral)
const COLOR_GUARD := Color(0.6, 0.8, 1.0)         # Blue tint
const COLOR_RAIDER := Color(1.0, 0.6, 0.6)        # Red tint


func _init(npc_manager: Node, mm_instance: MultiMeshInstance2D) -> void:
	manager = npc_manager
	multimesh_instance = mm_instance
	_init_multimesh()
	_connect_settings()


func _connect_settings() -> void:
	UserSettings.settings_changed.connect(_on_settings_changed)
	_apply_settings()


func _on_settings_changed() -> void:
	_apply_settings()


func _apply_settings() -> void:
	var mat: ShaderMaterial = multimesh_instance.material as ShaderMaterial
	if mat:
		mat.set_shader_parameter("show_hp_always", UserSettings.show_hp_bars_always)


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

	# Hide previously rendered NPCs (only those we tracked)
	for i in rendered_npcs:
		multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))
	rendered_npcs.clear()

	# Render visible NPCs
	for cell_idx in visible_cells:
		var start: int = manager._grid.grid_cell_starts[cell_idx]
		var cell_count: int = manager._grid.grid_cell_counts[cell_idx]

		for j in cell_count:
			var i: int = manager._grid.grid_cells[start + j]

			if manager.healths[i] <= 0:
				continue

			var pos: Vector2 = manager.positions[i]
			var size_scale: float = manager.get_size_scale(manager.levels[i])
			var xform := Transform2D(0, pos).scaled_local(Vector2(size_scale, size_scale))
			multimesh.set_instance_transform_2d(i, xform)
			rendered_npcs.append(i)

			if manager.health_dirty[i] == 1:
				var health_pct: float = manager.healths[i] / manager.get_scaled_max_health(i)
				var flash: float = manager.flash_timers[i]
				var frame: Vector2i = get_sprite_frame(manager.jobs[i])
				multimesh.set_instance_custom_data(i, Color(health_pct, flash, frame.x / 255.0, frame.y / 255.0))
				manager.health_dirty[i] = 0


func _update_all() -> void:
	for i in manager.count:
		if manager.healths[i] <= 0:
			continue
		var size_scale: float = manager.get_size_scale(manager.levels[i])
		var xform := Transform2D(0, manager.positions[i]).scaled_local(Vector2(size_scale, size_scale))
		multimesh.set_instance_transform_2d(i, xform)
		if manager.health_dirty[i] == 1:
			var health_pct: float = manager.healths[i] / manager.get_scaled_max_health(i)
			var flash: float = manager.flash_timers[i]
			var frame: Vector2i = get_sprite_frame(manager.jobs[i])
			multimesh.set_instance_custom_data(i, Color(health_pct, flash, frame.x / 255.0, frame.y / 255.0))
			manager.health_dirty[i] = 0


func set_npc_color(i: int, color: Color) -> void:
	multimesh.set_instance_color(i, color)


func get_sprite_frame(job: int) -> Vector2i:
	match job:
		0: return SPRITE_FARMER  # Job.FARMER
		1: return SPRITE_GUARD   # Job.GUARD
		2: return SPRITE_RAIDER  # Job.RAIDER
		_: return SPRITE_FARMER


func set_npc_sprite(i: int, job: int) -> void:
	var frame: Vector2i = get_sprite_frame(job)
	var health_pct: float = manager.healths[i] / manager.max_healths[i]
	var flash: float = manager.flash_timers[i]
	# Pack: r=health, g=flash, b=frame_x/255, a=frame_y/255
	multimesh.set_instance_custom_data(i, Color(health_pct, flash, frame.x / 255.0, frame.y / 255.0))
	# Set tint color based on job
	multimesh.set_instance_color(i, get_job_color(job))


func get_job_color(job: int) -> Color:
	match job:
		0: return COLOR_FARMER
		1: return COLOR_GUARD
		2: return COLOR_RAIDER
		_: return COLOR_FARMER


func set_npc_health_display(i: int, health_pct: float) -> void:
	var job: int = manager.jobs[i]
	var frame: Vector2i = get_sprite_frame(job)
	var flash: float = manager.flash_timers[i]
	multimesh.set_instance_custom_data(i, Color(health_pct, flash, frame.x / 255.0, frame.y / 255.0))


func trigger_flash(i: int) -> void:
	manager.flash_timers[i] = 1.0
	manager.health_dirty[i] = 1


func set_visible_count(new_count: int) -> void:
	multimesh.visible_instance_count = new_count


func hide_npc(i: int) -> void:
	multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))


func show_npc(i: int, pos: Vector2) -> void:
	multimesh.set_instance_transform_2d(i, Transform2D(0, pos))
