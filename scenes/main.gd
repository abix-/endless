extends Node2D

#var npc_manager_scene: PackedScene = preload("res://systems/npc_manager.tscn")
#var projectile_manager_scene: PackedScene = preload("res://systems/projectile_manager.tscn")
var player_scene: PackedScene = preload("res://entities/player.tscn")
var location_scene: PackedScene = preload("res://world/location.tscn")
var terrain_scene: PackedScene = preload("res://world/terrain_renderer.tscn")
var left_panel_scene: PackedScene = preload("res://ui/left_panel.tscn")
var settings_menu_scene: PackedScene = preload("res://ui/settings_menu.tscn")
var upgrade_menu_scene: PackedScene = preload("res://ui/upgrade_menu.tscn")
var combat_log_scene: PackedScene = preload("res://ui/combat_log.tscn")
var roster_panel_scene: PackedScene = preload("res://ui/roster_panel.tscn")
var build_menu_scene: PackedScene = preload("res://ui/build_menu.tscn")
var policies_panel_scene: PackedScene = preload("res://ui/policies_panel.tscn")
var guard_post_menu_scene: PackedScene = preload("res://ui/guard_post_menu.tscn")

var npc_manager  # EcsNpcManager (Rust)
var _uses_ecs := false  # True for EcsNpcManager
#var projectile_manager: Node  # ECS handles projectiles
var player: Node
var left_panel: Node
var settings_menu: Node
var upgrade_menu: Node
var build_menu: Node
var guard_post_menu: Node
var farm_menu: Node
var terrain_renderer: Node

# Currently selected terrain tile (for inspector)
var selected_tile: Dictionary = {}

# World data
var towns: Array = []  # Array of {center, grid, slots, guard_posts, camp}
var town_food: PackedInt32Array  # Food stored in each town
var camp_food: PackedInt32Array  # Food stored in each raider camp
var spawn_timers: PackedInt32Array  # Hours since last spawn per town
var player_town_idx: int = 0  # First town is player-controlled
var town_upgrades: Array = []  # Per-town upgrade levels
var town_max_farmers: PackedInt32Array  # Population cap per town
var town_max_guards: PackedInt32Array   # Population cap per town
var town_policies: Array = []  # Per-town faction policies
var guard_post_upgrades: Array[Dictionary] = []  # Per-town: slot_key -> {attack_enabled, range_level, damage_level}

# Grid slot keys - max 100x100 grid (-49 to +50)
# Town starts at 6x6 (-2 to +3), expands by unlocking adjacent slots
const BASE_GRID_MIN := -2
const BASE_GRID_MAX := 3
const MAX_GRID_MIN := -49
const MAX_GRID_MAX := 50

# Fixed slots: center area (0,0), (0,1), farms at (0,-1), (1,-1)
const FIXED_SLOTS := ["0,0", "0,1", "0,-1", "1,-1"]


# Get grid bounds for base level (6x6)
func _get_grid_bounds(_level: int) -> Dictionary:
	return {
		"min": BASE_GRID_MIN,
		"max": BASE_GRID_MAX
	}


# Get all grid keys for current town grid level
func _get_grid_keys_for_level(level: int) -> Array:
	var bounds := _get_grid_bounds(level)
	var keys: Array = []
	for row in range(bounds.min, bounds.max + 1):
		for col in range(bounds.min, bounds.max + 1):
			keys.append("%d,%d" % [row, col])
	return keys


# Get corner keys for guard posts at current grid level
func _get_corner_keys(level: int) -> Array:
	var bounds := _get_grid_bounds(level)
	# Clockwise order so guards patrol the perimeter, not diagonals
	return [
		"%d,%d" % [bounds.min, bounds.min],
		"%d,%d" % [bounds.min, bounds.max],
		"%d,%d" % [bounds.max, bounds.max],
		"%d,%d" % [bounds.max, bounds.min]
	]


var NUM_TOWNS: int  # Set from Config in _ready
const MIN_TOWN_DISTANCE := 1200  # Minimum distance between town centers
const FOOD_PER_WORK_HOUR := 1  # Food generated per farmer per work hour

const TOWN_NAMES := [
	"Miami", "Orlando", "Tampa", "Jacksonville", "Tallahassee",
	"Gainesville", "Pensacola", "Sarasota", "Naples", "Daytona",
	"Lakeland", "Ocala", "Boca Raton", "Key West", "Fort Myers"
]


func _ready() -> void:
	NUM_TOWNS = Config.num_towns
	_generate_world()
	_setup_terrain()
	_setup_managers()
	_setup_player()
	_setup_ui()
	_spawn_npcs()


