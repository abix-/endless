# hud.gd
# Displays game stats: unit counts, kills, time
extends CanvasLayer

@onready var stats_grid: GridContainer = $Panel/MarginContainer/VBox/StatsGrid
@onready var time_label: Label = $Panel/MarginContainer/VBox/TimeLabel
@onready var fps_label: Label = $Panel/MarginContainer/VBox/FPSLabel
@onready var zoom_label: Label = $Panel/MarginContainer/VBox/ZoomLabel
@onready var food_label: Label = $Panel/MarginContainer/VBox/FoodLabel
@onready var combat_log: RichTextLabel = $CombatLog

const MAX_LOG_LINES := 20
const JOB_NAMES := ["Farmer", "Guard", "Raider"]

# Batch level-up messages to avoid per-frame string operations
var _pending_levelups: Array[String] = []
var _log_dirty := false

# Grid cells (set in _ready after grid is populated)
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
var player: Node
var main_node: Node


func _ready() -> void:
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	player = get_tree().get_first_node_in_group("player")
	main_node = get_parent()

	if npc_manager:
		npc_manager.npc_leveled_up.connect(_on_npc_leveled_up)

	# Get grid cell references (row by row, skipping headers)
	var cells := stats_grid.get_children()
	# Row 0: headers (4 cells: blank, Alive, Dead, Kills)
	# Row 1: Farmer
	farmer_alive = cells[5]
	farmer_dead = cells[6]
	farmer_kills = cells[7]
	# Row 2: Guard
	guard_alive = cells[9]
	guard_dead = cells[10]
	guard_kills = cells[11]
	# Row 3: Raider
	raider_alive = cells[13]
	raider_dead = cells[14]
	raider_kills = cells[15]


func _process(_delta: float) -> void:
	if not npc_manager:
		return

	# Flush pending level-up messages (batched)
	if _log_dirty:
		_flush_combat_log()

	if Engine.get_process_frames() % 10 != 0:
		return

	_update_stats()
	_update_time()
	_update_fps()
	_update_zoom()
	_update_food()


func _update_stats() -> void:
	farmer_alive.text = str(npc_manager.alive_farmers)
	farmer_dead.text = str(npc_manager.dead_farmers)
	farmer_kills.text = "-"
	
	guard_alive.text = str(npc_manager.alive_guards)
	guard_dead.text = str(npc_manager.dead_guards)
	guard_kills.text = str(npc_manager.raider_kills)
	
	raider_alive.text = str(npc_manager.alive_raiders)
	raider_dead.text = str(npc_manager.dead_raiders)
	raider_kills.text = str(npc_manager.villager_kills)


func _update_time() -> void:
	var period := "Day" if WorldClock.is_daytime() else "Night"
	time_label.text = "Day %d - %02d:%02d (%s)" % [
		WorldClock.current_day,
		WorldClock.current_hour,
		WorldClock.current_minute,
		period
	]


func _update_fps() -> void:
	var fps: int = int(Engine.get_frames_per_second())
	fps_label.text = "FPS: %d | Loop: %.1fms" % [fps, npc_manager.last_loop_time]


func _update_zoom() -> void:
	if player:
		var camera: Camera2D = player.get_node_or_null("Camera2D")
		if camera:
			zoom_label.text = "Zoom: %.1fx" % camera.zoom.x


func _update_food() -> void:
	if not main_node or not "town_food" in main_node:
		return
	if not "towns" in main_node or main_node.towns.is_empty():
		return

	var lines: Array[String] = []
	var town_total := 0
	var camp_total := 0

	for i in main_node.towns.size():
		var town_name: String = main_node.towns[i].name
		var tf: int = main_node.town_food[i]
		var cf: int = main_node.camp_food[i]
		town_total += tf
		camp_total += cf
		lines.append("%s: %d | Raiders: %d" % [town_name, tf, cf])

	food_label.text = "Food (%d vs %d):\n%s" % [town_total, camp_total, "\n".join(lines)]


func _on_npc_leveled_up(_npc_index: int, job: int, old_level: int, new_level: int) -> void:
	var job_name: String = JOB_NAMES[job] if job < JOB_NAMES.size() else "NPC"
	_pending_levelups.append("%s %d → %d" % [job_name, old_level, new_level])
	_log_dirty = true


func _flush_combat_log() -> void:
	if _pending_levelups.is_empty():
		_log_dirty = false
		return

	# Build new text efficiently
	var lines := combat_log.text.split("\n", false)
	lines.append_array(_pending_levelups)

	# Keep only last MAX_LOG_LINES
	if lines.size() > MAX_LOG_LINES:
		lines = lines.slice(-MAX_LOG_LINES)

	combat_log.text = "\n".join(lines)
	combat_log.scroll_to_line(combat_log.get_line_count())

	_pending_levelups.clear()
	_log_dirty = false
