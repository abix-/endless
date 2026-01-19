# npc_renderer.gd
# Handles MultiMesh rendering with camera culling
extends RefCounted
class_name NPCRenderer

var manager: Node
var multimesh: MultiMesh
var multimesh_instance: MultiMeshInstance2D
var loot_multimesh: MultiMesh
var loot_multimesh_instance: MultiMeshInstance2D
var halo_multimesh: MultiMesh
var halo_multimesh_instance: MultiMeshInstance2D
var sleep_multimesh: MultiMesh
var sleep_multimesh_instance: MultiMeshInstance2D
var rendered_npcs: PackedInt32Array  # Track which NPCs were rendered last frame
var rendered_loot: PackedInt32Array  # Track which loot icons were rendered
var rendered_halos: PackedInt32Array  # Track which halos were rendered
var rendered_sleep: PackedInt32Array  # Track which sleep icons were rendered

const FLASH_DECAY := 8.0  # Flash fades in ~0.12 seconds
const LOOT_ICON_OFFSET := Vector2(0, -12)  # Offset on raider's head
const LOOT_ICON_SCALE := 1.5
const HALO_SCALE := 3.0  # Size of healing halo
const FOUNTAIN_RADIUS := 48.0  # Match npc_needs.gd
const SLEEP_ICON_OFFSET := Vector2(8, -12)  # Offset above/right of head
const SLEEP_ICON_SCALE := 1.0

# Sprite frames (column, row) in the character sheet
const SPRITE_FARMER := Vector2i(1, 6)
const SPRITE_GUARD := Vector2i(0, 11)
const SPRITE_RAIDER := Vector2i(0, 6)

# Job tint colors
const COLOR_FARMER := Color(0.6, 1.0, 0.6)        # Green tint
const COLOR_GUARD := Color(0.6, 0.8, 1.0)         # Blue tint
const COLOR_RAIDER := Color(1.0, 0.6, 0.6)        # Red tint


func _init(npc_manager: Node, mm_instance: MultiMeshInstance2D, loot_instance: MultiMeshInstance2D, halo_inst: MultiMeshInstance2D, sleep_inst: MultiMeshInstance2D) -> void:
	manager = npc_manager
	multimesh_instance = mm_instance
	loot_multimesh_instance = loot_instance
	halo_multimesh_instance = halo_inst
	sleep_multimesh_instance = sleep_inst
	_init_multimesh()
	_init_loot_multimesh()
	_init_halo_multimesh()
	_init_sleep_multimesh()
	_connect_settings()


func _connect_settings() -> void:
	UserSettings.settings_changed.connect(_on_settings_changed)
	_apply_settings()


func _on_settings_changed() -> void:
	_apply_settings()


func _apply_settings() -> void:
	var mat: ShaderMaterial = multimesh_instance.material as ShaderMaterial
	if mat:
		# 0 = off, 1 = when damaged, 2 = always
		mat.set_shader_parameter("hp_bar_mode", UserSettings.hp_bar_mode)


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


func _init_loot_multimesh() -> void:
	loot_multimesh = MultiMesh.new()
	loot_multimesh.transform_format = MultiMesh.TRANSFORM_2D
	loot_multimesh.instance_count = Config.RAIDERS_PER_CAMP * 10  # Max possible raiders with loot
	loot_multimesh.visible_instance_count = 0

	var quad := QuadMesh.new()
	quad.size = Vector2(Config.NPC_SPRITE_SIZE, Config.NPC_SPRITE_SIZE)
	loot_multimesh.mesh = quad

	loot_multimesh_instance.multimesh = loot_multimesh


func _init_halo_multimesh() -> void:
	halo_multimesh = MultiMesh.new()
	halo_multimesh.transform_format = MultiMesh.TRANSFORM_2D
	halo_multimesh.instance_count = manager.max_count
	halo_multimesh.visible_instance_count = 0

	var quad := QuadMesh.new()
	quad.size = Vector2(Config.NPC_SPRITE_SIZE, Config.NPC_SPRITE_SIZE)
	halo_multimesh.mesh = quad

	halo_multimesh_instance.multimesh = halo_multimesh