func _draw() -> void:
	# World border
	var border_color := Color(0.4, 0.4, 0.4, 0.8)
	var border_width := 4.0
	var rect := Rect2(0, 0, Config.world_width, Config.world_height)
	draw_rect(rect, border_color, false, border_width)

	# Corner markers for visibility
	var marker_size := 50.0
	var corners := [
		Vector2(0, 0),
		Vector2(Config.world_width, 0),
		Vector2(Config.world_width, Config.world_height),
		Vector2(0, Config.world_height)
	]
	for corner in corners:
		draw_circle(corner, marker_size, border_color)

	# Player town indicator - gold ring expands with building range
	if towns.size() > player_town_idx:
		var town: Dictionary = towns[player_town_idx]
		var town_center: Vector2 = town.center
		var grid: Dictionary = town.grid
		var gold := Color(1.0, 0.85, 0.3, 0.8)
		# Calculate radius based on farthest unlocked slot
		var max_dist := 60.0  # Minimum radius
		for slot_key in town.slots.keys():
			if slot_key in grid:
				var slot_pos: Vector2 = grid[slot_key]
				var dist: float = town_center.distance_to(slot_pos) + Config.TOWN_GRID_SPACING
				if dist > max_dist:
					max_dist = dist
		draw_arc(town_center, max_dist, 0, TAU, 64, gold, 3.0)

	# Draw buildable slot indicators for player's town
	_draw_buildable_slots()

	# Draw selected NPC's target visualization
	_draw_selected_npc_target()

	# Debug: Active radius circle (entity sleeping zone) — needs ECS API
	#if UserSettings.show_active_radius:
	#	var camera: Camera2D = get_viewport().get_camera_2d()
	#	if camera:
	#		var cam_pos: Vector2 = camera.global_position
	#		var radius: float = npc_manager.ACTIVE_RADIUS
	#		var color := Color(0.2, 0.8, 1.0, 0.4)
	#		draw_arc(cam_pos, radius, 0, TAU, 64, color, 2.0)


func _generate_world() -> void:
	# Initialize food and spawn arrays
	town_food.resize(NUM_TOWNS)
	camp_food.resize(NUM_TOWNS)
	spawn_timers.resize(NUM_TOWNS)
	town_max_farmers.resize(NUM_TOWNS)
	town_max_guards.resize(NUM_TOWNS)
	for i in NUM_TOWNS:
		town_food[i] = 0
		camp_food[i] = 0
		spawn_timers[i] = 0
		town_max_farmers[i] = Config.max_farmers_per_town
		town_max_guards[i] = Config.max_guards_per_town

	# Initialize town upgrades
	for i in NUM_TOWNS:
		town_upgrades.append({
			"guard_health": 0,
			"guard_attack": 0,
			"guard_range": 0,
			"guard_size": 0,
			"guard_attack_speed": 0,
			"guard_move_speed": 0,
			"farm_yield": 0,
			"farmer_hp": 0,
			"healing_rate": 0,
			"alert_radius": 0,
			"food_efficiency": 0,
			"farmer_cap": 0,
			"guard_cap": 0,
			"fountain_radius": 0
		})

	# Initialize town policies
	for i in NUM_TOWNS:
		town_policies.append({
			"eat_food": true,
			"farmer_flee_hp": 1.0,        # 100% = always flee
			"guard_flee_hp": 0.33,        # 33% default
			"recovery_hp": 0.75,          # 75% before resuming
			"guard_aggressive": true,     # Chase enemies vs defensive
			"guard_leash": false,         # Stay near posts vs chase anywhere
			"farmer_fight_back": false,   # Melee attack vs always flee
			"prioritize_healing": false,  # Stay at fountain until full HP
			"work_schedule": 0,           # 0=both shifts, 1=day only, 2=night only
			"farmer_off_duty": 0,         # 0=bed, 1=fountain, 2=wander town
			"guard_off_duty": 0           # 0=bed, 1=fountain, 2=wander town
		})

	# Initialize guard post upgrades (empty dict per town, filled when posts created)
	for i in NUM_TOWNS:
		guard_post_upgrades.append({})

	# Generate scattered town positions
	var town_positions: Array[Vector2] = []
	var attempts := 0
	var max_attempts := 1000

	while town_positions.size() < NUM_TOWNS and attempts < max_attempts:
		attempts += 1
		var pos := Vector2(
			randf_range(Config.WORLD_MARGIN, Config.world_width - Config.WORLD_MARGIN),
			randf_range(Config.WORLD_MARGIN, Config.world_height - Config.WORLD_MARGIN)
		)

		# Check distance from existing towns
		var valid := true
		for existing in town_positions:
			if pos.distance_to(existing) < MIN_TOWN_DISTANCE:
				valid = false
				break

		if valid:
			town_positions.append(pos)

	# Shuffle town names for variety
	var available_names := TOWN_NAMES.duplicate()
	available_names.shuffle()

	# Create each town with its structures
	for i in town_positions.size():
		var town_center: Vector2 = town_positions[i]
		var town_name: String = available_names[i % available_names.size()]
		var grid := _calculate_grid_positions(town_center)

		# Initialize slots for base grid size (6x6)
		var slots := {}
		var base_keys := _get_grid_keys_for_level(0)
		for key in base_keys:
			slots[key] = []

		var town_data := {
			"name": town_name,
			"center": town_center,
			"grid": grid,
			"slots": slots,
			"guard_posts": [],
			"camp": null
		}

		# Create fountain at center (0,0)
		var fountain = location_scene.instantiate()
		fountain.location_name = town_name
		fountain.location_type = "fountain"
		fountain.global_position = grid["0,0"]
		add_child(fountain)
		town_data.slots["0,0"].append({"type": "fountain", "node": fountain})

		# Create farm at west slot (0,-1)
		var farm_w = location_scene.instantiate()
		farm_w.location_name = "%s Farm W" % town_name
		farm_w.location_type = "field"
		farm_w.global_position = grid["0,-1"]
		add_child(farm_w)
		town_data.slots["0,-1"].append({"type": "farm", "node": farm_w})

		# Create farm at east slot (0,1)
		var farm_e = location_scene.instantiate()
		farm_e.location_name = "%s Farm E" % town_name
		farm_e.location_type = "field"
		farm_e.global_position = grid["0,1"]
		add_child(farm_e)
		town_data.slots["0,1"].append({"type": "farm", "node": farm_e})

		# Create initial beds in 4 inner corner slots (4 beds each = 16 total)
		var bed_slots := ["-1,-1", "-1,2", "2,-1", "2,2"]
		for slot_key in bed_slots:
			for bed_idx in 4:
				var bed_offset := Vector2(
					(bed_idx % 2 - 0.5) * 16,
					(floorf(bed_idx / 2.0) - 0.5) * 16
				)
				var bed = location_scene.instantiate()
				bed.location_name = "%s Bed" % town_name
				bed.location_type = "home"
				bed.global_position = grid[slot_key] + bed_offset
				add_child(bed)
				town_data.slots[slot_key].append({"type": "bed", "node": bed})

		# Create guard posts at corners of initial grid
		var corner_keys := _get_corner_keys(0)
		for corner_key in corner_keys:
			var post = location_scene.instantiate()
			post.location_name = "%s Post" % town_name
			post.location_type = "guard_post"
			post.global_position = grid[corner_key]
			add_child(post)
			town_data.guard_posts.append(post)
			town_data.slots[corner_key].append({"type": "guard_post", "node": post})
			# Initialize guard post upgrades
			guard_post_upgrades[i][corner_key] = {
				"attack_enabled": false,
				"range_level": 0,
				"damage_level": 0
			}

		# Create raider camp (away from all towns, in direction with most room)
		var camp_pos := _find_camp_position(town_center, town_positions)

		var camp = location_scene.instantiate()
		camp.location_name = "%s Raiders" % town_name
		camp.location_type = "camp"
		camp.global_position = camp_pos
		add_child(camp)
		town_data.camp = camp

		towns.append(town_data)

	print("Generated %d towns" % towns.size())


