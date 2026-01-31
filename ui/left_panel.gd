# left_panel.gd
# Unified left panel with collapsible sections: Stats, Performance, Inspector
# ECS-only: calls EcsNpcManager methods directly
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
@onready var log_scroll: ScrollContainer = $Panel/MarginContainer/VBox/InspectorContent/LogScroll
@onready var log_label: Label = $Panel/MarginContainer/VBox/InspectorContent/LogScroll/LogLabel
@onready var follow_btn: Button = $Panel/MarginContainer/VBox/InspectorContent/ButtonRow/FollowBtn
@onready var copy_btn: Button = $Panel/MarginContainer/VBox/InspectorContent/ButtonRow/CopyBtn

# Grid cells for population counts
var farmer_alive: Label
var farmer_dead: Label
var farmer_kills: Label
var guard_alive: Label
var guard_dead: Label
var guard_kills: Label
var raider_alive: Label
var raider_dead: Label
var raider_kills: Label

var npc_manager: Node  # EcsNpcManager
var main_node: Node
var player: Node

# State
var last_idx := -1
var following := false
var dragging := false
var drag_offset := Vector2.ZERO

const SETTINGS_KEY := "left_panel_pos"
const COLLAPSE_KEY := "left_panel_collapse"


func _ready() -> void:
	add_to_group("left_panel")
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	player = get_tree().get_first_node_in_group("player")
	main_node = get_parent()

	# Connect headers
	stats_header.pressed.connect(_toggle_section.bind("stats"))
	perf_header.pressed.connect(_toggle_section.bind("perf"))
	inspector_header.pressed.connect(_toggle_section.bind("inspector"))

	# Connect buttons
	perf_toggle.toggled.connect(_on_perf_toggled)
	perf_copy.pressed.connect(_on_perf_copy_pressed)
	radius_toggle.toggled.connect(_on_radius_toggled)
	follow_btn.toggled.connect(_on_follow_toggled)
	copy_btn.pressed.connect(_on_copy_pressed)
	rename_btn.pressed.connect(_on_rename_pressed)
	name_edit.text_submitted.connect(_on_name_submitted)
	name_edit.focus_exited.connect(_on_name_focus_lost)
	upgrades_btn.pressed.connect(_on_upgrades_pressed)
	roster_btn.pressed.connect(_on_roster_pressed)
	policies_btn.pressed.connect(_on_policies_pressed)

	# Hide GDScript-only toggles (ECS handles parallel/GPU internally)
	parallel_toggle.visible = false
	gpu_toggle.visible = false

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


func _input(event: InputEvent) -> void:
	var rect := Rect2(panel.position, panel.size)
	var mouse_over := rect.has_point(get_viewport().get_mouse_position())

	if event is InputEventMouseButton:
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
		var npc_count: int = npc_manager.get_npc_count()
		if last_idx < npc_count and npc_manager.get_npc_health(last_idx) > 0:
			var npc_pos: Vector2 = npc_manager.get_npc_position(last_idx)
			if player:
				player.global_position = npc_pos
		else:
			_set_following(false)

	if Engine.get_process_frames() % 10 != 0:
		return

	_update_stats()
	_update_perf()
	_update_inspector()


func _update_stats() -> void:
	if not stats_content.visible:
		return

	# Population stats from ECS
	var pop: Dictionary = npc_manager.get_population_stats()
	farmer_alive.text = str(pop.get("farmers_alive", 0))
	farmer_kills.text = "-"  # Farmers don't kill
	guard_alive.text = str(pop.get("guards_alive", 0))
	guard_kills.text = str(pop.get("guard_kills", 0))
	raider_alive.text = str(pop.get("raiders_alive", 0))
	raider_kills.text = str(pop.get("villager_kills", 0))

	# Per-faction stats (alive/dead/kills)
	var faction_stats: Array = npc_manager.get_all_faction_stats()
	if faction_stats.size() > 0:
		# Faction 0 = villagers (farmers + guards)
		var villager_stats: Dictionary = faction_stats[0] if faction_stats.size() > 0 else {}
		farmer_dead.text = str(villager_stats.get("dead", 0))
		guard_dead.text = "-"  # Can't separate farmer/guard deaths yet

		# Aggregate all raider factions (1..N)
		var raider_dead_total := 0
		var raider_kills_total := 0
		for i in range(1, faction_stats.size()):
			var s: Dictionary = faction_stats[i]
			raider_dead_total += s.get("dead", 0)
			raider_kills_total += s.get("kills", 0)
		raider_dead.text = str(raider_dead_total)
		# Update raider kills with faction-tracked value
		if raider_kills_total > 0:
			raider_kills.text = str(raider_kills_total)
	else:
		farmer_dead.text = "-"
		guard_dead.text = "-"
		raider_dead.text = "-"

	# Time (ECS GameTime resource)
	var game_time: Dictionary = npc_manager.get_game_time()
	var period := "Day" if game_time.get("is_daytime", true) else "Night"
	time_label.text = "Day %d - %02d:%02d (%s)" % [
		game_time.get("day", 1),
		game_time.get("hour", 6),
		game_time.get("minute", 0),
		period
	]

	# Food - unified town model: raider towns start at index towns.size()
	var town_total: int = npc_manager.get_town_food(0)
	var raider_town_idx: int = main_node.towns.size() if main_node and "towns" in main_node else 1
	var camp_total: int = npc_manager.get_town_food(raider_town_idx)
	food_label.text = "Food: %d vs %d" % [town_total, camp_total]

	# Bed stats from ECS
	var player_town: int = 0
	if main_node and "player_town_idx" in main_node:
		player_town = main_node.player_town_idx
	var beds: Dictionary = npc_manager.get_bed_stats(player_town)
	var total_beds: int = beds.get("total_beds", 0)
	var free_beds: int = beds.get("free_beds", 0)
	bed_label.text = "Beds: %d used, %d free" % [total_beds - free_beds, free_beds]


