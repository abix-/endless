# left_panel.gd
# Unified left panel with collapsible sections: Stats, Performance, Inspector
extends CanvasLayer

@onready var panel: PanelContainer = $Panel
@onready var stats_header: Button = $Panel/MarginContainer/VBox/StatsHeader
@onready var stats_content: VBoxContainer = $Panel/MarginContainer/VBox/StatsContent
@onready var perf_header: Button = $Panel/MarginContainer/VBox/PerfHeader
@onready var perf_content: VBoxContainer = $Panel/MarginContainer/VBox/PerfContent
@onready var inspector_header: Button = $Panel/MarginContainer/VBox/InspectorHeader
@onready var inspector_content: VBoxContainer = $Panel/MarginContainer/VBox/InspectorContent

# Stats labels
@onready var stats_grid: GridContainer = $Panel/MarginContainer/VBox/StatsContent/StatsGrid
@onready var time_label: Label = $Panel/MarginContainer/VBox/StatsContent/TimeLabel
@onready var food_label: Label = $Panel/MarginContainer/VBox/StatsContent/FoodLabel
@onready var bed_label: Label = $Panel/MarginContainer/VBox/StatsContent/BedLabel
@onready var upgrades_btn: Button = $Panel/MarginContainer/VBox/StatsContent/TownButtons/UpgradesBtn
@onready var roster_btn: Button = $Panel/MarginContainer/VBox/StatsContent/TownButtons/RosterBtn
@onready var policies_btn: Button = $Panel/MarginContainer/VBox/StatsContent/TownButtons/PoliciesBtn

# Perf labels
@onready var perf_label: RichTextLabel = $Panel/MarginContainer/VBox/PerfContent/PerfLabel
@onready var perf_toggle: Button = $Panel/MarginContainer/VBox/PerfContent/PerfRow/PerfToggle
@onready var perf_copy: Button = $Panel/MarginContainer/VBox/PerfContent/PerfRow/PerfCopy
@onready var radius_toggle: Button = $Panel/MarginContainer/VBox/PerfContent/PerfRow/RadiusToggle
@onready var parallel_toggle: Button = $Panel/MarginContainer/VBox/PerfContent/PerfRow2/ParallelToggle
@onready var gpu_toggle: Button = $Panel/MarginContainer/VBox/PerfContent/PerfRow2/GPUToggle

# Inspector labels
@onready var job_level: Label = $Panel/MarginContainer/VBox/InspectorContent/NameRow/JobLevel
@onready var name_edit: LineEdit = $Panel/MarginContainer/VBox/InspectorContent/NameRow/NameEdit
@onready var rename_btn: Button = $Panel/MarginContainer/VBox/InspectorContent/NameRow/RenameBtn
@onready var town_label: Label = $Panel/MarginContainer/VBox/InspectorContent/Town
@onready var health_bar: ProgressBar = $Panel/MarginContainer/VBox/InspectorContent/HealthRow/Bar
@onready var health_value: Label = $Panel/MarginContainer/VBox/InspectorContent/HealthRow/Value
@onready var energy_bar: ProgressBar = $Panel/MarginContainer/VBox/InspectorContent/EnergyRow/Bar
@onready var energy_value: Label = $Panel/MarginContainer/VBox/InspectorContent/EnergyRow/Value
@onready var xp_label: Label = $Panel/MarginContainer/VBox/InspectorContent/XP
@onready var state_label: Label = $Panel/MarginContainer/VBox/InspectorContent/State
@onready var target_label: Label = $Panel/MarginContainer/VBox/InspectorContent/Target
@onready var stats_label: Label = $Panel/MarginContainer/VBox/InspectorContent/Stats
@onready var extra_label: Label = $Panel/MarginContainer/VBox/InspectorContent/Extra
@onready var follow_btn: Button = $Panel/MarginContainer/VBox/InspectorContent/ButtonRow/FollowBtn
@onready var copy_btn: Button = $Panel/MarginContainer/VBox/InspectorContent/ButtonRow/CopyBtn

# Grid cells
var farmer_alive: Label
var farmer_dead: Label
var farmer_kills: Label
var guard_alive: Label
var guard_dead: Label
var guard_kills: Label
var raider_alive: Label
var raider_dead: Label
var raider_kills: Label

