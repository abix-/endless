extends Node2D

var npc_manager_scene: PackedScene = preload("res://systems/npc_manager.tscn")
var projectile_manager_scene: PackedScene = preload("res://systems/projectile_manager.tscn")
var player_scene: PackedScene = preload("res://entities/player.tscn")
var location_scene: PackedScene = preload("res://world/location.tscn")
var hud_scene: PackedScene = preload("res://ui/hud.tscn")
var settings_menu_scene: PackedScene = preload("res://ui/settings_menu.tscn")

var npc_manager: Node
var projectile_manager: Node
var player: Node
var hud: Node
var settings_menu: Node

var farms: Array = []
var guard_posts: Array = []
var homes: Array = []
var raider_camps: Array = []

# Village bounds (from Config)
var village_center_x: int
var village_center_y: int

func _ready() -> void:
	@warning_ignore("integer_division")
	village_center_x = (Config.VILLAGE_LEFT + Config.VILLAGE_RIGHT) / 2
	@warning_ignore("integer_division")
	village_center_y = (Config.VILLAGE_TOP + Config.VILLAGE_BOTTOM) / 2
	
	WorldClock.day_changed.connect(_on_day_changed)
	
	_create_locations()
	_setup_npc_manager()
	_setup_player()
	_setup_hud()
	_setup_settings_menu()
	_spawn_many_npcs(500)

func _create_locations() -> void:
	# Farms in center (5x4 grid = 20 farms, 5 farmers each)
	for i in range(20):
		var x = village_center_x - 200 + (i % 5) * 100
		var y = village_center_y - 100 + (i / 5) * 80
		var loc = location_scene.instantiate()
		loc.location_name = "Farm %d" % i
		loc.location_type = "field"
		loc.global_position = Vector2(x, y)
		add_child(loc)
		farms.append(loc)

	# Guard posts on border (spread around perimeter)
	var border_positions := []
	# Top edge
	for i in range(13):
		border_positions.append(Vector2(Config.VILLAGE_LEFT + i * 160, Config.VILLAGE_TOP))
	# Bottom edge
	for i in range(13):
		border_positions.append(Vector2(Config.VILLAGE_LEFT + i * 160, Config.VILLAGE_BOTTOM))
	# Left edge
	for i in range(12):
		border_positions.append(Vector2(Config.VILLAGE_LEFT, Config.VILLAGE_TOP + (i + 1) * 125))
	# Right edge
	for i in range(12):
		border_positions.append(Vector2(Config.VILLAGE_RIGHT, Config.VILLAGE_TOP + (i + 1) * 125))

	for i in range(mini(50, border_positions.size())):
		var loc = location_scene.instantiate()
		loc.location_name = "Post %d" % i
		loc.location_type = "guardpost"
		loc.global_position = border_positions[i]
		add_child(loc)
		guard_posts.append(loc)

	# Homes inside village (ring between farms and border)
	for i in range(75):
		var angle = (i / 75.0) * TAU
		var radius = randf_range(400, 600)
		var x = village_center_x + cos(angle) * radius
		var y = village_center_y + sin(angle) * radius
		var loc = location_scene.instantiate()
		loc.location_name = "Home %d" % i
		loc.location_type = "home"
		loc.global_position = Vector2(x, y)
		add_child(loc)
		homes.append(loc)

	# Raider camps outside village (one per side)
	var camp_positions := [
		Vector2(village_center_x, Config.VILLAGE_TOP - 300),      # North
		Vector2(village_center_x, Config.VILLAGE_BOTTOM + 300),   # South
		Vector2(Config.VILLAGE_LEFT - 300, village_center_y),     # West
		Vector2(Config.VILLAGE_RIGHT + 300, village_center_y),    # East
	]
	for i in range(camp_positions.size()):
		var loc = location_scene.instantiate()
		loc.location_name = "Camp %d" % i
		loc.location_type = "camp"
		loc.global_position = camp_positions[i]
		add_child(loc)
		raider_camps.append(loc)

func _setup_npc_manager() -> void:
	npc_manager = npc_manager_scene.instantiate()
	add_child(npc_manager)

	projectile_manager = projectile_manager_scene.instantiate()
	add_child(projectile_manager)

	# Connect managers
	projectile_manager.set_npc_manager(npc_manager)
	npc_manager.set_projectile_manager(projectile_manager)

	# Pass world info to npc_manager
	npc_manager.village_center = Vector2(village_center_x, village_center_y)
	for farm in farms:
		npc_manager.farm_positions.append(farm.global_position)

func _setup_player() -> void:
	player = player_scene.instantiate()
	player.global_position = Vector2(village_center_x, village_center_y)
	add_child(player)

func _setup_hud() -> void:
	hud = hud_scene.instantiate()
	add_child(hud)


func _setup_settings_menu() -> void:
	settings_menu = settings_menu_scene.instantiate()
	add_child(settings_menu)

func _spawn_many_npcs(total: int) -> void:
	var raider_count = total * 3 / 10  # 30%
	var guard_count = total / 2        # 50%
	var farmer_count = total / 5       # 20%
	
	# Farmers
	for i in range(farmer_count):
		var farm = farms[i % farms.size()]
		var home = homes[i % homes.size()]
		var home_offset = Vector2(randf_range(-20, 20), randf_range(-20, 20))
		var work_offset = Vector2(randf_range(-40, 40), randf_range(-40, 40))
		var pos = home.global_position + home_offset
		npc_manager.spawn_farmer(pos, home.global_position, farm.global_position + work_offset)
	
	# Guards
	for i in range(guard_count):
		var post = guard_posts[i % guard_posts.size()]
		var home = homes[(farmer_count + i) % homes.size()]
		var offset = Vector2(randf_range(-20, 20), randf_range(-20, 20))
		var pos = home.global_position + offset
		var night = randf() > 0.5
		npc_manager.spawn_guard(pos, home.global_position, post.global_position, night)
	
	# Raiders at camps (50 per camp, 16x16 each, need ~200x200 area)
	for i in range(raider_count):
		var camp = raider_camps[i % raider_camps.size()]
		var offset = Vector2(randf_range(-100, 100), randf_range(-100, 100))
		var pos = camp.global_position + offset
		npc_manager.spawn_raider(pos, camp.global_position)
	
	print("Spawned %d NPCs: %d farmers, %d guards, %d raiders" % [total, farmer_count, guard_count, raider_count])

func _on_day_changed(day: int) -> void:
	print("=== DAY %d ===" % day)

func _process(_delta: float) -> void:
	pass

func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed:
		match event.keycode:
			KEY_EQUAL:
				WorldClock.ticks_per_real_second *= 2.0
			KEY_MINUS:
				WorldClock.ticks_per_real_second /= 2.0
			KEY_SPACE:
				WorldClock.paused = not WorldClock.paused