func _update_perf() -> void:
	if not perf_content.visible:
		return

	var lines: PackedStringArray = []

	# FPS and zoom (works)
	var fps := int(Engine.get_frames_per_second())
	var zoom_str := "?"
	if player:
		var camera: Camera2D = player.get_node_or_null("Camera2D")
		if camera:
			zoom_str = "%.1fx" % camera.zoom.x
	lines.append("FPS: %d | Zoom: %s" % [fps, zoom_str])

	# Population counts (alive NPCs)
	var pop: Dictionary = npc_manager.get_population_stats()
	var alive: int = pop.get("farmers_alive", 0) + pop.get("guards_alive", 0) + pop.get("raiders_alive", 0)
	lines.append("Alive: %d (F:%d G:%d R:%d)" % [alive, pop.get("farmers_alive", 0), pop.get("guards_alive", 0), pop.get("raiders_alive", 0)])

	# ECS debug stats
	var stats: Dictionary = npc_manager.get_debug_stats()
	lines.append("Arrived: %d | Cells: %d" % [stats.get("arrived_count", 0), stats.get("cells_used", 0)])
	lines.append("Backoff: avg=%d max=%d" % [stats.get("avg_backoff", 0), stats.get("max_backoff", 0)])

	# Combat debug (works)
	if UserSettings.perf_metrics:
		var combat: Dictionary = npc_manager.get_combat_debug()
		lines.append("")
		lines.append("Combat: %d attackers, %d targets" % [combat.get("attackers", 0), combat.get("targets_found", 0)])
		lines.append("Attacks: %d | Chases: %d" % [combat.get("attacks", 0), combat.get("chases", 0)])

	# Rust ECS perf stats
	if UserSettings.perf_metrics:
		var perf: Dictionary = npc_manager.get_perf_stats()
		var frame_ms: float = perf.get("frame_ms", 0.0)
		var ecs_ms: float = perf.get("ecs_total_ms", 0.0)
		var godot_ms: float = perf.get("godot_ms", 0.0)
		lines.append("")
		lines.append("Frame: %.1fms (ECS: %.1f + Godot: %.1f)" % [frame_ms, ecs_ms, godot_ms])
		lines.append("  Bevy ECS: %.2f" % perf.get("bevy_ms", 0.0))
		lines.append("  GPU:      %.2f (Q:%.1f D:%.1f R:%.1f)" % [perf.get("gpu_total_ms", 0.0), perf.get("queue_ms", 0.0), perf.get("dispatch_ms", 0.0), perf.get("readpos_ms", 0.0)])
		lines.append("  Render:   %.2f (B:%.1f U:%.1f)" % [perf.get("build_ms", 0.0) + perf.get("upload_ms", 0.0), perf.get("build_ms", 0.0), perf.get("upload_ms", 0.0)])
		lines.append("  Godot:    %.2f" % godot_ms)

	perf_label.text = "\n".join(lines)