var npc_manager: Node
var main_node: Node
var player: Node
var _uses_methods := false  # True for EcsNpcManager, false for GDScript manager

# State
var pinned := true  # Always pinned - keep showing last selected NPC
var last_idx := -1
var following := false  # Camera follows selected NPC
var dragging := false
var drag_offset := Vector2.ZERO

const SETTINGS_KEY := "left_panel_pos"
const COLLAPSE_KEY := "left_panel_collapse"


# Helpers to abstract EcsNpcManager (methods) vs GDScript manager (properties)
func _get_npc_count() -> int:
	if not npc_manager:
		return 0
	if _uses_methods:
		return npc_manager.get_npc_count()
	return npc_manager.count

func _get_npc_health(idx: int) -> float:
	if not npc_manager or idx < 0:
		return 0.0
	if _uses_methods:
		return npc_manager.get_npc_health(idx)
	if idx < npc_manager.healths.size():
		return npc_manager.healths[idx]
	return 0.0

func _get_npc_position(idx: int) -> Vector2:
	if not npc_manager or idx < 0:
		return Vector2.ZERO
	if _uses_methods:
		return npc_manager.get_npc_position(idx)
	if idx < npc_manager.positions.size():
		return npc_manager.positions[idx]
	return Vector2.ZERO


func _ready() -> void:
	add_to_group("left_panel")
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	player = get_tree().get_first_node_in_group("player")
	main_node = get_parent()
	# Detect EcsNpcManager (uses methods) vs GDScript manager (uses properties)
	_uses_methods = npc_manager and npc_manager.has_method("get_npc_count")

	# Connect headers
	stats_header.pressed.connect(_toggle_section.bind("stats"))
	perf_header.pressed.connect(_toggle_section.bind("perf"))
	inspector_header.pressed.connect(_toggle_section.bind("inspector"))

	# Connect buttons
	perf_toggle.toggled.connect(_on_perf_toggled)
	perf_copy.pressed.connect(_on_perf_copy_pressed)
	radius_toggle.toggled.connect(_on_radius_toggled)
	parallel_toggle.toggled.connect(_on_parallel_toggled)
	gpu_toggle.toggled.connect(_on_gpu_toggled)
	follow_btn.toggled.connect(_on_follow_toggled)
	copy_btn.pressed.connect(_on_copy_pressed)
	rename_btn.pressed.connect(_on_rename_pressed)
	name_edit.text_submitted.connect(_on_name_submitted)
	name_edit.focus_exited.connect(_on_name_focus_lost)
	upgrades_btn.pressed.connect(_on_upgrades_pressed)
	roster_btn.pressed.connect(_on_roster_pressed)
	policies_btn.pressed.connect(_on_policies_pressed)

	# Get grid cells
	var cells := stats_grid.get_children()
	farmer_alive = cells[5]
	farmer_dead = cells[6]
	farmer_kills = cells[7]
	guard_alive = cells[9]
	guard_dead = cells[10]
	guard_kills = cells[11]
	raider_alive = cells[13]
	raider_dead = cells[14]
	raider_kills = cells[15]

	# Load saved state
	_load_state()
	_update_perf_toggle()
	_update_radius_toggle()
	_update_parallel_toggle()
	_update_gpu_toggle()


func _input(event: InputEvent) -> void:
	var rect := Rect2(panel.position, panel.size)
	var mouse_over := rect.has_point(get_viewport().get_mouse_position())

	if event is InputEventMouseButton:
		# Block scroll wheel over panel
		if mouse_over and (event.button_index == MOUSE_BUTTON_WHEEL_UP or event.button_index == MOUSE_BUTTON_WHEEL_DOWN):
			get_viewport().set_input_as_handled()
			return

		if event.button_index == MOUSE_BUTTON_LEFT:
			if event.pressed:
				if mouse_over:
					dragging = true
					drag_offset = event.position - panel.position
			else:
				if dragging:
					dragging = false
					UserSettings.set_setting(SETTINGS_KEY, panel.position)

	elif event is InputEventMouseMotion and dragging:
		panel.position = event.position - drag_offset


