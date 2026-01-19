# user_settings.gd
# User-configurable settings (persisted)
extends Node

signal settings_changed

# Display
var show_hp_bars_always := false

func _ready() -> void:
	load_settings()


func save_settings() -> void:
	var config := ConfigFile.new()
	config.set_value("display", "show_hp_bars_always", show_hp_bars_always)
	config.save("user://settings.cfg")


func load_settings() -> void:
	var config := ConfigFile.new()
	if config.load("user://settings.cfg") == OK:
		show_hp_bars_always = config.get_value("display", "show_hp_bars_always", false)


func set_show_hp_bars_always(value: bool) -> void:
	show_hp_bars_always = value
	save_settings()
	settings_changed.emit()