func _update_inspector() -> void:
	inspector_header.text = "▼ Inspector" if inspector_content.visible else "▶ Inspector"

	if not inspector_content.visible:
		return

	# Get selected NPC or use last
	var idx: int = npc_manager.get_selected_npc()
	if idx < 0:
		idx = last_idx
	if idx < 0 or idx >= npc_manager.get_npc_count() or npc_manager.get_npc_health(idx) <= 0.0:
		job_level.text = "Click NPC to inspect"
		town_label.visible = false
		health_bar.get_parent().visible = false
		energy_bar.get_parent().visible = false
		xp_label.visible = false
		state_label.visible = false
		target_label.visible = false
		stats_label.visible = false
		log_scroll.visible = false
		copy_btn.visible = false
		rename_btn.visible = false
		return

	last_idx = idx

	# Get NPC info from ECS
	var info: Dictionary = npc_manager.get_npc_info(idx)

	var job: String = info.get("job", "?")
	var level: int = info.get("level", 1)
	var npc_name: String = info.get("name", "NPC")
	var npc_trait: String = info.get("trait", "")

	job_level.text = "%s - %s Lv.%d" % [npc_name, job, level]
	if not npc_trait.is_empty():
		job_level.text += " (%s)" % npc_trait

	# Town
	var npc_town_idx: int = info.get("town_id", -1)
	if npc_town_idx >= 0 and main_node and "towns" in main_node and npc_town_idx < main_node.towns.size():
		town_label.text = main_node.towns[npc_town_idx].name
		town_label.visible = true
	else:
		town_label.visible = false

	# Health
	var hp: float = info.get("hp", 0.0)
	var max_hp: float = info.get("max_hp", 100.0)
	health_bar.get_parent().visible = true
	health_bar.value = hp / max_hp if max_hp > 0 else 0.0
	health_value.text = "%d/%d" % [int(hp), int(max_hp)]

	# Energy
	var energy: float = info.get("energy", 100.0)
	energy_bar.get_parent().visible = true
	energy_bar.value = energy / Config.ENERGY_MAX
	energy_value.text = "%d" % int(energy)

	# XP
	var xp: int = info.get("xp", 0)
	xp_label.visible = true
	xp_label.text = "XP: %d" % xp

	# State
	var state: String = info.get("state", "?")
	state_label.visible = true
	state_label.text = state

	# Target
	var target_npc: int = info.get("target_idx", -1)
	target_label.visible = true
	if target_npc >= 0 and target_npc < npc_manager.get_npc_count():
		var target_info: Dictionary = npc_manager.get_npc_info(target_npc)
		var t_job: String = target_info.get("job", "?")
		target_label.text = "Target: %s #%d" % [t_job, target_npc]
	else:
		target_label.text = "Target: -"

	# Stats (position)
	var pos_x: float = info.get("x", 0.0)
	var pos_y: float = info.get("y", 0.0)
	stats_label.visible = true
	stats_label.text = "Pos: %d, %d" % [int(pos_x), int(pos_y)]

	# Decision log (scrollable, all entries)
	var logs: Array = npc_manager.get_npc_log(idx, 100)
	if logs.size() > 0:
		var log_lines: PackedStringArray = []
		for entry in logs:
			log_lines.append("%02d:%02d %s" % [entry.get("hour", 0), entry.get("minute", 0), entry.get("message", "")])
		log_label.text = "\n".join(log_lines)
		log_scroll.visible = true
	else:
		log_scroll.visible = false

	# Show buttons
	copy_btn.visible = true
	rename_btn.visible = true


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
	if main_node:
		main_node.queue_redraw()


func _update_radius_toggle() -> void:
	radius_toggle.button_pressed = UserSettings.show_active_radius


func _on_perf_copy_pressed() -> void:
	DisplayServer.clipboard_set(perf_label.text)
	perf_copy.text = "Copied!"
	await get_tree().create_timer(1.0).timeout
	perf_copy.text = "Copy"


func _on_copy_pressed() -> void:
	# === ECS API NEEDED: per-NPC data for export ===
	pass


func _on_follow_toggled(enabled: bool) -> void:
	_set_following(enabled)


func _set_following(enabled: bool) -> void:
	following = enabled
	follow_btn.button_pressed = enabled
	follow_btn.text = "Following" if enabled else "Follow"


func _on_rename_pressed() -> void:
	if last_idx >= 0:
		name_edit.text = npc_manager.get_npc_name(last_idx)
		job_level.visible = false
		name_edit.visible = true
		name_edit.grab_focus()


func _on_name_submitted(new_name: String) -> void:
	if last_idx >= 0 and not new_name.is_empty():
		npc_manager.set_npc_name(last_idx, new_name)
	_close_name_edit()


func _on_name_focus_lost() -> void:
	_close_name_edit()


func _close_name_edit() -> void:
	name_edit.visible = false
	job_level.visible = true


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