func _setup_terrain() -> void:
	terrain_renderer = terrain_scene.instantiate()
	add_child(terrain_renderer)
	move_child(terrain_renderer, 0)  # Render behind everything

	# Collect town and camp positions for biome generation
	var town_centers: Array[Vector2] = []
	var camp_centers: Array[Vector2] = []
	for town in towns:
		town_centers.append(town.center)
		if town.camp:
			camp_centers.append(town.camp.global_position)

	terrain_renderer.generate(town_centers, camp_centers)


func _setup_managers() -> void:
	# EcsNpcManager (Rust) replaces GDScript npc_manager + projectile_manager
	npc_manager = ClassDB.instantiate("EcsNpcManager")
	add_child(npc_manager)
	npc_manager.add_to_group("npc_manager")
	_uses_ecs = npc_manager.has_method("get_npc_count")

	# Wire world data into ECS
	# Unified town model: villager towns (0..N-1) + raider towns (N..2N-1)
	var total_towns: int = NUM_TOWNS * 2
	npc_manager.init_world(total_towns)
	npc_manager.init_food_storage(total_towns)

	# Add villager towns (faction=0) with their buildings
	for town_idx in towns.size():
		var town: Dictionary = towns[town_idx]
		npc_manager.add_town(town.name, town.center.x, town.center.y, 0)  # faction=Villager

		# Add farms
		var farms := _get_farms_from_town(town)
		for farm in farms:
			npc_manager.add_farm(farm.global_position.x, farm.global_position.y, town_idx)

		# Add beds
		var beds := _get_beds_from_town(town)
		for bed in beds:
			npc_manager.add_bed(bed.global_position.x, bed.global_position.y, town_idx)

		# Add guard posts (patrol_order = index within town)
		for post_idx in town.guard_posts.size():
			var post = town.guard_posts[post_idx]
			npc_manager.add_guard_post(post.global_position.x, post.global_position.y, town_idx, post_idx)

	# Add raider towns (faction=1) - what were previously "camps"
	for town_idx in towns.size():
		var town: Dictionary = towns[town_idx]
		if town.camp:
			var camp_pos: Vector2 = town.camp.global_position
			npc_manager.add_town("Raider Camp %d" % town_idx, camp_pos.x, camp_pos.y, 1)  # faction=Raider