func _process(_delta: float) -> void:
	if not npc_manager:
		return

	# Camera follow
	if following and last_idx >= 0:
		if last_idx < _get_npc_count() and _get_npc_health(last_idx) > 0:
			var npc_pos: Vector2 = _get_npc_position(last_idx)
			if player:
				player.global_position = npc_pos
		else:
			# NPC died, stop following
			_set_following(false)

	if Engine.get_process_frames() % 10 != 0:
		return

	_update_stats()
	_update_perf()
	_update_inspector()


func _update_stats() -> void:
	if not stats_content.visible:
		return

	# Unit counts (EcsNpcManager doesn't expose these yet)
	if _uses_methods:
		farmer_alive.text = "-"
		farmer_dead.text = "-"
		farmer_kills.text = "-"
		guard_alive.text = "-"
		guard_dead.text = "-"
		guard_kills.text = "-"
		raider_alive.text = "-"
		raider_dead.text = "-"
		raider_kills.text = "-"
	else:
		farmer_alive.text = str(npc_manager.alive_farmers)
		farmer_dead.text = str(npc_manager.dead_farmers)
		farmer_kills.text = "-"
		guard_alive.text = str(npc_manager.alive_guards)
		guard_dead.text = str(npc_manager.dead_guards)
		guard_kills.text = str(npc_manager.raider_kills)
		raider_alive.text = str(npc_manager.alive_raiders)
		raider_dead.text = str(npc_manager.dead_raiders)
		raider_kills.text = str(npc_manager.villager_kills)

	# Time
	var period := "Day" if WorldClock.is_daytime() else "Night"
	time_label.text = "Day %d - %02d:%02d (%s)" % [
		WorldClock.current_day,
		WorldClock.current_hour,
		WorldClock.current_minute,
		period
	]

	# Food (read from Rust ECS)
	var town_total: int = npc_manager.get_town_food(0)
	var camp_total: int = npc_manager.get_camp_food(0)
	food_label.text = "Food: %d vs %d" % [town_total, camp_total]

	# Beds (player's town) - GDScript manager only
	if not _uses_methods and main_node and "player_town_idx" in main_node:
		var player_town: int = main_node.player_town_idx
		var free_beds: int = npc_manager.get_free_bed_count(player_town)
		var total_beds: int = npc_manager.get_total_bed_count(player_town)
		var used_beds: int = total_beds - free_beds
		bed_label.text = "Beds: %d used, %d free" % [used_beds, free_beds]
	elif _uses_methods:
		bed_label.text = "Beds: -"


func _update_perf() -> void:
	if not perf_content.visible:
		return

	var m = npc_manager
	var lines: PackedStringArray = []

	# FPS and zoom
	var fps := int(Engine.get_frames_per_second())
	var zoom_str := "?"
	if player:
		var camera: Camera2D = player.get_node_or_null("Camera2D")
		if camera:
			zoom_str = "%.1fx" % camera.zoom.x
	lines.append("FPS: %d | Zoom: %s" % [fps, zoom_str])

	# EcsNpcManager: simple stats from debug methods
	if _uses_methods:
		var stats: Dictionary = m.get_debug_stats()
		lines.append("NPCs: %d" % stats.get("npc_count", 0))
		lines.append("Arrived: %d | Cells: %d" % [stats.get("arrived_count", 0), stats.get("cells_used", 0)])
		perf_label.text = "\n".join(lines)
		return

	# NPC breakdown
	var alive: int = m.alive_farmers + m.alive_guards + m.alive_raiders
	var moving := 0
	var awake_count := 0
	for i in m.count:
		if m.healths[i] <= 0:
			continue
		if m.awake[i] == 1:
			awake_count += 1
		var state: int = m.states[i]
		# Stationary states bitmask: IDLE, RESTING, FARMING, OFF_DUTY, ON_DUTY
		if (227 & (1 << state)) == 0:
			moving += 1
	lines.append("NPCs: %d | Awake: %d | Moving: %d" % [alive, awake_count, moving])

	if not UserSettings.perf_metrics:
		lines.append("Loop: %.1fms" % m.last_loop_time)
	else:
		var n = m._nav
		lines.append("")
		lines.append("Loop: %.1fms" % m.last_loop_time)
		lines.append("  Sleep:   %.1f" % m.profile_sleep)
		lines.append("  Grid:    %.1f" % m.profile_grid)
		if m.use_gpu_separation:
			lines.append("  GPU Sep: %.1f" % m.profile_gpu_sep)
		lines.append("  Scan:    %.1f" % m.profile_scan)
		lines.append("  Combat:  %.1f" % m.profile_combat)
		lines.append("  Nav:     %.1f" % m.profile_nav)
		lines.append("  Proj:    %.1f" % m.profile_projectiles)
		lines.append("  Render:  %.1f" % m.profile_render)
		lines.append("")
		lines.append("Nav: Sep %.1f | Logic %.1f" % [n.profile_sep, n.profile_logic])
		if n.sep_call_count > 0:
			var avg := float(n.sep_neighbor_count) / float(n.sep_call_count)
			lines.append("Sep: %d calls, %.0f avg neighbors" % [n.sep_call_count, avg])

	perf_label.text = "\n".join(lines)


