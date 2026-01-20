# user_settings.gd
# User-configurable settings (persisted)
extends Node

signal settings_changed

# Display
enum HpBarMode { OFF, WHEN_DAMAGED, ALWAYS }
enum LogMode { OFF, OWN_FACTION, ALL }
enum TimestampMode { OFF, TIME, DAY_TIME }
var hp_bar_mode := HpBarMode.WHEN_DAMAGED
var death_log_mode := LogMode.OWN_FACTION
var level_log_mode := LogMode.OWN_FACTION
var spawn_log_mode := LogMode.OWN_FACTION
var food_log_mode := LogMode.OFF
var log_timestamp := TimestampMode.TIME

# Camera
var scroll_speed := 400.0

# Debug
var perf_metrics := false  # Performance profiling enabled

# Legacy property for compatibility
var show_hp_bars_always: bool:
	get: return hp_bar_mode == HpBarMode.ALWAYS


func _ready() -> void:
	load_settings()


func save_settings() -> void:
	var config := ConfigFile.new()
	config.set_value("display", "hp_bar_mode", hp_bar_mode)
	config.set_value("display", "death_log_mode", death_log_mode)
	config.set_value("display", "level_log_mode", level_log_mode)
	config.set_value("display", "spawn_log_mode", spawn_log_mode)
	config.set_value("display", "food_log_mode", food_log_mode)
	config.set_value("display", "log_timestamp", log_timestamp)
	config.set_value("camera", "scroll_speed", scroll_speed)
	config.set_value("debug", "perf_metrics", perf_metrics)
	config.save("user://settings.cfg")


func load_settings() -> void:
	var config := ConfigFile.new()
	if config.load("user://settings.cfg") == OK:
		hp_bar_mode = config.get_value("display", "hp_bar_mode", HpBarMode.WHEN_DAMAGED)
		death_log_mode = config.get_value("display", "death_log_mode", LogMode.OWN_FACTION)
		level_log_mode = config.get_value("display", "level_log_mode", LogMode.OWN_FACTION)
		spawn_log_mode = config.get_value("display", "spawn_log_mode", LogMode.OWN_FACTION)
		food_log_mode = config.get_value("display", "food_log_mode", LogMode.OFF)
		log_timestamp = config.get_value("display", "log_timestamp", TimestampMode.TIME)
		scroll_speed = config.get_value("camera", "scroll_speed", 400.0)
		perf_metrics = config.get_value("debug", "perf_metrics", false)


func set_hp_bar_mode(mode: int) -> void:
	hp_bar_mode = mode
	save_settings()
	settings_changed.emit()


func set_scroll_speed(speed: float) -> void:
	scroll_speed = speed
	save_settings()
	settings_changed.emit()


func set_death_log_mode(mode: int) -> void:
	death_log_mode = mode
	save_settings()
	settings_changed.emit()


func set_level_log_mode(mode: int) -> void:
	level_log_mode = mode
	save_settings()
	settings_changed.emit()


func set_spawn_log_mode(mode: int) -> void:
	spawn_log_mode = mode
	save_settings()
	settings_changed.emit()


func set_food_log_mode(mode: int) -> void:
	food_log_mode = mode
	save_settings()
	settings_changed.emit()


func set_log_timestamp(mode: int) -> void:
	log_timestamp = mode
	save_settings()
	settings_changed.emit()


func set_perf_metrics(enabled: bool) -> void:
	perf_metrics = enabled
	save_settings()
	settings_changed.emit()


# Generic key-value storage for UI state
var _custom: Dictionary = {}

func has_setting(key: String) -> bool:
	_load_custom()
	return _custom.has(key)


func get_setting(key: String, default: Variant = null) -> Variant:
	_load_custom()
	return _custom.get(key, default)


func set_setting(key: String, value: Variant) -> void:
	_load_custom()
	_custom[key] = value
	_save_custom()


func _load_custom() -> void:
	if not _custom.is_empty():
		return
	var config := ConfigFile.new()
	if config.load("user://settings.cfg") == OK:
		if config.has_section("custom"):
			for key in config.get_section_keys("custom"):
				_custom[key] = config.get_value("custom", key)


func _save_custom() -> void:
	var config := ConfigFile.new()
	config.load("user://settings.cfg")  # Load existing
	for key in _custom:
		config.set_value("custom", key, _custom[key])
	config.save("user://settings.cfg")