func _setup_player() -> void:
	player = player_scene.instantiate()
	# Center on player's town
	player.global_position = towns[player_town_idx].center
	add_child(player)


func _setup_ui() -> void:
	left_panel = left_panel_scene.instantiate()
	add_child(left_panel)

	settings_menu = settings_menu_scene.instantiate()
	add_child(settings_menu)

	upgrade_menu = upgrade_menu_scene.instantiate()
	#upgrade_menu.upgrade_purchased.connect(_on_upgrade_purchased)  # Phase 6
	add_child(upgrade_menu)

	var combat_log = combat_log_scene.instantiate()
	add_child(combat_log)

	var roster_panel = roster_panel_scene.instantiate()
	add_child(roster_panel)

	build_menu = build_menu_scene.instantiate()
	#build_menu.build_requested.connect(_on_build_requested)  # Phase 5
	#build_menu.destroy_requested.connect(_on_destroy_requested)  # Phase 5
	#build_menu.unlock_requested.connect(_on_unlock_requested)  # Phase 5
	add_child(build_menu)

	var policies_panel = policies_panel_scene.instantiate()
	add_child(policies_panel)

	guard_post_menu = guard_post_menu_scene.instantiate()
	add_child(guard_post_menu)

	var farm_menu_script: GDScript = preload("res://ui/farm_menu.gd")
	farm_menu = farm_menu_script.new()
	add_child(farm_menu)


func _spawn_npcs() -> void:
	var total_farmers := 0
	var total_guards := 0
	var total_raiders := 0

	for town_idx in towns.size():
		var town: Dictionary = towns[town_idx]
		var beds := _get_beds_from_town(town)
		var farms := _get_farms_from_town(town)
		var camp = town.camp
		var post_count: int = town.guard_posts.size()

		# Spawn farmers
		for i in Config.farmers_per_town:
			var bed = beds[i % beds.size()]
			var farm = farms[i % farms.size()]
			var spawn_offset := Vector2(randf_range(-15, 15), randf_range(-15, 15))
			var pos: Vector2 = bed.global_position + spawn_offset
			npc_manager.spawn_npc(pos.x, pos.y, 0, 0, {
				"home_x": bed.global_position.x,
				"home_y": bed.global_position.y,
				"work_x": farm.global_position.x,
				"work_y": farm.global_position.y,
				"town_idx": town_idx
			})
			total_farmers += 1

		# Spawn guards
		for i in Config.guards_per_town:
			var bed = beds[i % beds.size()]
			var spawn_offset := Vector2(randf_range(-15, 15), randf_range(-15, 15))
			var pos: Vector2 = bed.global_position + spawn_offset
			npc_manager.spawn_npc(pos.x, pos.y, 1, 0, {
				"home_x": bed.global_position.x,
				"home_y": bed.global_position.y,
				"town_idx": town_idx,
				"starting_post": i % post_count
			})
			total_guards += 1

		# Spawn raiders at camp (raider towns are at indices NUM_TOWNS..2*NUM_TOWNS-1)
		var raider_town_idx: int = NUM_TOWNS + town_idx
		for i in Config.raiders_per_camp:
			var spawn_offset := Vector2(randf_range(-80, 80), randf_range(-80, 80))
			var pos: Vector2 = camp.global_position + spawn_offset
			npc_manager.spawn_npc(pos.x, pos.y, 2, 1, {
				"home_x": camp.global_position.x,
				"home_y": camp.global_position.y,
				"town_idx": raider_town_idx
			})
			total_raiders += 1

	print("Spawned: %d farmers, %d guards, %d raiders" % [total_farmers, total_guards, total_raiders])




#func _spawn_town_npcs(town_idx: int) -> void:
	# Phase 2: respawn via count_alive() + spawn_npc()


#func _on_raider_delivered_food(town_idx: int) -> void:
#	if town_idx >= 0 and town_idx < camp_food.size():
#		camp_food[town_idx] += 1


#func _on_npc_ate_food(_npc_index: int, town_idx: int, job: int, _hp_before: float, _energy_before: float, _hp_after: float) -> void:
#	if job == NPCState.Job.RAIDER:
#		if town_idx >= 0 and town_idx < camp_food.size():
#			camp_food[town_idx] -= Config.FOOD_PER_MEAL
#	else:
#		if town_idx >= 0 and town_idx < town_food.size():
#			town_food[town_idx] -= Config.FOOD_PER_MEAL


func _process(_delta: float) -> void:
	# Redraw if showing active radius circle (follows camera)
	if UserSettings.show_active_radius:
		queue_redraw()

	# Redraw to update selected NPC target visualization
	if npc_manager.get_selected_npc() >= 0:
		queue_redraw()