func _update_inspector() -> void:
	# EcsNpcManager doesn't expose inspector data yet
	if _uses_methods:
		inspector_header.text = "▼ Inspector" if inspector_content.visible else "▶ Inspector"
		if inspector_content.visible:
			job_level.text = "ECS mode"
			town_label.visible = false
			health_bar.get_parent().visible = false
			energy_bar.get_parent().visible = false
			xp_label.visible = false
			state_label.visible = false
			target_label.visible = false
			stats_label.visible = false
			extra_label.visible = false
		return

	var idx: int = npc_manager.selected_npc

	# Update header with selection state
	if idx >= 0:
		inspector_header.text = "▼ Inspector [#%d]" % idx if inspector_content.visible else "▶ Inspector [#%d]" % idx
	else:
		inspector_header.text = "▼ Inspector" if inspector_content.visible else "▶ Inspector"

	if not inspector_content.visible:
		return

	# Handle no NPC selection - show terrain tile if selected
	if idx < 0 and not pinned:
		if main_node and not main_node.selected_tile.is_empty():
			_show_terrain_info(main_node.selected_tile)
			return
		job_level.text = "No selection"
		town_label.visible = false
		health_bar.get_parent().visible = false
		energy_bar.get_parent().visible = false
		xp_label.visible = false
		state_label.visible = false
		target_label.visible = false
		stats_label.visible = false
		extra_label.visible = false
		return

	if idx < 0:
		idx = last_idx
	if idx < 0 or idx >= _get_npc_count() or _get_npc_health(idx) <= 0:
		if following:
			_set_following(false)
		return

	# Stop following if user selected a different NPC
	if idx != last_idx and following:
		_set_following(false)

	last_idx = idx

	# Show all fields
	town_label.visible = true
	health_bar.get_parent().visible = true
	energy_bar.get_parent().visible = true
	xp_label.visible = true
	state_label.visible = true
	target_label.visible = true
	stats_label.visible = true
	extra_label.visible = true

	var job: int = npc_manager.jobs[idx]
	var level: int = npc_manager.levels[idx]
	var job_name: String = NPCState.JOB_NAMES[job] if job < NPCState.JOB_NAMES.size() else "NPC"
	var npc_name: String = npc_manager.npc_names[idx]
	var npc_trait: int = npc_manager.traits[idx]
	var trait_name: String = NPCState.TRAIT_NAMES.get(npc_trait, "")
	job_level.text = "%s - %s Lv.%d" % [npc_name, job_name, level]
	if not trait_name.is_empty():
		job_level.text += " (%s)" % trait_name

	# Town
	var town_idx: int = npc_manager.town_indices[idx]
	if town_idx >= 0 and main_node and "towns" in main_node and town_idx < main_node.towns.size():
		town_label.text = main_node.towns[town_idx].name
	else:
		town_label.text = "-"

	# Health/Energy
	var hp: float = npc_manager.healths[idx]
	var base_max_hp: float = npc_manager.max_healths[idx]
	var max_hp: float = npc_manager.get_scaled_max_health(idx)
	health_bar.value = hp / max_hp if max_hp > 0 else 0.0
	if max_hp > base_max_hp:
		health_value.text = "%d/%d (base %d)" % [int(hp), int(max_hp), int(base_max_hp)]
	else:
		health_value.text = "%d/%d" % [int(hp), int(max_hp)]

	var energy: float = npc_manager.energies[idx]
	energy_bar.value = energy / Config.ENERGY_MAX
	energy_value.text = "%d" % int(energy)

	# XP
	var xp: int = npc_manager.xp[idx]
	var next_xp: int = npc_manager.get_xp_for_next_level(level)
	xp_label.text = "XP: %d/%d" % [xp, next_xp]

	# State
	var state: int = npc_manager.states[idx]
	state_label.text = NPCState.STATE_NAMES.get(state, "?")

	# Target
	var target_npc: int = npc_manager.current_targets[idx]
	if target_npc >= 0 and target_npc < npc_manager.count and npc_manager.healths[target_npc] > 0:
		var t_job: int = npc_manager.jobs[target_npc]
		var t_name: String = NPCState.JOB_NAMES[t_job] if t_job < NPCState.JOB_NAMES.size() else "NPC"
		target_label.text = "Target: %s #%d" % [t_name, target_npc]
	else:
		target_label.text = "Target: -"

	# Stats - show base → effective for upgradeable stats
	var stats_lines: PackedStringArray = []
	var base_dmg: float = npc_manager.attack_damages[idx]
	var base_rng: float = npc_manager.attack_ranges[idx]
	var eff_dmg: float = npc_manager.get_scaled_damage(idx)
	var pos: Vector2 = npc_manager.positions[idx]

	# Get upgrade levels for this NPC's town
	var upgrades: Dictionary = {}
	if town_idx >= 0 and main_node and "town_upgrades" in main_node and town_idx < main_node.town_upgrades.size():
		upgrades = main_node.town_upgrades[town_idx]

	if job == NPCState.Job.GUARD:
		# Show all guard upgradeable stats
		var eff_rng: float = base_rng
		if upgrades.get("guard_range", 0) > 0:
			eff_rng = base_rng * (1.0 + upgrades.guard_range * Config.UPGRADE_GUARD_RANGE_BONUS)

		var base_cooldown: float = Config.ATTACK_COOLDOWN
		var eff_cooldown: float = base_cooldown
		if upgrades.get("guard_attack_speed", 0) > 0:
			eff_cooldown = base_cooldown * (1.0 - upgrades.guard_attack_speed * Config.UPGRADE_GUARD_ATTACK_SPEED)

		var base_speed: float = Config.MOVE_SPEED
		var eff_speed: float = base_speed
		if upgrades.get("guard_move_speed", 0) > 0:
			eff_speed = base_speed * (1.0 + upgrades.guard_move_speed * Config.UPGRADE_GUARD_MOVE_SPEED)

		stats_lines.append("Dmg: %.0f→%.0f | Rng: %.0f→%.0f" % [base_dmg, eff_dmg, base_rng, eff_rng])
		stats_lines.append("Cooldown: %.2f→%.2fs | Spd: %.0f→%.0f" % [base_cooldown, eff_cooldown, base_speed, eff_speed])
		stats_lines.append("Size: +%.0f%%" % (npc_manager.size_bonuses[idx] * 100))
	elif job == NPCState.Job.FARMER:
		stats_lines.append("Dmg: %.0f | Rng: %.0f" % [base_dmg, base_rng])
	else:  # Raider
		stats_lines.append("Dmg: %.0f→%.0f | Rng: %.0f" % [base_dmg, eff_dmg, base_rng])

	stats_lines.append("Pos: %d, %d" % [int(pos.x), int(pos.y)])
	stats_label.text = "\n".join(stats_lines)

	# Extra
	var extra: PackedStringArray = []
	if job == NPCState.Job.GUARD:
		extra.append("Night: %s" % ("Y" if npc_manager.works_at_night[idx] == 1 else "N"))
	elif job == NPCState.Job.RAIDER:
		extra.append("Food: %s" % ("Y" if npc_manager.carrying_food[idx] == 1 else "N"))
	if npc_manager.recovering[idx] == 1:
		extra.append("Recovering")
	extra_label.text = " | ".join(extra)


