# hud.gd
# Displays game stats: unit counts, kills, time
extends CanvasLayer

@onready var farmers_label: Label = $Panel/MarginContainer/VBox/FarmersLabel
@onready var guards_label: Label = $Panel/MarginContainer/VBox/GuardsLabel
@onready var raiders_label: Label = $Panel/MarginContainer/VBox/RaidersLabel
@onready var kills_label: Label = $Panel/MarginContainer/VBox/KillsLabel
@onready var time_label: Label = $Panel/MarginContainer/VBox/TimeLabel
@onready var fps_label: Label = $Panel/MarginContainer/VBox/FPSLabel

var npc_manager: Node


func _ready() -> void:
	# Find npc_manager in tree (set by main.gd)
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")


func _process(_delta: float) -> void:
	if not npc_manager:
		return
	
	# Update every 10 frames for performance
	if Engine.get_process_frames() % 10 != 0:
		return
	
	_update_unit_counts()
	_update_kills()
	_update_time()
	_update_fps()


func _update_unit_counts() -> void:
	farmers_label.text = "Farmers: %d / %d" % [npc_manager.alive_farmers, npc_manager.total_farmers]
	guards_label.text = "Guards: %d / %d" % [npc_manager.alive_guards, npc_manager.total_guards]
	raiders_label.text = "Raiders: %d / %d" % [npc_manager.alive_raiders, npc_manager.total_raiders]


func _update_kills() -> void:
	kills_label.text = "Kills - V: %d  R: %d" % [npc_manager.villager_kills, npc_manager.raider_kills]


func _update_time() -> void:
	var period := "Day" if WorldClock.is_daytime() else "Night"
	time_label.text = "Day %d - %02d:%02d (%s)" % [
		WorldClock.current_day,
		WorldClock.current_hour,
		WorldClock.current_minute,
		period
	]


func _update_fps() -> void:
	var fps: int = Engine.get_frames_per_second()
	fps_label.text = "FPS: %d | Loop: %.1fms" % [fps, npc_manager.last_loop_time]