func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed:
		match event.keycode:
			KEY_EQUAL:
				var time: Dictionary = npc_manager.get_game_time()
				npc_manager.set_time_scale(time.get("time_scale", 1.0) * 2.0)
			KEY_MINUS:
				var time: Dictionary = npc_manager.get_game_time()
				npc_manager.set_time_scale(time.get("time_scale", 1.0) / 2.0)
			KEY_SPACE:
				var time: Dictionary = npc_manager.get_game_time()
				npc_manager.set_paused(not time.get("paused", false))

	# Right-click on buildable slot opens build menu
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_RIGHT:
		var world_pos: Vector2 = get_global_mouse_position()
		var slot_info := _get_clicked_buildable_slot(world_pos)
		if slot_info.slot_key != "":
			build_menu.open(slot_info.slot_key, player_town_idx, event.position, slot_info.locked)
			get_viewport().set_input_as_handled()

	# Double-click on locked slot unlocks it
	if event is InputEventMouseButton and event.double_click and event.button_index == MOUSE_BUTTON_LEFT:
		var world_pos: Vector2 = get_global_mouse_position()
		var slot_info := _get_clicked_buildable_slot(world_pos)
		if slot_info.slot_key != "" and slot_info.locked:
			if _unlock_slot(player_town_idx, slot_info.slot_key):
				get_viewport().set_input_as_handled()

	# Left-click: guard post > NPC > terrain
	if event is InputEventMouseButton and event.pressed and not event.double_click and event.button_index == MOUSE_BUTTON_LEFT:
		var world_pos: Vector2 = get_global_mouse_position()
		var post_info := _get_clicked_guard_post(world_pos)
		if post_info.slot_key != "":
			guard_post_menu.open(post_info.slot_key, post_info.town_idx, event.position)
			get_viewport().set_input_as_handled()
		else:
			# Check for NPC click (20px radius)
			var clicked_npc: int = npc_manager.get_npc_at_position(world_pos.x, world_pos.y, 20.0)
			if clicked_npc >= 0:
				npc_manager.set_selected_npc(clicked_npc)
				get_viewport().set_input_as_handled()
			else:
				# Deselect NPC and select terrain
				npc_manager.set_selected_npc(-1)
				selected_tile = terrain_renderer.get_tile_at(world_pos)


#func _on_upgrade_purchased(upgrade_type: String, new_level: int) -> void:
#	# Phase 6: config-driven upgrades
#	if upgrade_type == "farmer_cap":
#		town_max_farmers[player_town_idx] = Config.max_farmers_per_town + new_level * Config.UPGRADE_FARMER_CAP_BONUS
#		return
#	if upgrade_type == "guard_cap":
#		town_max_guards[player_town_idx] = Config.max_guards_per_town + new_level * Config.UPGRADE_GUARD_CAP_BONUS
#		return
#	npc_manager.apply_town_upgrade(player_town_idx, upgrade_type, new_level)


func _draw_buildable_slots() -> void:
	if player_town_idx < 0 or player_town_idx >= towns.size():
		return

	var town: Dictionary = towns[player_town_idx]
	var grid: Dictionary = town.grid

	# Draw unlocked empty slots with green +
	for slot_key in town.slots.keys():
		if slot_key in FIXED_SLOTS:
			continue
		if not slot_key in grid:
			continue

		var slot_pos: Vector2 = grid[slot_key]
		var slot_contents: Array = town.slots[slot_key]

		# Count what's in the slot
		var bed_count := 0
		var has_building := false
		for building in slot_contents:
			if building.type == "bed":
				bed_count += 1
			elif building.type in ["farm", "guard_post"]:
				has_building = true

		# Skip full slots
		if has_building or bed_count >= 4:
			continue

		# Draw + icon for buildable slots
		var color := Color(0.5, 0.8, 0.5, 0.6)
		var size := 6.0
		draw_line(slot_pos + Vector2(-size, 0), slot_pos + Vector2(size, 0), color, 1.0)
		draw_line(slot_pos + Vector2(0, -size), slot_pos + Vector2(0, size), color, 1.0)

	# Draw adjacent locked slots with dim dotted corners
	var adjacent := _get_adjacent_locked_slots(player_town_idx)
	var locked_color := Color(0.6, 0.6, 0.6, 0.4)
	var corner_size := 4.0
	var half_slot := Config.TOWN_GRID_SPACING * 0.4

	for slot_key in adjacent:
		if not slot_key in grid:
			continue
		var slot_pos: Vector2 = grid[slot_key]
		# Draw corner brackets to indicate locked expandable slot
		# Top-left
		draw_line(slot_pos + Vector2(-half_slot, -half_slot), slot_pos + Vector2(-half_slot + corner_size, -half_slot), locked_color, 1.0)
		draw_line(slot_pos + Vector2(-half_slot, -half_slot), slot_pos + Vector2(-half_slot, -half_slot + corner_size), locked_color, 1.0)
		# Top-right
		draw_line(slot_pos + Vector2(half_slot, -half_slot), slot_pos + Vector2(half_slot - corner_size, -half_slot), locked_color, 1.0)
		draw_line(slot_pos + Vector2(half_slot, -half_slot), slot_pos + Vector2(half_slot, -half_slot + corner_size), locked_color, 1.0)
		# Bottom-left
		draw_line(slot_pos + Vector2(-half_slot, half_slot), slot_pos + Vector2(-half_slot + corner_size, half_slot), locked_color, 1.0)
		draw_line(slot_pos + Vector2(-half_slot, half_slot), slot_pos + Vector2(-half_slot, half_slot - corner_size), locked_color, 1.0)
		# Bottom-right
		draw_line(slot_pos + Vector2(half_slot, half_slot), slot_pos + Vector2(half_slot - corner_size, half_slot), locked_color, 1.0)
		draw_line(slot_pos + Vector2(half_slot, half_slot), slot_pos + Vector2(half_slot, half_slot - corner_size), locked_color, 1.0)


