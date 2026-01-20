extends Node2D

var npc_manager_scene: PackedScene = preload("res://systems/npc_manager.tscn")
var projectile_manager_scene: PackedScene = preload("res://systems/projectile_manager.tscn")
var player_scene: PackedScene = preload("res://entities/player.tscn")
var location_scene: PackedScene = preload("res://world/location.tscn")
var left_panel_scene: PackedScene = preload("res://ui/left_panel.tscn")
var settings_menu_scene: PackedScene = preload("res://ui/settings_menu.tscn")
var upgrade_menu_scene: PackedScene = preload("res://ui/upgrade_menu.tscn")
var combat_log_scene: PackedScene = preload("res://ui/combat_log.tscn")
var roster_panel_scene: PackedScene = preload("res://ui/roster_panel.tscn")

var npc_manager: Node
var projectile_manager: Node
var player: Node
var left_panel: Node
var settings_menu: Node
var upgrade_menu: Node

# World data
var towns: Array = []  # Array of {center, farms, homes, camp, food}
var town_food: PackedInt32Array  # Food stored in each town
var camp_food: PackedInt32Array  # Food stored in each raider camp
var spawn_timers: PackedInt32Array  # Hours since last spawn per town
var player_town_idx: int = 0  # First town is player-controlled
var town_upgrades: Array = []  # Per-town upgrade levels
var town_max_farmers: PackedInt32Array  # Population cap per town
var town_max_guards: PackedInt32Array   # Population cap per town

const NUM_TOWNS := 7
const MIN_TOWN_DISTANCE := 1200  # Minimum distance between town centers
const FOOD_PER_WORK_HOUR := 1  # Food generated per farmer per work hour

const TOWN_NAMES := [
	"Miami", "Orlando", "Tampa", "Jacksonville", "Tallahassee",
	"Gainesville", "Pensacola", "Sarasota", "Naples", "Daytona",
	"Lakeland", "Ocala", "Boca Raton", "Key West", "Fort Myers"
]


func _ready() -> void:
	WorldClock.day_changed.connect(_on_day_changed)
	WorldClock.time_tick.connect(_on_time_tick)

	_generate_world()
	_setup_managers()
	_setup_player()
	_setup_ui()
	_spawn_npcs()