func _show_terrain_info(tile: Dictionary) -> void:
	# Hide NPC-specific fields
	town_label.visible = false
	health_bar.get_parent().visible = false
	energy_bar.get_parent().visible = false
	xp_label.visible = false
	target_label.visible = false
	extra_label.visible = false

	# Show terrain info using available labels
	job_level.text = "Terrain Tile"
	state_label.visible = true
	state_label.text = tile.biome_name
	stats_label.visible = true
	stats_label.text = "Grid: (%d, %d)\nWorld: (%d, %d)\nSprite: (%d, %d)" % [
		tile.grid_x, tile.grid_y,
		tile.world_x, tile.world_y,
		tile.sprite_col, tile.sprite_row
	]


func _toggle_section(section: String) -> void:
	match section:
		"stats":
			stats_content.visible = not stats_content.visible
			stats_header.text = ("▼ " if stats_content.visible else "▶ ") + "Stats"
		"perf":
			perf_content.visible = not perf_content.visible
			perf_header.text = ("▼ " if perf_content.visible else "▶ ") + "Performance"
		"inspector":
			inspector_content.visible = not inspector_content.visible
			# Header updated in _update_inspector
	_save_collapse_state()


func _on_perf_toggled(enabled: bool) -> void:
	UserSettings.set_perf_metrics(enabled)
	_update_perf_toggle()