func _draw_selected_npc_target() -> void:
	var selected: int = npc_manager.get_selected_npc()
	if selected < 0:
		return

	var npc_pos: Vector2 = npc_manager.get_npc_position(selected)
	var target_pos: Vector2 = npc_manager.get_npc_target(selected)

	# Skip if no valid target (position 0,0 usually means not set)
	if target_pos == Vector2.ZERO or npc_pos == Vector2.ZERO:
		return

	# Draw line from NPC to target
	var line_color := Color(0.0, 1.0, 1.0, 0.7)  # Cyan
	draw_line(npc_pos, target_pos, line_color, 2.0)

	# Draw target marker (crosshair)
	var marker_size := 12.0
	var marker_color := Color(1.0, 0.0, 1.0, 0.9)  # Magenta
	draw_line(target_pos + Vector2(-marker_size, 0), target_pos + Vector2(marker_size, 0), marker_color, 2.0)
	draw_line(target_pos + Vector2(0, -marker_size), target_pos + Vector2(0, marker_size), marker_color, 2.0)
	draw_circle(target_pos, 8.0, marker_color)

	# Draw distance text
	var dist: float = npc_pos.distance_to(target_pos)
	# Note: Drawing text in _draw requires a font - skip for now, just show visual


func _calculate_grid_positions(center: Vector2) -> Dictionary:
	var s: float = Config.TOWN_GRID_SPACING
	var grid := {}
	# Calculate all possible positions up to max grid size
	for row in range(MAX_GRID_MIN, MAX_GRID_MAX + 1):
		for col in range(MAX_GRID_MIN, MAX_GRID_MAX + 1):
			var key := "%d,%d" % [row, col]
			# Offset so center is between (0,0) and (1,1)
			grid[key] = center + Vector2((col - 0.5) * s, (row - 0.5) * s)
	return grid


func _get_farms_from_town(town: Dictionary) -> Array:
	var farms: Array = []
	for key in town.slots:
		for building in town.slots[key]:
			if building.type == "farm":
				farms.append(building.node)
	return farms


func _get_beds_from_town(town: Dictionary) -> Array:
	var beds: Array = []
	for key in town.slots:
		for building in town.slots[key]:
			if building.type == "bed":
				beds.append(building.node)
	return beds


func _get_clicked_buildable_slot(world_pos: Vector2) -> Dictionary:
	# Only check player's town
	if player_town_idx < 0 or player_town_idx >= towns.size():
		return {"slot_key": "", "town_idx": -1, "locked": false}

	var town: Dictionary = towns[player_town_idx]
	var grid: Dictionary = town.grid
	var slot_radius: float = Config.TOWN_GRID_SPACING * 0.45  # Half slot size

	# Check unlocked slots except fountain (0,0)
	for slot_key in town.slots.keys():
		if slot_key == "0,0":
			continue  # Fountain is indestructible
		if not slot_key in grid:
			continue
		var slot_pos: Vector2 = grid[slot_key]
		if world_pos.distance_to(slot_pos) < slot_radius:
			return {"slot_key": slot_key, "town_idx": player_town_idx, "locked": false}

	# Check adjacent locked slots
	var adjacent := _get_adjacent_locked_slots(player_town_idx)
	for slot_key in adjacent:
		if not slot_key in grid:
			continue
		var slot_pos: Vector2 = grid[slot_key]
		if world_pos.distance_to(slot_pos) < slot_radius:
			return {"slot_key": slot_key, "town_idx": player_town_idx, "locked": true}

	return {"slot_key": "", "town_idx": -1, "locked": false}