func _init_sleep_multimesh() -> void:
	sleep_multimesh = MultiMesh.new()
	sleep_multimesh.transform_format = MultiMesh.TRANSFORM_2D
	sleep_multimesh.instance_count = manager.max_count
	sleep_multimesh.visible_instance_count = 0

	var quad := QuadMesh.new()
	quad.size = Vector2(Config.NPC_SPRITE_SIZE, Config.NPC_SPRITE_SIZE)
	sleep_multimesh.mesh = quad

	sleep_multimesh_instance.multimesh = sleep_multimesh


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

	# Hide previously rendered loot icons
	for i in rendered_loot.size():
		loot_multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))
	rendered_loot.clear()

	# Hide previously rendered halos
	for i in rendered_halos.size():
		halo_multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))
	rendered_halos.clear()

	# Hide previously rendered sleep icons
	for i in rendered_sleep.size():
		sleep_multimesh.set_instance_transform_2d(i, Transform2D(0, Vector2(-9999, -9999)))
	rendered_sleep.clear()

	var loot_idx := 0
	var halo_idx := 0
	var sleep_idx := 0

	# Render visible NPCs
	for cell_idx in visible_cells:
		var start: int = manager._grid.grid_cell_starts[cell_idx]
		var cell_count: int = manager._grid.grid_cell_counts[cell_idx]

		for j in cell_count:
			var i: int = manager._grid.grid_cells[start + j]

			if manager.healths[i] <= 0:
				continue

			var pos: Vector2 = manager.positions[i]
			var size_scale: float = manager.get_npc_size_scale(i)
			var xform := Transform2D(0, pos).scaled_local(Vector2(size_scale, size_scale))
			multimesh.set_instance_transform_2d(i, xform)
			rendered_npcs.append(i)

			if manager.health_dirty[i] == 1:
				var health_pct: float = manager.healths[i] / manager.get_scaled_max_health(i)
				var flash: float = manager.flash_timers[i]
				var frame: Vector2i = get_sprite_frame(manager.jobs[i])
				multimesh.set_instance_custom_data(i, Color(health_pct, flash, frame.x / 255.0, frame.y / 255.0))
				manager.health_dirty[i] = 0

			# Render loot icon for raiders carrying food
			if manager.jobs[i] == manager.Job.RAIDER and manager.carrying_food[i] == 1:
				if loot_idx < loot_multimesh.instance_count:
					var loot_pos: Vector2 = pos + LOOT_ICON_OFFSET * size_scale
					var loot_xform := Transform2D(0, loot_pos).scaled_local(Vector2(LOOT_ICON_SCALE, LOOT_ICON_SCALE))
					loot_multimesh.set_instance_transform_2d(loot_idx, loot_xform)
					rendered_loot.append(loot_idx)
					loot_idx += 1

			# Render halo for NPCs receiving healing bonus
			if _is_healing_boosted(i, pos):
				if halo_idx < halo_multimesh.instance_count:
					var halo_xform := Transform2D(0, pos).scaled_local(Vector2(HALO_SCALE, HALO_SCALE))
					halo_multimesh.set_instance_transform_2d(halo_idx, halo_xform)
					rendered_halos.append(halo_idx)
					halo_idx += 1

			# Render sleep z for resting NPCs
			if manager.states[i] == NPCState.State.RESTING:
				if sleep_idx < sleep_multimesh.instance_count:
					var sleep_pos: Vector2 = pos + SLEEP_ICON_OFFSET * size_scale
					var sleep_xform := Transform2D(0, sleep_pos).scaled_local(Vector2(SLEEP_ICON_SCALE * size_scale, SLEEP_ICON_SCALE * size_scale))
					sleep_multimesh.set_instance_transform_2d(sleep_idx, sleep_xform)
					rendered_sleep.append(sleep_idx)
					sleep_idx += 1

	loot_multimesh.visible_instance_count = loot_idx
	halo_multimesh.visible_instance_count = halo_idx
	sleep_multimesh.visible_instance_count = sleep_idx


func _update_all() -> void:
	var loot_idx := 0
	var halo_idx := 0
	var sleep_idx := 0
	for i in manager.count:
		if manager.healths[i] <= 0:
			continue
		var pos: Vector2 = manager.positions[i]
		var size_scale: float = manager.get_npc_size_scale(i)
		var xform := Transform2D(0, pos).scaled_local(Vector2(size_scale, size_scale))
		multimesh.set_instance_transform_2d(i, xform)
		if manager.health_dirty[i] == 1:
			var health_pct: float = manager.healths[i] / manager.get_scaled_max_health(i)
			var flash: float = manager.flash_timers[i]
			var frame: Vector2i = get_sprite_frame(manager.jobs[i])
			multimesh.set_instance_custom_data(i, Color(health_pct, flash, frame.x / 255.0, frame.y / 255.0))
			manager.health_dirty[i] = 0

		# Render loot icon for raiders carrying food
		if manager.jobs[i] == manager.Job.RAIDER and manager.carrying_food[i] == 1:
			if loot_idx < loot_multimesh.instance_count:
				var loot_pos: Vector2 = pos + LOOT_ICON_OFFSET * size_scale
				var loot_xform := Transform2D(0, loot_pos).scaled_local(Vector2(LOOT_ICON_SCALE, LOOT_ICON_SCALE))
				loot_multimesh.set_instance_transform_2d(loot_idx, loot_xform)
				loot_idx += 1

		# Render halo for NPCs receiving healing bonus
		if _is_healing_boosted(i, pos):
			if halo_idx < halo_multimesh.instance_count:
				var halo_xform := Transform2D(0, pos).scaled_local(Vector2(HALO_SCALE, HALO_SCALE))
				halo_multimesh.set_instance_transform_2d(halo_idx, halo_xform)
				halo_idx += 1

		# Render sleep z for resting NPCs
		if manager.states[i] == NPCState.State.RESTING:
			if sleep_idx < sleep_multimesh.instance_count:
				var sleep_pos: Vector2 = pos + SLEEP_ICON_OFFSET * size_scale
				var sleep_xform := Transform2D(0, sleep_pos).scaled_local(Vector2(SLEEP_ICON_SCALE * size_scale, SLEEP_ICON_SCALE * size_scale))
				sleep_multimesh.set_instance_transform_2d(sleep_idx, sleep_xform)
				sleep_idx += 1

	loot_multimesh.visible_instance_count = loot_idx
	halo_multimesh.visible_instance_count = halo_idx
	sleep_multimesh.visible_instance_count = sleep_idx


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


func _is_healing_boosted(i: int, pos: Vector2) -> bool:
	var job: int = manager.jobs[i]
	# Raiders get boost at camp
	if job == manager.Job.RAIDER:
		var home_pos: Vector2 = manager.home_positions[i]
		return pos.distance_to(home_pos) < FOUNTAIN_RADIUS
	# Villagers get boost at fountain
	for center in manager.town_centers:
		if pos.distance_to(center) < FOUNTAIN_RADIUS:
			return true
	return false
