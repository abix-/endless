extends Node2D

var npc_manager_scene: PackedScene = preload("res://systems/npc_manager.tscn")
var player_scene: PackedScene = preload("res://entities/player.tscn")
var location_scene: PackedScene = preload("res://world/location.tscn")

var npc_manager: Node
var player: Node

var farms: Array = []
var guard_posts: Array = []
var homes: Array = []

# Village bounds
var village_left := 500
var village_right := 2500
var village_top := 500
var village_bottom := 2000
var village_center_x: int
var village_center_y: int

func _ready() -> void:
	village_center_x = (village_left + village_right) / 2
	village_center_y = (village_top + village_bottom) / 2
	
	WorldClock.day_changed.connect(_on_day_changed)
	
	_create_locations()
	_setup_npc_manager()
	_setup_player()
	_spawn_many_npcs(2000)

func _create_locations() -> void:
	# Farms in center (grid)
	for i in range(40):
		var x = village_center_x - 400 + (i % 8) * 100
		var y = village_center_y - 250 + (i / 8) * 100
		var loc = location_scene.instantiate()
		loc.location_name = "Farm %d" % i
		loc.location_type = "field"
		loc.global_position = Vector2(x, y)
		add_child(loc)
		farms.append(loc)
	
	# Guard posts on border (spread around perimeter)
	var border_positions := []
	# Top edge
	for i in range(25):
		border_positions.append(Vector2(village_left + i * 80, village_top))
	# Bottom edge
	for i in range(25):
		border_positions.append(Vector2(village_left + i * 80, village_bottom))
	# Left edge
	for i in range(19):
		border_positions.append(Vector2(village_left, village_top + (i + 1) * 80))
	# Right edge
	for i in range(19):
		border_positions.append(Vector2(village_right, village_top + (i + 1) * 80))
	
	for i in range(mini(100, border_positions.size())):
		var loc = location_scene.instantiate()
		loc.location_name = "Post %d" % i
		loc.location_type = "guardpost"
		loc.global_position = border_positions[i]
		add_child(loc)
		guard_posts.append(loc)
	
	# Homes inside village (ring between farms and border)
	for i in range(200):
		var angle = (i / 200.0) * TAU
		var radius = randf_range(500, 700)
		var x = village_center_x + cos(angle) * radius
		var y = village_center_y + sin(angle) * radius
		var loc = location_scene.instantiate()
		loc.location_name = "Home %d" % i
		loc.location_type = "home"
		loc.global_position = Vector2(x, y)
		add_child(loc)
		homes.append(loc)

func _setup_npc_manager() -> void:
	npc_manager = npc_manager_scene.instantiate()
	add_child(npc_manager)

func _setup_player() -> void:
	player = player_scene.instantiate()
	player.global_position = Vector2(village_center_x, village_center_y)
	add_child(player)

func _spawn_many_npcs(total: int) -> void:
	var raider_count = total * 2 / 5
	var guard_count = total * 2 / 5
	var farmer_count = total / 5
	
	# Farmers
	for i in range(farmer_count):
		var farm = farms[i % farms.size()]
		var home = homes[i % homes.size()]
		var offset = Vector2(randf_range(-20, 20), randf_range(-20, 20))
		var pos = home.global_position + offset
		npc_manager.spawn_farmer(pos, home.global_position, farm.global_position)
	
	# Guards
	for i in range(guard_count):
		var post = guard_posts[i % guard_posts.size()]
		var home = homes[(farmer_count + i) % homes.size()]
		var offset = Vector2(randf_range(-20, 20), randf_range(-20, 20))
		var pos = home.global_position + offset
		var night = randf() > 0.5
		npc_manager.spawn_guard(pos, home.global_position, post.global_position, night)
	
	# Raiders outside village
	for i in range(raider_count):
		var side = randi() % 4
		var pos: Vector2
		match side:
			0:  # Above
				pos = Vector2(randf_range(village_left, village_right), randf_range(0, village_top - 100))
			1:  # Below
				pos = Vector2(randf_range(village_left, village_right), randf_range(village_bottom + 100, village_bottom + 600))
			2:  # Left
				pos = Vector2(randf_range(0, village_left - 100), randf_range(village_top, village_bottom))
			3:  # Right
				pos = Vector2(randf_range(village_right + 100, village_right + 600), randf_range(village_top, village_bottom))
		npc_manager.spawn_raider(pos)
	
	print("Spawned %d NPCs: %d farmers, %d guards, %d raiders" % [total, farmer_count, guard_count, raider_count])

func _on_day_changed(day: int) -> void:
	print("=== DAY %d ===" % day)

func _process(_delta: float) -> void:
	var time_str := "%02d:%02d" % [WorldClock.current_hour, WorldClock.current_minute]
	var period := "Day" if WorldClock.is_daytime() else "Night"
	var fps := Engine.get_frames_per_second()
	get_window().title = "Day %d - %s (%s) | NPCs: %d | FPS: %d" % [WorldClock.current_day, time_str, period, npc_manager.count, fps]

func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed:
		match event.keycode:
			KEY_EQUAL:
				WorldClock.ticks_per_real_second *= 2.0
			KEY_MINUS:
				WorldClock.ticks_per_real_second /= 2.0
			KEY_SPACE:
				WorldClock.paused = not WorldClock.paused