func _get_clicked_guard_post(world_pos: Vector2) -> Dictionary:
	# Only check player's town
	if player_town_idx < 0 or player_town_idx >= towns.size():
		return {"slot_key": "", "town_idx": -1}

	var town: Dictionary = towns[player_town_idx]
	var click_radius := 20.0  # Guard post click area

	for slot_key in town.slots:
		for building in town.slots[slot_key]:
			if building.type == "guard_post":
				var pos: Vector2 = building.node.global_position
				if world_pos.distance_to(pos) < click_radius:
					return {"slot_key": slot_key, "town_idx": player_town_idx}

	return {"slot_key": "", "town_idx": -1}


#func _get_clicked_farm(world_pos: Vector2) -> Dictionary:
#	# Phase 5: needs ECS farm query API
#	return {"town_idx": -1, "farm_idx": -1}


func _on_build_requested(slot_key: String, building_type: String) -> void:
	if player_town_idx < 0 or player_town_idx >= towns.size():
		return

	var town: Dictionary = towns[player_town_idx]
	var grid: Dictionary = town.grid
	var slot_pos: Vector2 = grid[slot_key]

	if building_type == "farm":
		var farm = location_scene.instantiate()
		farm.location_name = "%s Farm" % town.name
		farm.location_type = "field"
		farm.global_position = slot_pos
		add_child(farm)
		town.slots[slot_key].append({"type": "farm", "node": farm})
		# Add to npc_manager farm positions (GDScript manager only)
		if not _uses_ecs:
			npc_manager.farm_positions.append(slot_pos)
			if player_town_idx < npc_manager.farms_by_town.size():
				npc_manager.farms_by_town[player_town_idx].append(slot_pos)
				npc_manager.farm_occupant_counts[player_town_idx].append(0)

	elif building_type == "bed":
		var slot_contents: Array = town.slots[slot_key]
		var bed_count := 0
		for b in slot_contents:
			if b.type == "bed":
				bed_count += 1

		# Calculate bed offset within slot (2x2 grid, 16px beds)
		var bed_offset := Vector2(
			(bed_count % 2 - 0.5) * 16,
			(floorf(bed_count / 2.0) - 0.5) * 16
		)

		var bed = location_scene.instantiate()
		bed.location_name = "%s Bed" % town.name
		bed.location_type = "home"
		bed.global_position = slot_pos + bed_offset
		add_child(bed)
		town.slots[slot_key].append({"type": "bed", "node": bed})
		# Add to bed tracking (GDScript manager only)
		if not _uses_ecs and player_town_idx < npc_manager.beds_by_town.size():
			npc_manager.beds_by_town[player_town_idx].append(bed.global_position)
			npc_manager.bed_occupants[player_town_idx].append(-1)

	elif building_type == "guard_post":
		var post = location_scene.instantiate()
		post.location_name = "%s Post" % town.name
		post.location_type = "guard_post"
		post.global_position = slot_pos
		add_child(post)
		town.slots[slot_key].append({"type": "guard_post", "node": post})
		# Add to guard posts for this town
		town.guard_posts.append(post)
		# Update npc_manager's guard post list (GDScript manager only)
		if not _uses_ecs:
			if player_town_idx < npc_manager.guard_posts_by_town.size():
				npc_manager.guard_posts_by_town[player_town_idx].append(slot_pos)
			if npc_manager._guard_post_combat:
				npc_manager._guard_post_combat.register_post(slot_pos, player_town_idx, slot_key)
		# Initialize guard post upgrades
		guard_post_upgrades[player_town_idx][slot_key] = {
			"attack_enabled": false,
			"range_level": 0,
			"damage_level": 0
		}

	queue_redraw()  # Update slot indicators


func _on_destroy_requested(slot_key: String) -> void:
	if player_town_idx < 0 or player_town_idx >= towns.size():
		return

	var town: Dictionary = towns[player_town_idx]
	var slot_contents: Array = town.slots[slot_key]

	# Remove all buildings in this slot
	for building in slot_contents:
		var node = building.node
		var btype = building.type

		# Remove from tracking arrays (GDScript manager only)
		if btype == "farm":
			if not _uses_ecs:
				var pos: Vector2 = node.global_position
				var farm_idx: int = npc_manager.farm_positions.find(pos)
				if farm_idx >= 0:
					npc_manager.farm_positions.remove_at(farm_idx)
				if player_town_idx < npc_manager.farms_by_town.size():
					var farms: Array = npc_manager.farms_by_town[player_town_idx]
					for fi in farms.size():
						if farms[fi] == pos:
							for npc_i in npc_manager.count:
								if npc_manager.current_farm_idx[npc_i] == fi and npc_manager.town_indices[npc_i] == player_town_idx:
									npc_manager.current_farm_idx[npc_i] = -1
								elif npc_manager.current_farm_idx[npc_i] > fi and npc_manager.town_indices[npc_i] == player_town_idx:
									npc_manager.current_farm_idx[npc_i] -= 1
							farms.remove_at(fi)
							npc_manager.farm_occupant_counts[player_town_idx].remove_at(fi)
							break
		elif btype == "guard_post":
			if not _uses_ecs:
				var pos: Vector2 = node.global_position
				if player_town_idx < npc_manager.guard_posts_by_town.size():
					var posts: Array = npc_manager.guard_posts_by_town[player_town_idx]
					for pi in posts.size():
						if posts[pi] == pos:
							posts.remove_at(pi)
							break
				if npc_manager._guard_post_combat:
					npc_manager._guard_post_combat.unregister_post(player_town_idx, slot_key)
			for gi in town.guard_posts.size():
				if town.guard_posts[gi] == node:
					town.guard_posts.remove_at(gi)
					break
			if guard_post_upgrades[player_town_idx].has(slot_key):
				guard_post_upgrades[player_town_idx].erase(slot_key)
		elif btype == "bed":
			if not _uses_ecs:
				var pos: Vector2 = node.global_position
				if player_town_idx < npc_manager.beds_by_town.size():
					var beds: Array = npc_manager.beds_by_town[player_town_idx]
					for bi in beds.size():
						if beds[bi] == pos:
							var occupant: int = npc_manager.bed_occupants[player_town_idx][bi]
							if occupant >= 0:
								npc_manager.current_bed_idx[occupant] = -1
							beds.remove_at(bi)
							npc_manager.bed_occupants[player_town_idx].remove_at(bi)
							break

		node.queue_free()

	slot_contents.clear()
	queue_redraw()