func _update_perf_toggle() -> void:
	perf_toggle.button_pressed = UserSettings.perf_metrics
	perf_toggle.text = "Detail: " + ("ON" if UserSettings.perf_metrics else "OFF")


func _on_radius_toggled(enabled: bool) -> void:
	UserSettings.set_show_active_radius(enabled)
	_update_radius_toggle()
	# Force main to redraw
	if main_node:
		main_node.queue_redraw()


func _update_radius_toggle() -> void:
	radius_toggle.button_pressed = UserSettings.show_active_radius


func _on_parallel_toggled(enabled: bool) -> void:
	if npc_manager and "use_parallel" in npc_manager:
		npc_manager.use_parallel = enabled
	_update_parallel_toggle()


func _update_parallel_toggle() -> void:
	# EcsNpcManager runs parallel by default, hide toggle
	if not npc_manager or not "use_parallel" in npc_manager:
		parallel_toggle.visible = false
		return
	parallel_toggle.visible = true
	parallel_toggle.button_pressed = npc_manager.use_parallel
	parallel_toggle.text = "Parallel: " + ("ON" if npc_manager.use_parallel else "OFF")


func _on_gpu_toggled(enabled: bool) -> void:
	if npc_manager and "use_gpu_separation" in npc_manager:
		npc_manager.use_gpu_separation = enabled
	_update_gpu_toggle()


func _update_gpu_toggle() -> void:
	# EcsNpcManager uses GPU compute, hide toggle
	if not npc_manager or not "_gpu_separation" in npc_manager:
		gpu_toggle.visible = false
		return
	var gpu_available: bool = npc_manager._gpu_separation and npc_manager._gpu_separation.is_initialized
	gpu_toggle.visible = gpu_available
	if gpu_available:
		gpu_toggle.button_pressed = npc_manager.use_gpu_separation
		gpu_toggle.text = "GPU: " + ("ON" if npc_manager.use_gpu_separation else "OFF")


func _on_perf_copy_pressed() -> void:
	DisplayServer.clipboard_set(perf_label.text)
	perf_copy.text = "Copied!"
	await get_tree().create_timer(1.0).timeout
	perf_copy.text = "Copy"


func _on_copy_pressed() -> void:
	if _uses_methods or last_idx < 0 or last_idx >= _get_npc_count():
		return
	DisplayServer.clipboard_set(_format_npc_data(last_idx))
	copy_btn.text = "Copied!"
	await get_tree().create_timer(1.0).timeout
	copy_btn.text = "Copy"


func _on_follow_toggled(enabled: bool) -> void:
	_set_following(enabled)


func _set_following(enabled: bool) -> void:
	following = enabled
	follow_btn.button_pressed = enabled
	follow_btn.text = "Following" if enabled else "Follow"


func _on_rename_pressed() -> void:
	if _uses_methods or last_idx < 0 or last_idx >= _get_npc_count():
		return
	name_edit.text = npc_manager.npc_names[last_idx]
	job_level.visible = false
	name_edit.visible = true
	name_edit.grab_focus()
	name_edit.select_all()


func _on_name_submitted(new_name: String) -> void:
	if not _uses_methods and last_idx >= 0 and last_idx < _get_npc_count() and not new_name.strip_edges().is_empty():
		npc_manager.npc_names[last_idx] = new_name.strip_edges()
	_close_name_edit()


func _on_name_focus_lost() -> void:
	_close_name_edit()


func _close_name_edit() -> void:
	name_edit.visible = false
	job_level.visible = true