func _draw() -> void:
	# World border
	var border_color := Color(0.4, 0.4, 0.4, 0.8)
	var border_width := 4.0
	var rect := Rect2(0, 0, Config.WORLD_WIDTH, Config.WORLD_HEIGHT)
	draw_rect(rect, border_color, false, border_width)

	# Corner markers for visibility
	var marker_size := 50.0
	var corners := [
		Vector2(0, 0),
		Vector2(Config.WORLD_WIDTH, 0),
		Vector2(Config.WORLD_WIDTH, Config.WORLD_HEIGHT),
		Vector2(0, Config.WORLD_HEIGHT)
	]
	for corner in corners:
		draw_circle(corner, marker_size, border_color)

	# Player town indicator - gold ring around fountain
	if towns.size() > player_town_idx:
		var town_center: Vector2 = towns[player_town_idx].center
		var gold := Color(1.0, 0.85, 0.3, 0.8)
		draw_arc(town_center, 60.0, 0, TAU, 32, gold, 3.0)


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
		town_max_farmers[i] = Config.MAX_FARMERS_PER_TOWN
		town_max_guards[i] = Config.MAX_GUARDS_PER_TOWN

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
			"guard_cap": 0
		})

	# Generate scattered town positions
	var town_positions: Array[Vector2] = []
	var attempts := 0
	var max_attempts := 1000

	while town_positions.size() < NUM_TOWNS and attempts < max_attempts:
		attempts += 1
		var pos := Vector2(
			randf_range(Config.WORLD_MARGIN, Config.WORLD_WIDTH - Config.WORLD_MARGIN),
			randf_range(Config.WORLD_MARGIN, Config.WORLD_HEIGHT - Config.WORLD_MARGIN)
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
		var town_data := {
			"name": town_name,
			"center": town_center,
			"farms": [],
			"homes": [],
			"guard_posts": [],
			"camp": null
		}

		# Create farms (close to town center)
		for f in Config.FARMS_PER_TOWN:
			var angle: float = (f / float(Config.FARMS_PER_TOWN)) * TAU + randf_range(-0.3, 0.3)
			var dist: float = randf_range(200, 300)
			var farm_pos: Vector2 = town_center + Vector2(cos(angle), sin(angle)) * dist

			var farm = location_scene.instantiate()
			farm.location_name = "%s Farm" % town_name
			farm.location_type = "field"
			farm.global_position = farm_pos
			add_child(farm)
			town_data.farms.append(farm)

		# Create homes (ring around center) - just for farmers, guards patrol from posts
		var num_homes: int = Config.FARMERS_PER_TOWN
		for h in num_homes:
			var angle: float = (h / float(num_homes)) * TAU
			var dist: float = randf_range(350, 450)
			var home_pos: Vector2 = town_center + Vector2(cos(angle), sin(angle)) * dist

			var home = location_scene.instantiate()
			home.location_name = "%s Home" % town_name
			home.location_type = "home"
			home.global_position = home_pos
			add_child(home)
			town_data.homes.append(home)

		# Create guard posts (perimeter around town, between homes and camp)
		for g in Config.GUARD_POSTS_PER_TOWN:
			var angle: float = (g / float(Config.GUARD_POSTS_PER_TOWN)) * TAU
			var dist: float = randf_range(500, 600)
			var post_pos: Vector2 = town_center + Vector2(cos(angle), sin(angle)) * dist

			var post = location_scene.instantiate()
			post.location_name = "%s Post" % town_name
			post.location_type = "guard_post"
			post.global_position = post_pos
			add_child(post)
			town_data.guard_posts.append(post)

		# Create raider camp (away from all towns, in direction with most room)
		var camp_pos := _find_camp_position(town_center, town_positions)

		var camp = location_scene.instantiate()
		camp.location_name = "%s Raiders" % town_name
		camp.location_type = "camp"
		camp.global_position = camp_pos
		add_child(camp)
		town_data.camp = camp

		# Create town center marker
		var town_marker = location_scene.instantiate()
		town_marker.location_name = town_name
		town_marker.location_type = "fountain"
		town_marker.global_position = town_center
		add_child(town_marker)

		towns.append(town_data)

	print("Generated %d towns" % towns.size())


func _setup_managers() -> void:
	npc_manager = npc_manager_scene.instantiate()
	add_child(npc_manager)
	npc_manager.raider_delivered_food.connect(_on_raider_delivered_food)

	projectile_manager = projectile_manager_scene.instantiate()
	add_child(projectile_manager)

	projectile_manager.set_npc_manager(npc_manager)
	npc_manager.set_projectile_manager(projectile_manager)

	# Pass farm positions to npc_manager
	for town in towns:
		for farm in town.farms:
			npc_manager.farm_positions.append(farm.global_position)

	# Pass guard post positions per town
	for town in towns:
		var posts: Array[Vector2] = []
		for post in town.guard_posts:
			posts.append(post.global_position)
		npc_manager.guard_posts_by_town.append(posts)

	# Pass town centers (fountains) for flee destinations
	for town in towns:
		npc_manager.town_centers.append(town.center)

	# Pass town upgrades reference
	npc_manager.town_upgrades = town_upgrades

	# Pass food references for eating
	npc_manager.town_food = town_food
	npc_manager.camp_food = camp_food
	npc_manager.npc_ate_food.connect(_on_npc_ate_food)

	# Set village center to world center (for compatibility)
	@warning_ignore("integer_division")
	npc_manager.village_center = Vector2(Config.WORLD_WIDTH / 2, Config.WORLD_HEIGHT / 2)


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
	upgrade_menu.upgrade_purchased.connect(_on_upgrade_purchased)
	add_child(upgrade_menu)

	var combat_log = combat_log_scene.instantiate()
	add_child(combat_log)

	var roster_panel = roster_panel_scene.instantiate()
	add_child(roster_panel)


func _spawn_npcs() -> void:
	var total_farmers := 0
	var total_guards := 0
	var total_raiders := 0

	for town_idx in towns.size():
		var town: Dictionary = towns[town_idx]
		var homes: Array = town.homes
		var farms: Array = town.farms
		var camp = town.camp

		# Spawn farmers (target building centers, spawn with small offset)
		for i in Config.FARMERS_PER_TOWN:
			var home = homes[i % homes.size()]
			var farm = farms[i % farms.size()]
			var spawn_offset := Vector2(randf_range(-15, 15), randf_range(-15, 15))
			npc_manager.spawn_farmer(
				home.global_position + spawn_offset,
				home.global_position,  # home center
				farm.global_position,  # farm center
				town_idx
			)
			total_farmers += 1

		# Spawn guards (live in homes, patrol at posts)
		# Alternate day/night shifts for even coverage
		for i in Config.GUARDS_PER_TOWN:
			var home = homes[i % homes.size()]
			var spawn_offset := Vector2(randf_range(-15, 15), randf_range(-15, 15))
			var night_shift: bool = i % 2 == 1  # Odd = night, even = day
			npc_manager.spawn_guard(
				home.global_position + spawn_offset,
				home.global_position,  # home center
				home.global_position,  # unused - guards patrol all posts
				night_shift,
				town_idx
			)
			total_guards += 1

		# Spawn raiders at camp
		for i in Config.RAIDERS_PER_CAMP:
			var spawn_offset := Vector2(randf_range(-80, 80), randf_range(-80, 80))
			npc_manager.spawn_raider(
				camp.global_position + spawn_offset,
				camp.global_position,  # camp center
				town_idx
			)
			total_raiders += 1

	print("Spawned: %d farmers, %d guards, %d raiders" % [total_farmers, total_guards, total_raiders])


func _on_day_changed(day: int) -> void:
	print("=== DAY %d ===" % day)


func _on_time_tick(_hour: int, minute: int) -> void:
	# Only process on the hour
	if minute != 0:
		return

	# Generate food when farmers are working
	for i in npc_manager.count:
		if npc_manager.healths[i] <= 0:
			continue
		if npc_manager.jobs[i] != NPCState.Job.FARMER:
			continue
		if npc_manager.states[i] != NPCState.State.FARMING:
			continue

		var town_idx: int = npc_manager.town_indices[i]
		if town_idx >= 0 and town_idx < town_food.size():
			var yield_level: int = town_upgrades[town_idx].farm_yield
			var yield_mult: float = 1.0 + yield_level * Config.UPGRADE_FARM_YIELD_BONUS
			var npc_trait: int = npc_manager.traits[i]
			if npc_trait == NPCState.Trait.EFFICIENT:
				yield_mult *= 1.25
			elif npc_trait == NPCState.Trait.LAZY:
				yield_mult *= 0.8
			town_food[town_idx] += int(FOOD_PER_WORK_HOUR * yield_mult)

	# Spawn new NPCs at regular intervals
	for town_idx in towns.size():
		spawn_timers[town_idx] += 1
		if spawn_timers[town_idx] >= Config.SPAWN_INTERVAL_HOURS:
			spawn_timers[town_idx] = 0
			_spawn_town_npcs(town_idx)


func _spawn_town_npcs(town_idx: int) -> void:
	var town: Dictionary = towns[town_idx]
	var homes: Array = town.homes
	var farms: Array = town.farms
	var camp = town.camp
	var town_center: Vector2 = town.center

	# Spawn 1 farmer at fountain (if under cap)
	var farmer_count: int = npc_manager.count_alive_by_job_and_town(NPCState.Job.FARMER, town_idx)
	if farmer_count < town_max_farmers[town_idx]:
		var home = homes[randi() % homes.size()]
		var farm = farms[randi() % farms.size()]
		var farmer_idx: int = npc_manager.spawn_farmer(
			town_center,
			home.global_position,
			farm.global_position,
			town_idx
		)
		npc_manager.npc_spawned.emit(farmer_idx, NPCState.Job.FARMER, town_idx)

	# Spawn 1 guard at fountain (if under cap)
	var guard_count: int = npc_manager.count_alive_by_job_and_town(NPCState.Job.GUARD, town_idx)
	if guard_count < town_max_guards[town_idx]:
		var home = homes[randi() % homes.size()]
		var night_shift: bool = randi() % 2 == 1
		var guard_idx: int = npc_manager.spawn_guard(
			town_center,
			home.global_position,
			home.global_position,
			night_shift,
			town_idx
		)
		npc_manager.npc_spawned.emit(guard_idx, NPCState.Job.GUARD, town_idx)

	# Spawn 1 raider at camp
	var raider_idx: int = npc_manager.spawn_raider(
		camp.global_position,
		camp.global_position,
		town_idx
	)
	npc_manager.npc_spawned.emit(raider_idx, NPCState.Job.RAIDER, town_idx)


func _on_raider_delivered_food(town_idx: int) -> void:
	if town_idx >= 0 and town_idx < camp_food.size():
		camp_food[town_idx] += 1


func _on_npc_ate_food(_npc_index: int, town_idx: int, job: int, _hp_before: float, _energy_before: float, _hp_after: float) -> void:
	if job == NPCState.Job.RAIDER:
		if town_idx >= 0 and town_idx < camp_food.size():
			camp_food[town_idx] -= Config.FOOD_PER_MEAL
	else:
		if town_idx >= 0 and town_idx < town_food.size():
			town_food[town_idx] -= Config.FOOD_PER_MEAL


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


func _on_upgrade_purchased(upgrade_type: String, new_level: int) -> void:
	# Handle population cap upgrades
	if upgrade_type == "farmer_cap":
		town_max_farmers[player_town_idx] = Config.MAX_FARMERS_PER_TOWN + new_level * Config.UPGRADE_FARMER_CAP_BONUS
		return
	if upgrade_type == "guard_cap":
		town_max_guards[player_town_idx] = Config.MAX_GUARDS_PER_TOWN + new_level * Config.UPGRADE_GUARD_CAP_BONUS
		return

	# Apply upgrade to all guards in this town
	npc_manager.apply_town_upgrade(player_town_idx, upgrade_type, new_level)


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
		pos.x = clampf(pos.x, Config.WORLD_MARGIN, Config.WORLD_WIDTH - Config.WORLD_MARGIN)
		pos.y = clampf(pos.y, Config.WORLD_MARGIN, Config.WORLD_HEIGHT - Config.WORLD_MARGIN)

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