func _on_unlock_requested(slot_key: String) -> void:
	_unlock_slot(player_town_idx, slot_key)


func _get_adjacent_locked_slots(town_idx: int) -> Array:
	if town_idx < 0 or town_idx >= towns.size():
		return []

	var town: Dictionary = towns[town_idx]
	var adjacent: Array = []

	# For each unlocked slot, check 4 neighbors
	for slot_key in town.slots.keys():
		var parts = slot_key.split(",")
		var row: int = int(parts[0])
		var col: int = int(parts[1])

		# Check 4 directions
		for offset in [[-1, 0], [1, 0], [0, -1], [0, 1]]:
			var nr: int = row + offset[0]
			var nc: int = col + offset[1]
			# Stay within max grid bounds
			if nr < MAX_GRID_MIN or nr > MAX_GRID_MAX:
				continue
			if nc < MAX_GRID_MIN or nc > MAX_GRID_MAX:
				continue
			var neighbor_key := "%d,%d" % [nr, nc]
			# If not already unlocked and not already in list
			if not neighbor_key in town.slots and not neighbor_key in adjacent:
				adjacent.append(neighbor_key)

	return adjacent


func _unlock_slot(town_idx: int, slot_key: String) -> bool:
	if town_idx < 0 or town_idx >= towns.size():
		return false

	var town: Dictionary = towns[town_idx]

	# Check if slot is adjacent to an unlocked slot
	var adjacent := _get_adjacent_locked_slots(town_idx)
	if not slot_key in adjacent:
		return false

	# Check food cost
	if town_food[town_idx] < Config.SLOT_UNLOCK_COST:
		return false

	# Unlock the slot
	town_food[town_idx] -= Config.SLOT_UNLOCK_COST
	town.slots[slot_key] = []
	queue_redraw()
	return true


func _find_camp_position(town_center: Vector2, all_town_centers: Array[Vector2]) -> Vector2:
	var min_dist_from_any_town := 700.0  # Must be past guard posts (500-600px)
	var best_pos := town_center
	var best_score := -999999.0

	# Try 16 directions, pick the one furthest from all towns
	for i in 16:
		var angle: float = i * TAU / 16.0 + randf_range(-0.1, 0.1)
		var dir := Vector2(cos(angle), sin(angle))
		var pos: Vector2 = town_center + dir * Config.CAMP_DISTANCE

		# Clamp to world bounds
		pos.x = clampf(pos.x, Config.WORLD_MARGIN, Config.world_width - Config.WORLD_MARGIN)
		pos.y = clampf(pos.y, Config.WORLD_MARGIN, Config.world_height - Config.WORLD_MARGIN)

		# Score = minimum distance to any town (higher is better)
		var min_dist := 999999.0
		for tc in all_town_centers:
			min_dist = minf(min_dist, pos.distance_to(tc))

		if min_dist > best_score:
			best_score = min_dist
			best_pos = pos

	# If best position is still too close to any town, try to push it away
	if best_score < min_dist_from_any_town:
		# Find direction away from nearest town
		var nearest_town := town_center
		var nearest_dist := 999999.0
		for tc in all_town_centers:
			var d := best_pos.distance_to(tc)
			if d < nearest_dist:
				nearest_dist = d
				nearest_town = tc
		var away_dir: Vector2 = (best_pos - nearest_town).normalized()
		if away_dir.length_squared() < 0.1:
			away_dir = Vector2.RIGHT
		best_pos = nearest_town + away_dir * min_dist_from_any_town

	return best_pos
