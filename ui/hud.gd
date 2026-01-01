# hud.gd
# Displays game stats: unit counts, kills, time
extends CanvasLayer

@onready var stats_grid: GridContainer = $Panel/MarginContainer/VBox/StatsGrid
@onready var time_label: Label = $Panel/MarginContainer/VBox/TimeLabel
@onready var fps_label: Label = $Panel/MarginContainer/VBox/FPSLabel
@onready var zoom_label: Label = $Panel/MarginContainer/VBox/ZoomLabel

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


func _ready() -> void:
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	player = get_tree().get_first_node_in_group("player")
	
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
	
	if Engine.get_process_frames() % 10 != 0:
		return
	
	_update_stats()
	_update_time()
	_update_fps()
	_update_zoom()


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
	var fps: int = Engine.get_frames_per_second()
	fps_label.text = "FPS: %d | Loop: %.1fms" % [fps, npc_manager.last_loop_time]


func _update_zoom() -> void:
	if player:
		var camera: Camera2D = player.get_node_or_null("Camera2D")
		if camera:
			zoom_label.text = "Zoom: %.1fx" % camera.zoom.x
