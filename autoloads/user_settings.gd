# user_settings.gd
# User-configurable settings (persisted)
extends Node

signal settings_changed

# Display
enum HpBarMode { OFF, WHEN_DAMAGED, ALWAYS }
var hp_bar_mode := HpBarMode.WHEN_DAMAGED

# Camera
var scroll_speed := 400.0

# Legacy property for compatibility
var show_hp_bars_always: bool:
	get: return hp_bar_mode == HpBarMode.ALWAYS


func _ready() -> void:
	load_settings()


func save_settings() -> void:
	var config := ConfigFile.new()
	config.set_value("display", "hp_bar_mode", hp_bar_mode)
	config.set_value("camera", "scroll_speed", scroll_speed)
	config.save("user://settings.cfg")


func load_settings() -> void:
	var config := ConfigFile.new()
	if config.load("user://settings.cfg") == OK:
		hp_bar_mode = config.get_value("display", "hp_bar_mode", HpBarMode.WHEN_DAMAGED)
		scroll_speed = config.get_value("camera", "scroll_speed", 400.0)


func set_hp_bar_mode(mode: int) -> void:
	hp_bar_mode = mode
	save_settings()
	settings_changed.emit()


func set_scroll_speed(speed: float) -> void:
	scroll_speed = speed
	save_settings()
	settings_changed.emit()