func _format_npc_data(i: int) -> String:
	if _uses_methods:
		return "NPC #%d (ECS mode - export not available)" % i

	var lines: PackedStringArray = []
	lines.append("NPC Export #%d" % i)

	var job: int = npc_manager.jobs[i]
	var job_name: String = NPCState.JOB_NAMES[job] if job < NPCState.JOB_NAMES.size() else "NPC"
	var npc_name: String = npc_manager.npc_names[i]
	var npc_trait: int = npc_manager.traits[i]
	var trait_name: String = NPCState.TRAIT_NAMES.get(npc_trait, "None")
	lines.append("%s - %s Lv.%d" % [npc_name, job_name, npc_manager.levels[i]])
	if not trait_name.is_empty():
		lines.append("Trait: %s" % trait_name)

	var town_idx: int = npc_manager.town_indices[i]
	var town_name := "-"
	if town_idx >= 0 and main_node and "towns" in main_node and town_idx < main_node.towns.size():
		town_name = main_node.towns[town_idx].name
	lines.append("Town: %s (idx %d)" % [town_name, town_idx])
	lines.append("")

	var hp: float = npc_manager.healths[i]
	var max_hp: float = npc_manager.get_scaled_max_health(i)
	lines.append("HP: %.0f/%.0f | Energy: %.0f" % [hp, max_hp, npc_manager.energies[i]])
	lines.append("XP: %d | State: %s" % [npc_manager.xp[i], NPCState.STATE_NAMES.get(npc_manager.states[i], "?")])
	lines.append("")

	lines.append("Dmg: %.1f | Rng: %.0f" % [npc_manager.get_scaled_damage(i), npc_manager.attack_ranges[i]])
	var pos: Vector2 = npc_manager.positions[i]
	var vel: Vector2 = npc_manager.velocities[i]
	lines.append("Pos: %.0f, %.0f | Vel: %.0f, %.0f" % [pos.x, pos.y, vel.x, vel.y])
	lines.append("")

	lines.append("Target NPC: %d" % npc_manager.current_targets[i])
	lines.append("Home: %.0f, %.0f" % [npc_manager.home_positions[i].x, npc_manager.home_positions[i].y])
	lines.append("Work: %.0f, %.0f" % [npc_manager.work_positions[i].x, npc_manager.work_positions[i].y])
	lines.append("")

	lines.append("Flags: flee=%d night=%d food=%d recovering=%d" % [
		npc_manager.will_flee[i], npc_manager.works_at_night[i],
		npc_manager.carrying_food[i], npc_manager.recovering[i]
	])
	lines.append("Timers: atk=%.2f scan=%.2f patrol=%d" % [
		npc_manager.attack_timers[i], npc_manager.scan_timers[i], npc_manager.patrol_timer[i]
	])

	return "\n".join(lines)


func _load_state() -> void:
	if UserSettings.has_setting(SETTINGS_KEY):
		panel.position = UserSettings.get_setting(SETTINGS_KEY)

	if UserSettings.has_setting(COLLAPSE_KEY):
		var state: Dictionary = UserSettings.get_setting(COLLAPSE_KEY)
		if "stats" in state:
			stats_content.visible = state.stats
			stats_header.text = ("▼ " if state.stats else "▶ ") + "Stats"
		if "perf" in state:
			perf_content.visible = state.perf
			perf_header.text = ("▼ " if state.perf else "▶ ") + "Performance"
		if "inspector" in state:
			inspector_content.visible = state.inspector


func _save_collapse_state() -> void:
	UserSettings.set_setting(COLLAPSE_KEY, {
		"stats": stats_content.visible,
		"perf": perf_content.visible,
		"inspector": inspector_content.visible
	})


func _on_upgrades_pressed() -> void:
	var upgrade_menu = get_tree().get_first_node_in_group("upgrade_menu")
	if upgrade_menu and upgrade_menu.has_method("toggle"):
		upgrade_menu.toggle()


func _on_roster_pressed() -> void:
	var roster_panel = get_tree().get_first_node_in_group("roster_panel")
	if roster_panel and roster_panel.has_method("open"):
		roster_panel.open()


func _on_policies_pressed() -> void:
	var policies_panel = get_tree().get_first_node_in_group("policies_panel")
	if policies_panel and policies_panel.has_method("open"):
		policies_panel.open()
