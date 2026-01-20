# combat_log.gd
# Resizable combat log panel at bottom of screen
extends CanvasLayer

@onready var panel: PanelContainer = $Panel
@onready var log_text: RichTextLabel = $Panel/VBox/Log
@onready var resize_handle: Control = $Panel/VBox/ResizeHandle

var npc_manager: Node
var main_node: Node

const SETTINGS_KEY := "combat_log"
const DEFAULT_WIDTH := 400
const DEFAULT_HEIGHT := 150
const MIN_WIDTH := 200
const MIN_HEIGHT := 80
const MAX_LOG_LINES := 50
const JOB_NAMES := ["Farmer", "Guard", "Raider"]

var _pending_messages: Array[String] = []
var _log_dirty := false

# Resize state
var _resizing := false
var _resize_start_pos := Vector2.ZERO
var _resize_start_size := Vector2.ZERO


func _ready() -> void:
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	main_node = get_parent()

	# Connect signals
	if npc_manager:
		npc_manager.npc_leveled_up.connect(_on_npc_leveled_up)
		npc_manager.npc_died.connect(_on_npc_died)
		npc_manager.npc_spawned.connect(_on_npc_spawned)
		npc_manager.npc_ate_food.connect(_on_npc_ate_food)

	# Setup resize handle
	resize_handle.gui_input.connect(_on_resize_input)

	_load_settings()


func _process(_delta: float) -> void:
	if _log_dirty:
		_flush_log()


func _input(event: InputEvent) -> void:
	if _resizing and event is InputEventMouseMotion:
		var delta: Vector2 = event.position - _resize_start_pos
		var new_width: float = maxf(MIN_WIDTH, _resize_start_size.x + delta.x)
		var new_height: float = maxf(MIN_HEIGHT, _resize_start_size.y - delta.y)
		panel.custom_minimum_size = Vector2(new_width, new_height)
		_update_position()

	if _resizing and event is InputEventMouseButton and not event.pressed:
		_resizing = false
		_save_settings()

	# Block scroll wheel when hovering over panel
	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_WHEEL_UP or event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			var rect := Rect2(panel.global_position, panel.size)
			if rect.has_point(event.position):
				get_viewport().set_input_as_handled()


func _on_resize_input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.button_index == MOUSE_BUTTON_LEFT:
		if event.pressed:
			_resizing = true
			_resize_start_pos = event.global_position
			_resize_start_size = panel.size
		else:
			_resizing = false
			_save_settings()


func _update_position() -> void:
	# Center horizontally at bottom
	var viewport_size: Vector2 = get_viewport().get_visible_rect().size
	panel.position.x = (viewport_size.x - panel.size.x) / 2
	panel.position.y = viewport_size.y - panel.size.y - 10


func _get_timestamp() -> String:
	match UserSettings.log_timestamp:
		UserSettings.TimestampMode.TIME:
			return "[%02d:%02d] " % [WorldClock.current_hour, WorldClock.current_minute]
		UserSettings.TimestampMode.DAY_TIME:
			return "[D%d %02d:%02d] " % [WorldClock.current_day, WorldClock.current_hour, WorldClock.current_minute]
	return ""


func _on_npc_leveled_up(npc_index: int, job: int, old_level: int, new_level: int) -> void:
	if UserSettings.level_log_mode == UserSettings.LogMode.OFF:
		return
	var display := _format_npc(npc_index, job)
	_pending_messages.append("%s%s Lv.%d->%d" % [_get_timestamp(), display, old_level, new_level])
	_log_dirty = true


func _on_npc_died(npc_index: int, job: int, level: int, _town_idx: int, killer_job: int, killer_level: int) -> void:
	if UserSettings.death_log_mode == UserSettings.LogMode.OFF:
		return
	var display := _format_npc(npc_index, job, level)
	var msg: String
	if killer_job >= 0:
		var killer_job_name: String = JOB_NAMES[killer_job] if killer_job < JOB_NAMES.size() else "NPC"
		msg = "%s%s killed by %s Lv.%d" % [_get_timestamp(), display, killer_job_name, killer_level]
	else:
		msg = "%s%s died" % [_get_timestamp(), display]
	_pending_messages.append(msg)
	_log_dirty = true


func _format_npc(idx: int, job: int, level: int = -1) -> String:
	var npc_name: String = npc_manager.npc_names[idx] if idx >= 0 else "NPC"
	var job_name: String = JOB_NAMES[job] if job < JOB_NAMES.size() else "NPC"
	var npc_trait: int = npc_manager.traits[idx] if idx >= 0 else 0
	var trait_name: String = NPCState.TRAIT_NAMES.get(npc_trait, "")
	var result := "%s - %s" % [npc_name, job_name]
	if level > 0:
		result += " Lv.%d" % level
	if not trait_name.is_empty():
		result += " (%s)" % trait_name
	return result


func _on_npc_spawned(npc_index: int, job: int, town_idx: int) -> void:
	if UserSettings.spawn_log_mode == UserSettings.LogMode.OFF:
		return
	if UserSettings.spawn_log_mode == UserSettings.LogMode.OWN_FACTION:
		if job == NPCState.Job.RAIDER:
			return
		if not main_node or town_idx != main_node.player_town_idx:
			return
	var display := _format_npc(npc_index, job)
	_pending_messages.append("%s%s spawned" % [_get_timestamp(), display])
	_log_dirty = true


func _on_npc_ate_food(npc_index: int, town_idx: int, job: int, hp_before: float, energy_before: float, hp_after: float) -> void:
	if UserSettings.food_log_mode == UserSettings.LogMode.OFF:
		return
	if UserSettings.food_log_mode == UserSettings.LogMode.OWN_FACTION:
		if job == NPCState.Job.RAIDER:
			return
		if not main_node or town_idx != main_node.player_town_idx:
			return
	var display := _format_npc(npc_index, job)
	_pending_messages.append("%s%s ate (HP %.0f->%.0f, E %.0f->100)" % [
		_get_timestamp(), display, hp_before, hp_after, energy_before
	])
	_log_dirty = true


func _flush_log() -> void:
	if _pending_messages.is_empty():
		_log_dirty = false
		return

	var lines := log_text.text.split("\n", false)
	lines.append_array(_pending_messages)

	if lines.size() > MAX_LOG_LINES:
		lines = lines.slice(-MAX_LOG_LINES)

	log_text.text = "\n".join(lines)
	log_text.scroll_to_line(log_text.get_line_count())

	_pending_messages.clear()
	_log_dirty = false


func _load_settings() -> void:
	var settings = UserSettings.get_setting(SETTINGS_KEY)
	if settings == null or settings.is_empty():
		panel.custom_minimum_size = Vector2(DEFAULT_WIDTH, DEFAULT_HEIGHT)
	else:
		panel.custom_minimum_size = Vector2(
			settings.get("width", DEFAULT_WIDTH),
			settings.get("height", DEFAULT_HEIGHT)
		)
	# Position after size is set
	await get_tree().process_frame
	_update_position()


func _save_settings() -> void:
	UserSettings.set_setting(SETTINGS_KEY, {
		"width": panel.size.x,
		"height": panel.size.y
	})
