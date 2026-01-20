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
var build_menu_scene: PackedScene = preload("res://ui/build_menu.tscn")
var policies_panel_scene: PackedScene = preload("res://ui/policies_panel.tscn")

var npc_manager: Node
var projectile_manager: Node
var player: Node
var left_panel: Node
var settings_menu: Node
var upgrade_menu: Node
var build_menu: Node

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

# Grid slot keys - max 10x10 grid (-4 to +5)
# Town starts at 6x6 (-2 to +3), expands with grid_size upgrade
const BASE_GRID_MIN := -2
const BASE_GRID_MAX := 3
const MAX_GRID_MIN := -4
const MAX_GRID_MAX := 5

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
	return [
		"%d,%d" % [bounds.min, bounds.min],
		"%d,%d" % [bounds.min, bounds.max],
		"%d,%d" % [bounds.max, bounds.min],
		"%d,%d" % [bounds.max, bounds.max]
	]


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

	# Draw buildable slot indicators for player's town
	_draw_buildable_slots()


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
			"work_schedule": 0            # 0=both shifts, 1=day only, 2=night only
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

		# Create initial beds in (-1,-1) slot (4 beds in 2x2 grid)
		for bed_idx in 4:
			var bed_offset := Vector2(
				(bed_idx % 2 - 0.5) * 16,
				(floorf(bed_idx / 2.0) - 0.5) * 16
			)
			var bed = location_scene.instantiate()
			bed.location_name = "%s Bed" % town_name
			bed.location_type = "home"
			bed.global_position = grid["-1,-1"] + bed_offset
			add_child(bed)
			town_data.slots["-1,-1"].append({"type": "bed", "node": bed})

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
		var farms := _get_farms_from_town(town)
		for farm in farms:
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

	# Pass town upgrades and policies references
	npc_manager.town_upgrades = town_upgrades
	npc_manager.town_policies = town_policies

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

	build_menu = build_menu_scene.instantiate()
	build_menu.build_requested.connect(_on_build_requested)
	build_menu.destroy_requested.connect(_on_destroy_requested)
	build_menu.unlock_requested.connect(_on_unlock_requested)
	add_child(build_menu)

	var policies_panel = policies_panel_scene.instantiate()
	add_child(policies_panel)


func _spawn_npcs() -> void:
	var total_farmers := 0
	var total_guards := 0
	var total_raiders := 0

	for town_idx in towns.size():
		var town: Dictionary = towns[town_idx]
		var beds := _get_beds_from_town(town)
		var farms := _get_farms_from_town(town)
		var camp = town.camp

		# Spawn farmers (target building centers, spawn with small offset)
		for i in Config.FARMERS_PER_TOWN:
			var bed = beds[i % beds.size()]
			var farm = farms[i % farms.size()]
			var spawn_offset := Vector2(randf_range(-15, 15), randf_range(-15, 15))
			npc_manager.spawn_farmer(
				bed.global_position + spawn_offset,
				bed.global_position,  # home center
				farm.global_position,  # farm center
				town_idx
			)
			total_farmers += 1

		# Spawn guards (live in beds, patrol at posts)
		# Alternate day/night shifts for even coverage
		for i in Config.GUARDS_PER_TOWN:
			var bed = beds[i % beds.size()]
			var spawn_offset := Vector2(randf_range(-15, 15), randf_range(-15, 15))
			var night_shift: bool = i % 2 == 1  # Odd = night, even = day
			npc_manager.spawn_guard(
				bed.global_position + spawn_offset,
				bed.global_position,  # home center
				bed.global_position,  # unused - guards patrol all posts
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
	var beds := _get_beds_from_town(town)
	var farms := _get_farms_from_town(town)
	var camp = town.camp
	var town_center: Vector2 = town.center

	# Spawn 1 farmer at fountain (if under cap)
	var farmer_count: int = npc_manager.count_alive_by_job_and_town(NPCState.Job.FARMER, town_idx)
	if farmer_count < town_max_farmers[town_idx] and beds.size() > 0 and farms.size() > 0:
		var bed = beds[randi() % beds.size()]
		var farm = farms[randi() % farms.size()]
		var farmer_idx: int = npc_manager.spawn_farmer(
			town_center,
			bed.global_position,
			farm.global_position,
			town_idx
		)
		npc_manager.npc_spawned.emit(farmer_idx, NPCState.Job.FARMER, town_idx)

	# Spawn 1 guard at fountain (if under cap)
	var guard_count: int = npc_manager.count_alive_by_job_and_town(NPCState.Job.GUARD, town_idx)
	if guard_count < town_max_guards[town_idx] and beds.size() > 0:
		var bed = beds[randi() % beds.size()]
		var night_shift: bool = randi() % 2 == 1
		var guard_idx: int = npc_manager.spawn_guard(
			town_center,
			bed.global_position,
			bed.global_position,
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

	# Right-click on buildable slot opens build menu
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_RIGHT:
		var world_pos: Vector2 = get_global_mouse_position()
		var slot_info := _get_clicked_buildable_slot(world_pos)
		if slot_info.slot_key != "":
			build_menu.open(slot_info.slot_key, player_town_idx, event.position, slot_info.locked)
			get_viewport().set_input_as_handled()


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
		# Add to npc_manager farm positions
		npc_manager.farm_positions.append(slot_pos)

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

	elif building_type == "guard_post":
		var post = location_scene.instantiate()
		post.location_name = "%s Post" % town.name
		post.location_type = "guard_post"
		post.global_position = slot_pos
		add_child(post)
		town.slots[slot_key].append({"type": "guard_post", "node": post})
		# Add to guard posts for this town
		town.guard_posts.append(post)
		# Update npc_manager's guard post list for this town
		if player_town_idx < npc_manager.guard_posts_by_town.size():
			npc_manager.guard_posts_by_town[player_town_idx].append(slot_pos)

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

		# Remove from tracking arrays
		if btype == "farm":
			var pos: Vector2 = node.global_position
			var farm_idx: int = npc_manager.farm_positions.find(pos)
			if farm_idx >= 0:
				npc_manager.farm_positions.remove_at(farm_idx)
		elif btype == "guard_post":
			var pos: Vector2 = node.global_position
			if player_town_idx < npc_manager.guard_posts_by_town.size():
				var posts: Array = npc_manager.guard_posts_by_town[player_town_idx]
				for pi in posts.size():
					if posts[pi] == pos:
						posts.remove_at(pi)
						break
			for gi in town.guard_posts.size():
				if town.guard_posts[gi] == node:
					town.guard_posts.remove_at(gi)
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
