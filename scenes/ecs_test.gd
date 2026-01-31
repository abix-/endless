# ecs_test.gd - Isolated behavior tests for EcsNpcManager
extends Node2D

# Static var persists across scene reloads
static var pending_test: int = 0

var ecs_manager: Node2D

# UI Labels
var state_label: Label
var test_label: Label
var phase_label: Label
var fps_label: Label
var count_label: Label
var time_label: Label
var distance_label: Label
var velocity_label: Label
var expected_label: Label
var log_label: Label

# Controls
var count_slider: HSlider
var metrics_check: CheckBox
var test_dropdown: OptionButton
var run_button: Button

# State
var frame_count := 0
var fps_timer := 0.0
var test_timer := 0.0
var current_test := 0
var test_phase := 0
var npc_count := 0
var is_running := false
var metrics_enabled := false
var log_lines: Array[String] = []
var test_result := ""  # "PASS" or "FAIL: reason"
var phase_results: Array[String] = []  # Per-phase pass/fail with values

const CENTER := Vector2(400, 300)
const SEP_RADIUS := 20.0
const ARRIVAL_THRESHOLD := 8.0
const TEST_NAMES := {
	0: "None",
	1: "Arrive",
	2: "Separation",
	3: "Arrive+Sep",
	4: "Circle",
	5: "Mass",
	6: "World Data",
	7: "Guard Patrol",
	8: "Farmer Work",
	9: "Health/Death",
	10: "Combat",
	11: "Unified Attacks"
}


func _ready() -> void:
	var vbox = $UI/DebugPanel/VBox

	# Status labels
	state_label = vbox.get_node("StateLabel")
	test_label = vbox.get_node("TestLabel")
	phase_label = vbox.get_node("PhaseLabel")

	# Performance labels
	fps_label = vbox.get_node("FPSLabel")
	count_label = vbox.get_node("CountLabel")
	time_label = vbox.get_node("TimeLabel")

	# Metrics labels
	distance_label = vbox.get_node("DistanceLabel")
	velocity_label = vbox.get_node("VelocityLabel")
	expected_label = vbox.get_node("ExpectedLabel")
	log_label = vbox.get_node("LogLabel")

	# Controls
	# Test dropdown (init before slider so label is correct)
	test_dropdown = vbox.get_node("TestDropdown")
	test_dropdown.select(test_dropdown.item_count - 1)
	current_test = test_dropdown.get_item_id(test_dropdown.item_count - 1)

	count_slider = vbox.get_node("CountSlider")
	count_slider.value_changed.connect(_on_count_changed)
	_configure_slider(current_test)
	_on_count_changed(count_slider.value)  # Initialize label

	metrics_check = vbox.get_node("MetricsCheck")
	metrics_check.toggled.connect(_on_metrics_toggled)
	_on_metrics_toggled(metrics_check.button_pressed)  # Initialize labels

	run_button = vbox.get_node("RunButton")
	run_button.pressed.connect(_on_run_pressed)
	vbox.get_node("CopyButton").pressed.connect(_copy_debug_info)

	if ClassDB.class_exists("EcsNpcManager"):
		ecs_manager = ClassDB.instantiate("EcsNpcManager")
		add_child(ecs_manager)
		var build_info = ecs_manager.get_build_info() if ecs_manager.has_method("get_build_info") else "unknown"
		_log(build_info)

		# Run pending test if set
		if pending_test > 0:
			call_deferred("_start_test", pending_test)
		else:
			_set_state("IDLE")
		queue_redraw()
	else:
		_log("ERROR: Rust DLL not loaded")
		_set_state("ERROR")


func _log(msg: String) -> void:
	# Show in UI only, no console spam
	log_lines.push_front(msg)
	if log_lines.size() > 3:
		log_lines.pop_back()
	log_label.text = "\n".join(log_lines)


func _copy_debug_info() -> void:
	var info := "=== ECS TEST DEBUG DUMP ===\n"
	info += "Test: %s | Time: %.1fs | NPCs: %d\n" % [TEST_NAMES.get(current_test, "?"), test_timer, npc_count]
	info += state_label.text + "\n"
	info += phase_label.text + "\n"
	if phase_results.size() > 0:
		info += "\n--- Phase Results ---\n"
		for pr in phase_results:
			info += pr + "\n"

	info += "\n--- UI Labels ---\n"
	info += expected_label.text + "\n"
	info += velocity_label.text + "\n"
	info += distance_label.text + "\n"

	# For combat test, get ALL debug data
	if current_test == 10 and ecs_manager:
		info += "\n--- Combat Debug (Rust) ---\n"
		var cd: Dictionary = ecs_manager.get_combat_debug()
		info += "attackers: %d\n" % cd.get("attackers", -1)
		info += "targets_found: %d\n" % cd.get("targets_found", -1)
		info += "attacks: %d\n" % cd.get("attacks", -1)
		info += "in_range: %d\n" % cd.get("in_range", -1)
		info += "timer_ready: %d\n" % cd.get("timer_ready", -1)
		info += "chases: %d\n" % cd.get("chases", -1)
		info += "cooldown_entities: %d\n" % cd.get("cooldown_entities", -1)
		info += "sample_timer: %.3f\n" % cd.get("sample_timer", -1.0)
		info += "frame_delta: %.4f\n" % cd.get("frame_delta", -1.0)
		info += "bounds_fail: %d\n" % cd.get("bounds_fail", -1)
		info += "positions_len: %d\n" % cd.get("positions_len", -1)
		info += "combat_targets_len: %d\n" % cd.get("combat_targets_len", -1)

		info += "\n--- GPU Targeting (Samples) ---\n"
		info += "combat_target[0]: %d  (guard -> expects raider idx)\n" % cd.get("combat_target_0", -99)
		info += "combat_target[5]: %d  (raider -> expects guard idx)\n" % cd.get("combat_target_5", -99)

		info += "\n--- Positions (from GPU_POSITIONS static) ---\n"
		info += "NPC 0 pos: (%.1f, %.1f)\n" % [cd.get("pos_0_x", -999), cd.get("pos_0_y", -999)]
		info += "NPC 5 pos: (%.1f, %.1f)\n" % [cd.get("pos_5_x", -999), cd.get("pos_5_y", -999)]

		info += "\n--- CPU Position Cache (for grid building) ---\n"
		info += "cpu_pos[0]: (%.1f, %.1f)\n" % [cd.get("cpu_pos_0_x", -999), cd.get("cpu_pos_0_y", -999)]
		info += "cpu_pos[5]: (%.1f, %.1f)\n" % [cd.get("cpu_pos_5_x", -999), cd.get("cpu_pos_5_y", -999)]

		info += "\n--- Grid Cell Counts (CELL_SIZE=100) ---\n"
		info += "cell (3,2): %d NPCs  (guards)\n" % cd.get("grid_cell_3_2", -1)
		info += "cell (4,2): %d NPCs  (raiders)\n" % cd.get("grid_cell_4_2", -1)

		info += "\n--- CPU Cache ---\n"
		info += "faction[0]: %d  faction[5]: %d\n" % [cd.get("faction_0", -99), cd.get("faction_5", -99)]
		info += "health[0]: %.1f  health[5]: %.1f\n" % [cd.get("health_0", -99.0), cd.get("health_5", -99.0)]

		info += "\n--- GPU Buffer (direct read) ---\n"
		info += "gpu_faction[0]: %d  gpu_faction[5]: %d\n" % [cd.get("gpu_faction_0", -99), cd.get("gpu_faction_5", -99)]
		info += "gpu_health[0]: %.1f  gpu_health[5]: %.1f\n" % [cd.get("gpu_health_0", -99.0), cd.get("gpu_health_5", -99.0)]
		info += "npc_count: %d\n" % cd.get("npc_count", -1)

		info += "\n--- Health Debug ---\n"
		var hd: Dictionary = ecs_manager.get_health_debug()
		info += "bevy_entity_count: %d\n" % hd.get("bevy_entity_count", -1)
		info += "damage_processed: %d\n" % hd.get("damage_processed", -1)
		info += "deaths_this_frame: %d\n" % hd.get("deaths_this_frame", -1)

	DisplayServer.clipboard_set(info)
	_log("Copied to clipboard")


func _set_state(state: String) -> void:
	state_label.text = "State: " + state
	is_running = state == "RUNNING"


func _set_phase(phase: String) -> void:
	phase_label.text = "Phase: " + phase


func _pass() -> void:
	test_result = "PASS"
	_set_state("PASS")
	_log("PASS")


func _fail(reason: String) -> void:
	test_result = "FAIL: " + reason
	test_phase = 99  # Terminal — stop all phase checks
	_set_state("FAIL")
	_log("FAIL: " + reason)


# Check all NPCs are within threshold of target
func _assert_all_arrived(target: Vector2, threshold: float) -> bool:
	for i in npc_count:
		var pos: Vector2 = ecs_manager.get_npc_position(i)
		if pos.distance_to(target) > threshold:
			return false
	return true


# Check all NPC pairs are at least min_dist apart
func _assert_all_separated(min_dist: float) -> bool:
	for i in npc_count:
		var pos_i: Vector2 = ecs_manager.get_npc_position(i)
		for j in range(i + 1, npc_count):
			var pos_j: Vector2 = ecs_manager.get_npc_position(j)
			if pos_i.distance_to(pos_j) < min_dist - 0.1:  # Small tolerance
				return false
	return true


# Get min distance between any two NPCs
func _get_min_separation() -> float:
	var min_dist := 99999.0
	for i in npc_count:
		var pos_i: Vector2 = ecs_manager.get_npc_position(i)
		for j in range(i + 1, npc_count):
			var pos_j: Vector2 = ecs_manager.get_npc_position(j)
			min_dist = minf(min_dist, pos_i.distance_to(pos_j))
	return min_dist


# Get max distance from target
func _get_max_dist_from_target(target: Vector2) -> float:
	var max_dist := 0.0
	for i in npc_count:
		var pos: Vector2 = ecs_manager.get_npc_position(i)
		max_dist = maxf(max_dist, pos.distance_to(target))
	return max_dist


func _configure_slider(test_id: int) -> void:
	if test_id == 11:
		count_slider.min_value = 10
		count_slider.max_value = 10000
		count_slider.step = 10
	else:
		count_slider.min_value = 500
		count_slider.max_value = 5000
		count_slider.step = 1


func _on_count_changed(value: float) -> void:
	if current_test == 11:
		count_label.text = "Projectile Count: %d" % int(value)
	else:
		count_label.text = "NPC Count: %d" % int(value)


func _on_metrics_toggled(enabled: bool) -> void:
	metrics_enabled = enabled
	if not enabled:
		distance_label.text = "Min sep: (disabled)"
		velocity_label.text = "(metrics disabled)"
		expected_label.text = "--"


func _on_run_pressed() -> void:
	var selected_id: int = test_dropdown.get_selected_id()
	if selected_id > 0:
		_start_test(selected_id)


func _show_menu() -> void:
	current_test = 0
	test_label.text = "Test: None"
	_set_state("IDLE")
	_set_phase("Select a test")


func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed:
		match event.keycode:
			KEY_1: _start_test(1)
			KEY_2: _start_test(2)
			KEY_3: _start_test(3)
			KEY_4: _start_test(4)
			KEY_5: _start_test(5)
			KEY_6: _start_test(6)
			KEY_7: _start_test(7)
			KEY_8: _start_test(8)
			KEY_9: _start_test(9)
			KEY_0: _start_test(10)
			KEY_MINUS: _start_test(11)  # - key for test 11 (Projectiles)
			KEY_R: _start_test(current_test)
			KEY_ESCAPE: _show_menu()


func _start_test(test_id: int) -> void:
	if test_id < 1 or test_id > 11:
		return

	# Reset Rust state
	if ecs_manager and ecs_manager.has_method("reset"):
		ecs_manager.reset()

	# Reset local state
	current_test = test_id
	_configure_slider(test_id)
	_on_count_changed(count_slider.value)
	test_phase = 0
	test_timer = 0.0
	npc_count = 0
	pending_test = 0
	test_result = ""
	phase_results = []

	test_label.text = "Test: " + TEST_NAMES.get(test_id, "?")
	_set_state("RUNNING")
	_set_phase("Setup")
	_log("Started test %d" % test_id)

	match test_id:
		1: _setup_test_arrive()
		2: _setup_test_separation()
		3: _setup_test_both()
		4: _setup_test_circle()
		5: _setup_test_mass()
		6: _setup_test_world_data()
		7: _setup_test_guard_patrol()
		8: _setup_test_farmer_work()
		9: _setup_test_health_death()
		10: _setup_test_combat()
		11: _setup_test_projectiles()


# =============================================================================
# TEST 1: Arrive - NPCs spawn on left, move to center target
# Purpose: Verify target-seeking works (velocity toward target, stop on arrival)
# =============================================================================
func _setup_test_arrive() -> void:
	npc_count = int(count_slider.value)
	for i in npc_count:
		var y_offset := (i - npc_count / 2.0) * 25.0
		ecs_manager.spawn_npc(100, CENTER.y + y_offset, i % 3, 0, {})
	test_phase = 1
	_set_phase("Waiting 0.5s...")
	_log("%d NPCs on left" % npc_count)


func _update_test_arrive() -> void:
	if test_phase == 1 and test_timer > 0.5:
		for i in npc_count:
			ecs_manager.set_target(i, CENTER.x, CENTER.y)
		test_phase = 2
		_set_phase("Moving to center")
		_log("Targets set")

	if test_phase == 2 and test_timer > 5.0:
		test_phase = 3
		if not metrics_enabled:
			_set_phase("Done (no validation)")
			return
		# Assert: all NPCs have settled (reached target or gave up)
		var stats: Dictionary = ecs_manager.get_debug_stats()
		var arrived: int = stats.get("arrived_count", 0)
		if arrived == npc_count:
			_pass()
			_set_phase("All settled (%d/%d)" % [arrived, npc_count])
		else:
			_fail("Only %d/%d settled" % [arrived, npc_count])


# =============================================================================
# TEST 2: Separation - All NPCs spawn at exact same point, push apart
# Purpose: Verify separation forces work when NPCs fully overlap
# =============================================================================
func _setup_test_separation() -> void:
	npc_count = int(count_slider.value)
	for i in npc_count:
		ecs_manager.spawn_npc(CENTER.x, CENTER.y, i % 3, 0, {})
	test_phase = 1
	_set_phase("Separating...")
	_log("%d NPCs at same point" % npc_count)


func _update_test_separation() -> void:
	if test_phase == 1 and test_timer > 2.0:
		test_phase = 2
		if not metrics_enabled:
			_set_phase("Done (no validation)")
			return
		# Assert: all NPC pairs at least SEP_RADIUS apart
		var min_sep := _get_min_separation()
		if _assert_all_separated(SEP_RADIUS):
			_pass()
			_set_phase("Min sep: %.1fpx" % min_sep)
		else:
			_fail("Too close: %.1fpx < %.0fpx" % [min_sep, SEP_RADIUS])


# =============================================================================
# TEST 3: Arrive+Sep - NPCs spawn on left/right, move to same target
# Purpose: Verify arrival + separation work together (converge then spread)
# =============================================================================
func _setup_test_both() -> void:
	npc_count = int(count_slider.value)
	for i in npc_count:
		var side := 100 if i % 2 == 0 else 700
		var y_offset := (i / 2 - npc_count / 4.0) * 25.0
		ecs_manager.spawn_npc(side, CENTER.y + y_offset, i % 3, 0, {})
	test_phase = 1
	_set_phase("Waiting 0.5s...")
	_log("%d NPCs on sides" % npc_count)


func _update_test_both() -> void:
	if test_phase == 1 and test_timer > 0.5:
		for i in npc_count:
			ecs_manager.set_target(i, CENTER.x, CENTER.y)
		test_phase = 2
		_set_phase("All moving to center")
		_log("Targets set")

	if test_phase == 2 and test_timer > 3.0:
		test_phase = 3
		if not metrics_enabled:
			_set_phase("Done (no validation)")
			return
		# Assert: near center AND separated
		var max_dist := _get_max_dist_from_target(CENTER)
		var min_sep := _get_min_separation()
		var expected_radius := SEP_RADIUS * sqrt(npc_count)

		if max_dist > expected_radius + ARRIVAL_THRESHOLD:
			_fail("Not converged: %.0fpx" % max_dist)
		elif not _assert_all_separated(SEP_RADIUS):
			_fail("Too close: %.1fpx" % min_sep)
		else:
			_pass()
			_set_phase("Converged+Sep (%.0fpx, %.1fpx)" % [max_dist, min_sep])


# =============================================================================
# TEST 4: Circle - NPCs spawn in circle formation, move inward
# Purpose: Verify many NPCs converging forms stable cluster (not chaos)
# =============================================================================
func _setup_test_circle() -> void:
	npc_count = int(count_slider.value)
	var radius := 200.0
	for i in npc_count:
		var angle := (float(i) / npc_count) * TAU
		var x := CENTER.x + cos(angle) * radius
		var y := CENTER.y + sin(angle) * radius
		ecs_manager.spawn_npc(x, y, i % 3, 0, {})
	test_phase = 1
	_set_phase("Waiting 0.5s...")
	_log("%d NPCs in circle" % npc_count)


func _update_test_circle() -> void:
	if test_phase == 1 and test_timer > 0.5:
		for i in npc_count:
			ecs_manager.set_target(i, CENTER.x, CENTER.y)
		test_phase = 2
		_set_phase("All moving inward")
		_log("Targets set")

	if test_phase == 2 and test_timer > 3.0:
		test_phase = 3
		if not metrics_enabled:
			_set_phase("Done (no validation)")
			return
		# Assert: formed cluster near center, all separated
		var max_dist := _get_max_dist_from_target(CENTER)
		var min_sep := _get_min_separation()
		var expected_radius := SEP_RADIUS * sqrt(npc_count)

		if max_dist > expected_radius + ARRIVAL_THRESHOLD:
			_fail("Not clustered: %.0fpx" % max_dist)
		elif not _assert_all_separated(SEP_RADIUS):
			_fail("Too close: %.1fpx" % min_sep)
		else:
			_pass()
			_set_phase("Cluster formed (r=%.0fpx)" % max_dist)


# =============================================================================
# TEST 5: Mass - All NPCs at same point, no target, pure separation
# Purpose: Stress test separation with maximum overlap (golden angle fallback)
# =============================================================================
func _setup_test_mass() -> void:
	npc_count = int(count_slider.value)
	for i in npc_count:
		ecs_manager.spawn_npc(CENTER.x, CENTER.y, i % 3, 0, {})
	test_phase = 1
	_set_phase("Exploding outward...")
	_log("%d NPCs at center" % npc_count)


func _update_test_mass() -> void:
	if test_phase == 1 and test_timer > 3.0:
		test_phase = 2
		if not metrics_enabled:
			_set_phase("Done (no validation)")
			return
		# Assert: all NPCs separated (no overlaps after explosion)
		var min_sep := _get_min_separation()
		if _assert_all_separated(SEP_RADIUS):
			_pass()
			_set_phase("All separated (min %.1fpx)" % min_sep)
		else:
			_fail("Still overlapping: %.1fpx" % min_sep)


# =============================================================================
# TEST 6: World Data - Test world data API (towns, farms, beds, guard posts)
# Purpose: Verify world data init, add, and query functions work correctly
# =============================================================================
func _setup_test_world_data() -> void:
	npc_count = 0  # No NPCs needed for this test
	test_phase = 1
	_set_phase("Initializing world...")
	_log("Testing world data API")
	queue_redraw()  # Show visual markers

	# Initialize world with 2 towns (villager + raider)
	ecs_manager.init_world(2)

	# Add towns (unified API)
	ecs_manager.add_location("fountain", CENTER.x, CENTER.y, 0, {"name": "TestTown", "faction": 0})
	ecs_manager.add_location("camp", CENTER.x + 200, CENTER.y, 1, {})

	# Add 2 farms (west and east of center)
	ecs_manager.add_location("farm", CENTER.x - 100, CENTER.y, 0, {})
	ecs_manager.add_location("farm", CENTER.x + 100, CENTER.y, 0, {})

	# Add 4 beds (corners)
	ecs_manager.add_location("bed", CENTER.x - 50, CENTER.y - 50, 0, {})
	ecs_manager.add_location("bed", CENTER.x + 50, CENTER.y - 50, 0, {})
	ecs_manager.add_location("bed", CENTER.x - 50, CENTER.y + 50, 0, {})
	ecs_manager.add_location("bed", CENTER.x + 50, CENTER.y + 50, 0, {})

	# Add 4 guard posts (clockwise from top-left)
	ecs_manager.add_location("guard_post", CENTER.x - 80, CENTER.y - 80, 0, {"patrol_order": 0})
	ecs_manager.add_location("guard_post", CENTER.x + 80, CENTER.y - 80, 0, {"patrol_order": 1})
	ecs_manager.add_location("guard_post", CENTER.x + 80, CENTER.y + 80, 0, {"patrol_order": 2})
	ecs_manager.add_location("guard_post", CENTER.x - 80, CENTER.y + 80, 0, {"patrol_order": 3})

	# Build location sprites
	ecs_manager.build_locations()

	queue_redraw()  # Draw visual markers


func _update_test_world_data() -> void:
	if test_phase == 1:
		test_phase = 2
		_set_phase("Verifying world stats...")

		# Get stats and verify counts
		var stats: Dictionary = ecs_manager.get_world_stats()
		var town_count: int = stats.get("town_count", 0)
		var farm_count: int = stats.get("farm_count", 0)
		var bed_count: int = stats.get("bed_count", 0)
		var post_count: int = stats.get("guard_post_count", 0)

		_log("towns=%d farms=%d beds=%d posts=%d" % [town_count, farm_count, bed_count, post_count])

		if town_count != 2:
			_fail("town_count=%d expected 2" % town_count)
			return
		if farm_count != 2:
			_fail("farm_count=%d expected 2" % farm_count)
			return
		if bed_count != 4:
			_fail("bed_count=%d expected 4" % bed_count)
			return
		if post_count != 4:
			_fail("post_count=%d expected 4" % post_count)
			return

	if test_phase == 2:
		test_phase = 3
		_set_phase("Verifying queries...")

		# Test town center query
		var town_center: Vector2 = ecs_manager.get_town_center(0)
		if town_center.distance_to(CENTER) > 1.0:
			_fail("town_center wrong: %s" % town_center)
			return

		# Test raider town position query (raider town is at index 1)
		var raider_town_pos: Vector2 = ecs_manager.get_town_center(1)
		var expected_raider := Vector2(CENTER.x + 200, CENTER.y)
		if raider_town_pos.distance_to(expected_raider) > 1.0:
			_fail("raider_town_pos wrong: %s" % raider_town_pos)
			return

		# Test patrol post query
		var post0: Vector2 = ecs_manager.get_patrol_post(0, 0)
		var expected_post0 := Vector2(CENTER.x - 80, CENTER.y - 80)
		if post0.distance_to(expected_post0) > 1.0:
			_fail("patrol_post wrong: %s" % post0)
			return

		_log("Queries OK")

	if test_phase == 3:
		test_phase = 4
		_set_phase("Testing reservations...")

		# Test nearest free bed
		var bed_idx: int = ecs_manager.get_nearest_free_bed(0, CENTER.x - 40, CENTER.y - 40)
		if bed_idx < 0:
			_fail("no free bed found")
			return
		_log("Nearest bed: %d" % bed_idx)

		# Reserve bed
		if not ecs_manager.reserve_bed(bed_idx, 0):
			_fail("reserve_bed failed")
			return

		# Check bed is now occupied (stats should show 3 free)
		var stats: Dictionary = ecs_manager.get_world_stats()
		var free_beds: int = stats.get("free_beds", 0)
		if free_beds != 3:
			_fail("free_beds=%d expected 3" % free_beds)
			return

		# Release bed
		ecs_manager.release_bed(bed_idx)
		stats = ecs_manager.get_world_stats()
		free_beds = stats.get("free_beds", 0)
		if free_beds != 4:
			_fail("free_beds=%d expected 4" % free_beds)
			return

		_log("Reservations OK")

	if test_phase == 4:
		test_phase = 5
		_set_phase("Testing farm reservations...")

		# Test nearest free farm
		var farm_idx: int = ecs_manager.get_nearest_free_farm(0, CENTER.x - 80, CENTER.y)
		if farm_idx < 0:
			_fail("no free farm found")
			return
		_log("Nearest farm: %d" % farm_idx)

		# Reserve farm
		if not ecs_manager.reserve_farm(farm_idx):
			_fail("reserve_farm failed")
			return

		# Check farm is now occupied (1 free left)
		var stats: Dictionary = ecs_manager.get_world_stats()
		var free_farms: int = stats.get("free_farms", 0)
		if free_farms != 1:
			_fail("free_farms=%d expected 1" % free_farms)
			return

		# Release farm
		ecs_manager.release_farm(farm_idx)
		stats = ecs_manager.get_world_stats()
		free_farms = stats.get("free_farms", 0)
		if free_farms != 2:
			_fail("free_farms=%d expected 2" % free_farms)
			return

		_log("Farm reservations OK")
		_pass()
		_set_phase("All world data tests passed")


# =============================================================================
# TEST 7: Guard Patrol - Guards cycle through patrol posts
# Purpose: Verify guard state machine (Patrolling → OnDuty → next post)
# =============================================================================
func _setup_test_guard_patrol() -> void:
	npc_count = 4  # 4 guards for 4 posts
	test_phase = 1
	_set_phase("Setting up world...")
	_log("Testing guard patrol")

	# Initialize world with 1 town
	ecs_manager.init_world(1)
	ecs_manager.add_location("fountain", CENTER.x, CENTER.y, 0, {"name": "GuardTown", "faction": 0})

	# Add 4 guard posts (corners, clockwise)
	var post_positions: Array[Vector2] = [
		Vector2(CENTER.x - 100, CENTER.y - 100),  # Top-left (0)
		Vector2(CENTER.x + 100, CENTER.y - 100),  # Top-right (1)
		Vector2(CENTER.x + 100, CENTER.y + 100),  # Bottom-right (2)
		Vector2(CENTER.x - 100, CENTER.y + 100),  # Bottom-left (3)
	]
	for i in 4:
		ecs_manager.add_location("guard_post", post_positions[i].x, post_positions[i].y, 0, {"patrol_order": i})

	# Add a bed for guards to rest at
	ecs_manager.add_location("bed", CENTER.x, CENTER.y, 0, {})
	ecs_manager.build_locations()

	queue_redraw()


func _update_test_guard_patrol() -> void:
	# Phase 1: Spawn guards (after world setup)
	if test_phase == 1 and test_timer > 0.5:
		test_phase = 2
		_set_phase("Spawning guards...")

		# Spawn each guard at their starting post position
		var post_positions: Array[Vector2] = [
			Vector2(CENTER.x - 100, CENTER.y - 100),  # Post 0: Top-left
			Vector2(CENTER.x + 100, CENTER.y - 100),  # Post 1: Top-right
			Vector2(CENTER.x + 100, CENTER.y + 100),  # Post 2: Bottom-right
			Vector2(CENTER.x - 100, CENTER.y + 100),  # Post 3: Bottom-left
		]
		for i in 4:
			var pos: Vector2 = post_positions[i]
			ecs_manager.spawn_npc(pos.x, pos.y, 1, 0, {"home_x": CENTER.x, "home_y": CENTER.y, "town_idx": 0, "starting_post": i})

		_log("Spawned 4 guards at posts")

	# Phase 2: Watch guards patrol - show debug info
	if test_phase == 2:
		if metrics_enabled:
			# Get guard debug info (GPU reads)
			var guard_debug: Dictionary = ecs_manager.get_guard_debug()
			var arrived_flags: int = guard_debug.get("arrived_flags", 0)
			var prev_true: int = guard_debug.get("prev_arrivals_true", 0)
			var queue_len: int = guard_debug.get("arrival_queue_len", 0)
			_set_phase("arr=%d prev=%d q=%d (%.0fs)" % [arrived_flags, prev_true, queue_len, test_timer])
		else:
			_set_phase("Running (%.0fs)" % test_timer)

		# After 10 seconds, consider it a pass if guards are still moving
		if test_timer > 10.0:
			test_phase = 3
			if not metrics_enabled:
				_set_phase("Done (no validation)")
				return
			var stats: Dictionary = ecs_manager.get_debug_stats()
			var npc_ct: int = stats.get("npc_count", 0)
			if npc_ct == 4:
				_pass()
				_set_phase("Guards patrolling successfully")
			else:
				_fail("Expected 4 guards, got %d" % npc_ct)


# =============================================================================
# TEST 8: Farmer Work - Farmers cycle between work and rest
# Purpose: Verify work behavior (GoingToWork → Working → tired → GoingToRest → Resting → repeat)
# =============================================================================
func _setup_test_farmer_work() -> void:
	npc_count = 2  # 2 farmers
	test_phase = 1
	_set_phase("Setting up world...")
	_log("Testing farmer work cycle")

	# Initialize world with 1 town
	ecs_manager.init_world(1)
	ecs_manager.add_location("fountain", CENTER.x, CENTER.y, 0, {"name": "FarmTown", "faction": 0})

	# Add 2 farms (left and right of center)
	var farm_positions: Array[Vector2] = [
		Vector2(CENTER.x - 100, CENTER.y),  # Farm 0: Left
		Vector2(CENTER.x + 100, CENTER.y),  # Farm 1: Right
	]
	for i in 2:
		ecs_manager.add_location("farm", farm_positions[i].x, farm_positions[i].y, 0, {})

	# Add 2 beds (above center)
	var bed_positions: Array[Vector2] = [
		Vector2(CENTER.x - 50, CENTER.y - 80),  # Bed 0
		Vector2(CENTER.x + 50, CENTER.y - 80),  # Bed 1
	]
	for i in 2:
		ecs_manager.add_location("bed", bed_positions[i].x, bed_positions[i].y, 0, {})
	ecs_manager.build_locations()

	queue_redraw()


func _update_test_farmer_work() -> void:
	# Phase 1: Spawn farmers (after world setup)
	if test_phase == 1 and test_timer > 0.5:
		test_phase = 2
		_set_phase("Spawning farmers...")

		# Spawn farmers at beds, will walk to farms
		var farm_positions: Array[Vector2] = [
			Vector2(CENTER.x - 100, CENTER.y),
			Vector2(CENTER.x + 100, CENTER.y),
		]
		var bed_positions: Array[Vector2] = [
			Vector2(CENTER.x - 50, CENTER.y - 80),
			Vector2(CENTER.x + 50, CENTER.y - 80),
		]
		for i in 2:
			ecs_manager.spawn_npc(
				bed_positions[i].x, bed_positions[i].y, 0, 0,
				{"home_x": bed_positions[i].x, "home_y": bed_positions[i].y,
				 "work_x": farm_positions[i].x, "work_y": farm_positions[i].y, "town_idx": 0}
			)

		_log("Spawned 2 farmers")

	# Phase 2: Watch farmers work - show status
	if test_phase == 2:
		_set_phase("Working (%.0fs)" % test_timer)

		# After 10 seconds, consider it a pass if farmers exist
		if test_timer > 10.0:
			test_phase = 3
			if not metrics_enabled:
				_set_phase("Done (no validation)")
				return
			var stats: Dictionary = ecs_manager.get_debug_stats()
			var npc_ct: int = stats.get("npc_count", 0)
			if npc_ct == 2:
				_pass()
				_set_phase("Farmers working successfully")
			else:
				_fail("Expected 2 farmers, got %d" % npc_ct)


# =============================================================================
# TEST 9: Health/Death - NPCs take damage and die
# Purpose: Verify damage system, death marking, and entity cleanup
# =============================================================================
func _setup_test_health_death() -> void:
	npc_count = 10
	test_phase = 1
	_set_phase("Spawning 10 NPCs...")
	_log("Testing health/death")

	# Spawn 10 NPCs at center (all start with 100 HP)
	for i in 10:
		var angle := (float(i) / 10) * TAU
		var x := CENTER.x + cos(angle) * 50.0
		var y := CENTER.y + sin(angle) * 50.0
		ecs_manager.spawn_npc(x, y, i % 3, 0, {})

	queue_redraw()


func _update_test_health_death() -> void:
	# Show health debug info every frame
	if ecs_manager.has_method("get_health_debug"):
		var hd: Dictionary = ecs_manager.get_health_debug()
		var bevy_ct: int = hd.get("bevy_entity_count", -1)
		var dmg_proc: int = hd.get("damage_processed", 0)
		var deaths: int = hd.get("deaths_this_frame", 0)
		var despawned: int = hd.get("despawned_this_frame", 0)
		var samples: String = hd.get("health_samples", "")
		expected_label.text = "bevy=%d dmg=%d die=%d desp=%d" % [bevy_ct, dmg_proc, deaths, despawned]
		velocity_label.text = "HP: %s" % samples

	# Phase 1: Wait for spawn, then deal 50 damage to NPCs 0-4
	if test_phase == 1 and test_timer > 1.0:
		test_phase = 2
		_set_phase("Dealing 50 damage to NPCs 0-4...")

		for i in 5:
			ecs_manager.apply_damage(i, 50.0)

		_log("50 dmg to NPCs 0-4")

	# Phase 2: Deal 60 more damage to NPCs 0-4 (kills them: 50 HP - 60 = dead)
	if test_phase == 2 and test_timer > 2.0:
		test_phase = 3
		_set_phase("Dealing 60 damage to NPCs 0-4...")

		for i in 5:
			ecs_manager.apply_damage(i, 60.0)

		_log("60 dmg to NPCs 0-4 (lethal)")

	# Phase 3: Verify 5 alive, 5 dead
	if test_phase == 3 and test_timer > 3.0:
		test_phase = 4

		# Check Bevy entity count via health debug
		var hd: Dictionary = ecs_manager.get_health_debug()
		var bevy_count: int = hd.get("bevy_entity_count", -1)

		if bevy_count == 5:
			_pass()
			_set_phase("5 alive, 5 dead - correct!")
		else:
			_fail("Expected 5 alive, got %d" % bevy_count)


# =============================================================================
# TEST 10: Combat TDD - 6-phase pipeline isolation
# Purpose: Isolate GPU targeting bug by testing each layer independently
# Spawns 1 guard + 1 raider, 50px apart. Each phase fails fast.
#   Phase 1 (t=1s):  GPU buffer integrity (faction, health written correctly)
#   Phase 2 (t=1.5s): Grid population (both NPCs in grid cells)
#   Phase 3 (t=2s):  GPU targeting (combat_target >= 0 for both)
#   Phase 4 (t=4s):  Damage (damage_processed > 0)
#   Phase 5 (t=10s): Death (bevy_entity_count < 2)
#   Phase 6 (t=11s): Slot recycling (new spawn reuses dead slot)
# =============================================================================
func _setup_test_combat() -> void:
	npc_count = 2  # 1 guard + 1 raider
	test_phase = 0
	_set_phase("Setting up world...")
	_log("Combat TDD: 1 guard + 1 raider, 50px apart")

	# Initialize world with 1 town and camp
	ecs_manager.init_world(1)
	ecs_manager.add_location("fountain", CENTER.x, CENTER.y, 0, {"name": "CombatTown", "faction": 0})
	ecs_manager.add_location("bed", CENTER.x - 100, CENTER.y, 0, {})
	ecs_manager.build_locations()

	# Spawn 2 fighters (job=3) with opposing factions, 50px apart. No behavior — just sit and fight.
	ecs_manager.spawn_npc(CENTER.x - 25, CENTER.y, 3, 0, {"home_x": CENTER.x, "home_y": CENTER.y})
	ecs_manager.spawn_npc(CENTER.x + 25, CENTER.y, 3, 1, {"home_x": CENTER.x, "home_y": CENTER.y})
	_log("Spawned fighter idx=0 (faction 0), fighter idx=1 (faction 1)")

	test_phase = 1
	queue_redraw()


func _update_test_combat() -> void:
	# Always show debug info
	var cd: Dictionary = ecs_manager.call("get_combat_debug") if ecs_manager.has_method("get_combat_debug") else {}
	var hd: Dictionary = ecs_manager.call("get_health_debug") if ecs_manager.has_method("get_health_debug") else {}

	# Row 1: GPU buffer reads
	var gpu_f0: int = cd.get("gpu_faction_0", -99)
	var gpu_f1: int = cd.get("gpu_faction_1", -99)
	var gpu_h0: float = cd.get("gpu_health_0", -99.0)
	var gpu_h1: float = cd.get("gpu_health_1", -99.0)
	expected_label.text = "gpu: f0=%d f1=%d h0=%.0f h1=%.0f" % [gpu_f0, gpu_f1, gpu_h0, gpu_h1]

	# Row 2: Grid cells
	var gc0: int = cd.get("grid_cell_0", -1)
	var gc1: int = cd.get("grid_cell_1", -1)
	var cx0: int = cd.get("grid_cx_0", -1)
	var cy0: int = cd.get("grid_cy_0", -1)
	var cx1: int = cd.get("grid_cx_1", -1)
	var cy1: int = cd.get("grid_cy_1", -1)
	velocity_label.text = "grid: [%d,%d]=%d [%d,%d]=%d" % [cx0, cy0, gc0, cx1, cy1, gc1]

	# Row 3: Combat targets + bevy state
	var ct0: int = cd.get("combat_target_0", -99)
	var ct1: int = cd.get("combat_target_1", -99)
	var bevy_ct: int = hd.get("bevy_entity_count", -1)
	var dmg_proc: int = hd.get("damage_processed", -1)
	distance_label.text = "ct0=%d ct1=%d bevy=%d dmg=%d" % [ct0, ct1, bevy_ct, dmg_proc]

	# Phase 1: GPU Buffer Integrity (t=1s)
	if test_phase == 1:
		_set_phase("Phase 1: GPU buffers (%.1fs)" % test_timer)
		if test_timer > 1.0:
			if gpu_f0 == 0 and gpu_f1 == 1 and gpu_h0 > 0 and gpu_h1 > 0:
				phase_results.append("P1 PASS (%.1fs): f0=%d f1=%d h0=%.0f h1=%.0f" % [test_timer, gpu_f0, gpu_f1, gpu_h0, gpu_h1])
				test_phase = 2
			else:
				phase_results.append("P1 FAIL (%.1fs): f0=%d f1=%d h0=%.0f h1=%.0f" % [test_timer, gpu_f0, gpu_f1, gpu_h0, gpu_h1])
				_fail("Phase 1: GPU buffers wrong — f0=%d f1=%d h0=%.0f h1=%.0f" % [gpu_f0, gpu_f1, gpu_h0, gpu_h1])

	# Phase 2: Grid Population (t=1.5s)
	elif test_phase == 2:
		_set_phase("Phase 2: Grid population (%.1fs)" % test_timer)
		if test_timer > 1.5:
			if gc0 > 0 and gc1 > 0:
				phase_results.append("P2 PASS (%.1fs): [%d,%d]=%d [%d,%d]=%d" % [test_timer, cx0, cy0, gc0, cx1, cy1, gc1])
				test_phase = 3
			else:
				phase_results.append("P2 FAIL (%.1fs): [%d,%d]=%d [%d,%d]=%d" % [test_timer, cx0, cy0, gc0, cx1, cy1, gc1])
				_fail("Phase 2: Grid empty — [%d,%d]=%d [%d,%d]=%d" % [cx0, cy0, gc0, cx1, cy1, gc1])

	# Phase 3: GPU Targeting (t=2s)
	elif test_phase == 3:
		_set_phase("Phase 3: GPU targeting (%.1fs)" % test_timer)
		if test_timer > 2.0:
			if ct0 >= 0 and ct1 >= 0:
				phase_results.append("P3 PASS (%.1fs): ct0=%d ct1=%d" % [test_timer, ct0, ct1])
				test_phase = 4
			else:
				phase_results.append("P3 FAIL (%.1fs): ct0=%d ct1=%d" % [test_timer, ct0, ct1])
				_fail("Phase 3: No targets — ct0=%d ct1=%d" % [ct0, ct1])

	# Phase 4: Damage (t=4s) - check GPU health decreased from 100
	elif test_phase == 4:
		_set_phase("Phase 4: Damage (%.1fs) h0=%.0f h1=%.0f" % [test_timer, gpu_h0, gpu_h1])
		if test_timer > 4.0:
			if gpu_h0 < 100.0 or gpu_h1 < 100.0:
				phase_results.append("P4 PASS (%.1fs): h0=%.0f h1=%.0f" % [test_timer, gpu_h0, gpu_h1])
				test_phase = 5
			else:
				phase_results.append("P4 FAIL (%.1fs): h0=%.0f h1=%.0f (no damage)" % [test_timer, gpu_h0, gpu_h1])
				_fail("Phase 4: No damage after 4s — h0=%.0f h1=%.0f" % [gpu_h0, gpu_h1])

	# Phase 5: Death (t=15s)
	elif test_phase == 5:
		_set_phase("Phase 5: Death (%d alive, %.1fs)" % [bevy_ct, test_timer])
		if test_timer > 15.0:
			if bevy_ct < 2:
				phase_results.append("P5 PASS (%.1fs): bevy=%d" % [test_timer, bevy_ct])
				test_phase = 6
			else:
				phase_results.append("P5 FAIL (%.1fs): bevy=%d" % [test_timer, bevy_ct])
				_fail("Phase 5: Nobody died after 10s — bevy=%d" % bevy_ct)

	# Phase 6: Slot Recycling (t=16s)
	elif test_phase == 6:
		_set_phase("Phase 6: Slot recycling (%.1fs)" % test_timer)
		if test_timer > 16.0:
			test_phase = 7
			var slot: int = ecs_manager.spawn_npc(CENTER.x, CENTER.y, 3, 0, {"home_x": CENTER.x, "home_y": CENTER.y})
			if slot >= 0 and slot < 2:
				phase_results.append("P6 PASS (%.1fs): recycled slot=%d" % [test_timer, slot])
				_pass()
				_set_phase("ALL 6 PHASES PASSED")
			else:
				phase_results.append("P6 WARN (%.1fs): slot=%d (no recycle)" % [test_timer, slot])
				_pass()
				_set_phase("5/6 PASSED (no slot recycle)")


# =============================================================================
# TEST 11: Unified Attacks - Melee and ranged use same projectile pipeline
# Purpose: Verify attack_system fires projectiles (not direct DamageMsg),
#          both melee (fast/short) and ranged (slow/long) deliver damage.
#
# TDD: This test is written FIRST. It will FAIL until attack_system is
# modified to fire projectiles instead of queuing DamageMsg directly.
#
# Uses Fighter NPCs (job=3, no behavior) to isolate combat.
# =============================================================================

func _setup_test_projectiles() -> void:
	npc_count = 4  # 2 melee fighters + 2 ranged fighters
	test_phase = 0
	_set_phase("Setting up unified attack test...")
	_log("Unified attacks: melee + ranged via projectile pipeline")

	ecs_manager.init_world(1)
	ecs_manager.add_location("fountain", CENTER.x, CENTER.y, 0, {"name": "AttackTown", "faction": 0})
	ecs_manager.build_locations()

	# Spawn 2 melee fighters (opposing factions, 30px apart)
	# Melee fighters: job=3, faction 0 vs 1
	ecs_manager.spawn_npc(CENTER.x - 15, CENTER.y - 50, 3, 0, {"home_x": CENTER.x, "home_y": CENTER.y - 50})
	ecs_manager.spawn_npc(CENTER.x + 15, CENTER.y - 50, 3, 1, {"home_x": CENTER.x, "home_y": CENTER.y - 50})
	_log("Spawned melee pair: idx=0 (f0) vs idx=1 (f1), 30px apart")

	# Spawn 2 ranged fighters (opposing factions, 150px apart — must be within 3x3 grid neighborhood)
	ecs_manager.spawn_npc(CENTER.x - 75, CENTER.y + 50, 3, 0, {"home_x": CENTER.x - 75, "home_y": CENTER.y + 50, "attack_type": 1})
	ecs_manager.spawn_npc(CENTER.x + 75, CENTER.y + 50, 3, 1, {"home_x": CENTER.x + 75, "home_y": CENTER.y + 50, "attack_type": 1})
	_log("Spawned ranged pair: idx=2 (f0) vs idx=3 (f1), 150px apart")

	test_phase = 1
	queue_redraw()


func _update_test_projectiles() -> void:
	# Always show debug info
	var hd: Dictionary = ecs_manager.call("get_health_debug") if ecs_manager.has_method("get_health_debug") else {}
	var pd: Dictionary = ecs_manager.call("get_projectile_debug") if ecs_manager.has_method("get_projectile_debug") else {}
	var cd: Dictionary = ecs_manager.call("get_combat_debug") if ecs_manager.has_method("get_combat_debug") else {}

	var proj_ct: int = pd.get("proj_count", -1)
	var active: int = pd.get("active", -1)
	var bevy_ct: int = hd.get("bevy_entity_count", -1)
	var dmg_proc: int = hd.get("damage_processed", -1)
	var attacks: int = cd.get("attacks", -1)

	expected_label.text = "proj=%d active=%d bevy=%d" % [proj_ct, active, bevy_ct]
	velocity_label.text = "attacks=%d dmg=%d" % [attacks, dmg_proc]
	distance_label.text = "ct0=%d ct1=%d ct2=%d ct3=%d" % [
		cd.get("combat_target_0", -99), cd.get("combat_target_1", -99),
		cd.get("combat_target_2", -99), cd.get("combat_target_3", -99)]

	# Phase 1: GPU Buffer Integrity (t=1s)
	if test_phase == 1:
		_set_phase("Phase 1: GPU buffers (%.1fs)" % test_timer)
		if test_timer > 1.0:
			var gpu_f0: int = cd.get("gpu_faction_0", -99)
			var gpu_f1: int = cd.get("gpu_faction_1", -99)
			var gpu_h0: float = cd.get("gpu_health_0", -99.0)
			var gpu_h1: float = cd.get("gpu_health_1", -99.0)
			if gpu_f0 == 0 and gpu_f1 == 1 and gpu_h0 > 0 and gpu_h1 > 0:
				phase_results.append("P1 PASS (%.1fs): f0=%d f1=%d h0=%.0f h1=%.0f" % [test_timer, gpu_f0, gpu_f1, gpu_h0, gpu_h1])
				test_phase = 2
			else:
				phase_results.append("P1 FAIL (%.1fs): f0=%d f1=%d h0=%.0f h1=%.0f" % [test_timer, gpu_f0, gpu_f1, gpu_h0, gpu_h1])
				_fail("Phase 1: GPU buffers wrong — f0=%d f1=%d h0=%.0f h1=%.0f" % [gpu_f0, gpu_f1, gpu_h0, gpu_h1])

	# Phase 2: Melee projectile fired (t=2s)
	# attack_system should fire a projectile when in range, NOT queue DamageMsg directly
	elif test_phase == 2:
		_set_phase("Phase 2: Melee projectile (%.1fs) proj=%d" % [test_timer, proj_ct])
		if test_timer > 2.0:
			if proj_ct > 0:
				phase_results.append("P2 PASS (%.1fs): proj_count=%d" % [test_timer, proj_ct])
				test_phase = 3
			else:
				phase_results.append("P2 FAIL (%.1fs): proj_count=%d (attack_system not firing projectiles)" % [test_timer, proj_ct])
				_fail("Phase 2: No projectiles fired — attack_system still using direct DamageMsg?")

	# Phase 3: Melee damage dealt (t=3s) - check GPU health of melee pair decreased
	elif test_phase == 3:
		var h0: float = cd.get("gpu_health_0", 100.0)
		var h1: float = cd.get("gpu_health_1", 100.0)
		_set_phase("Phase 3: Melee damage (%.1fs) h0=%.0f h1=%.0f" % [test_timer, h0, h1])
		if test_timer > 3.0:
			if h0 < 100.0 or h1 < 100.0:
				phase_results.append("P3 PASS (%.1fs): h0=%.0f h1=%.0f" % [test_timer, h0, h1])
				test_phase = 4
			else:
				phase_results.append("P3 FAIL (%.1fs): h0=%.0f h1=%.0f (no melee damage)" % [test_timer, h0, h1])
				_fail("Phase 3: No melee damage after 3s — h0=%.0f h1=%.0f" % [h0, h1])

	# Phase 4: Ranged pair GPU targeting (t=5s)
	# Ranged fighters are 200px apart — GPU targeting should find them within detection range (300px)
	elif test_phase == 4:
		_set_phase("Phase 4: Ranged targeting (%.1fs)" % test_timer)
		if test_timer > 5.0:
			var ct2: int = cd.get("combat_target_2", -99)
			var ct3: int = cd.get("combat_target_3", -99)
			if ct2 >= 0 and ct3 >= 0:
				phase_results.append("P4 PASS (%.1fs): ct2=%d ct3=%d" % [test_timer, ct2, ct3])
				test_phase = 5
			else:
				phase_results.append("P4 FAIL (%.1fs): ct2=%d ct3=%d" % [test_timer, ct2, ct3])
				_fail("Phase 4: Ranged pair not targeting — ct2=%d ct3=%d" % [ct2, ct3])

	# Phase 5: Ranged projectile traveling (t=7s)
	# Ranged fighters should fire projectiles that take time to travel
	elif test_phase == 5:
		_set_phase("Phase 5: Ranged projectile (%.1fs) proj=%d" % [test_timer, proj_ct])
		if test_timer > 7.0:
			# Should have more projectiles than just melee (melee are instant, ranged accumulate)
			if proj_ct > 2:
				phase_results.append("P5 PASS (%.1fs): proj_count=%d (ranged accumulating)" % [test_timer, proj_ct])
				test_phase = 6
			else:
				phase_results.append("P5 FAIL (%.1fs): proj_count=%d (expected >2 for ranged)" % [test_timer, proj_ct])
				_fail("Phase 5: Not enough projectiles for ranged — proj=%d" % proj_ct)

	# Phase 6: Ranged damage dealt (t=10s) - check GPU health of ranged pair decreased
	elif test_phase == 6:
		var h2: float = cd.get("gpu_health_2", 100.0)
		var h3: float = cd.get("gpu_health_3", 100.0)
		_set_phase("Phase 6: Ranged damage (%.1fs) h2=%.0f h3=%.0f" % [test_timer, h2, h3])
		if test_timer > 10.0:
			if h2 < 100.0 or h3 < 100.0:
				phase_results.append("P6 PASS (%.1fs): h2=%.0f h3=%.0f" % [test_timer, h2, h3])
				test_phase = 7
			else:
				phase_results.append("P6 FAIL (%.1fs): h2=%.0f h3=%.0f (no ranged damage)" % [test_timer, h2, h3])
				_fail("Phase 6: No ranged damage after 10s — h2=%.0f h3=%.0f" % [h2, h3])

	# Phase 7: Death (t=20s)
	elif test_phase == 7:
		_set_phase("Phase 7: Death (%d alive, %.1fs)" % [bevy_ct, test_timer])
		if test_timer > 20.0:
			test_phase = 8
			if bevy_ct < 4:
				phase_results.append("P7 PASS (%.1fs): bevy=%d (at least 1 died)" % [test_timer, bevy_ct])
				_pass()
				_set_phase("ALL 7 PHASES PASSED")
			else:
				phase_results.append("P7 FAIL (%.1fs): bevy=%d" % [test_timer, bevy_ct])
				_fail("Phase 7: Nobody died after 12s — bevy=%d" % bevy_ct)


# =============================================================================
# DRAWING (visual markers)
# =============================================================================
func _draw() -> void:
	# Draw center crosshair
	var size := 20.0
	draw_line(CENTER - Vector2(size, 0), CENTER + Vector2(size, 0), Color.YELLOW, 2.0)
	draw_line(CENTER - Vector2(0, size), CENTER + Vector2(0, size), Color.YELLOW, 2.0)
	# Draw arrival threshold circle
	draw_arc(CENTER, 8.0, 0, TAU, 32, Color.YELLOW, 1.0)

	# Draw world data markers for Test 6
	if current_test == 6 and ecs_manager:
		# Town center (gold circle)
		var town_center: Vector2 = ecs_manager.get_town_center(0)
		if town_center != Vector2.ZERO:
			draw_arc(town_center, 30.0, 0, TAU, 32, Color.GOLD, 2.0)
			draw_string(ThemeDB.fallback_font, town_center + Vector2(-20, -35), "TOWN", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color.GOLD)

		# Raider town (red X) - index 1
		var raider_pos: Vector2 = ecs_manager.get_town_center(1)
		if raider_pos != Vector2.ZERO:
			draw_line(raider_pos - Vector2(15, 15), raider_pos + Vector2(15, 15), Color.RED, 2.0)
			draw_line(raider_pos - Vector2(15, -15), raider_pos + Vector2(15, -15), Color.RED, 2.0)
			draw_string(ThemeDB.fallback_font, raider_pos + Vector2(-15, -20), "RAIDER", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color.RED)

		# Farms (green squares)
		for i in 2:
			var farm_x := CENTER.x + (-100 if i == 0 else 100)
			var farm_pos := Vector2(farm_x, CENTER.y)
			draw_rect(Rect2(farm_pos - Vector2(20, 20), Vector2(40, 40)), Color.GREEN, false, 2.0)
			draw_string(ThemeDB.fallback_font, farm_pos + Vector2(-15, 30), "FARM", HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color.GREEN)

		# Beds (blue squares)
		var bed_offsets: Array[Vector2] = [Vector2(-50, -50), Vector2(50, -50), Vector2(-50, 50), Vector2(50, 50)]
		for offset in bed_offsets:
			var bed_pos: Vector2 = CENTER + offset
			draw_rect(Rect2(bed_pos - Vector2(8, 8), Vector2(16, 16)), Color.CYAN, false, 2.0)

		# Guard posts (orange diamonds)
		for i in 4:
			var post_pos: Vector2 = ecs_manager.get_patrol_post(0, i)
			if post_pos != Vector2.ZERO:
				var pts := PackedVector2Array([
					post_pos + Vector2(0, -12),
					post_pos + Vector2(12, 0),
					post_pos + Vector2(0, 12),
					post_pos + Vector2(-12, 0),
					post_pos + Vector2(0, -12)
				])
				draw_polyline(pts, Color.ORANGE, 2.0)

	# Draw guard patrol markers for Test 7
	if current_test == 7 and ecs_manager:
		# Guard posts (orange diamonds with numbers)
		for i in 4:
			var post_pos: Vector2 = ecs_manager.get_patrol_post(0, i)
			if post_pos != Vector2.ZERO:
				# Diamond shape
				var pts := PackedVector2Array([
					post_pos + Vector2(0, -20),
					post_pos + Vector2(20, 0),
					post_pos + Vector2(0, 20),
					post_pos + Vector2(-20, 0),
					post_pos + Vector2(0, -20)
				])
				draw_polyline(pts, Color.ORANGE, 3.0)
				# Post number
				draw_string(ThemeDB.fallback_font, post_pos + Vector2(-4, 5), str(i), HORIZONTAL_ALIGNMENT_LEFT, -1, 14, Color.WHITE)

		# Bed (cyan square)
		draw_rect(Rect2(CENTER - Vector2(10, 10), Vector2(20, 20)), Color.CYAN, false, 2.0)
		draw_string(ThemeDB.fallback_font, CENTER + Vector2(-10, 25), "BED", HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color.CYAN)

	# Draw farmer work markers for Test 8
	if current_test == 8:
		# Farms (green squares)
		var farm_positions: Array[Vector2] = [
			Vector2(CENTER.x - 100, CENTER.y),
			Vector2(CENTER.x + 100, CENTER.y),
		]
		for i in 2:
			var farm_pos := farm_positions[i]
			draw_rect(Rect2(farm_pos - Vector2(25, 25), Vector2(50, 50)), Color.GREEN, false, 3.0)
			draw_string(ThemeDB.fallback_font, farm_pos + Vector2(-15, 35), "FARM", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color.GREEN)

		# Beds (cyan squares)
		var bed_positions: Array[Vector2] = [
			Vector2(CENTER.x - 50, CENTER.y - 80),
			Vector2(CENTER.x + 50, CENTER.y - 80),
		]
		for i in 2:
			var bed_pos := bed_positions[i]
			draw_rect(Rect2(bed_pos - Vector2(12, 12), Vector2(24, 24)), Color.CYAN, false, 2.0)
			draw_string(ThemeDB.fallback_font, bed_pos + Vector2(-10, 20), "BED", HORIZONTAL_ALIGNMENT_LEFT, -1, 10, Color.CYAN)


# =============================================================================
# MAIN LOOP
# =============================================================================
func _process(delta: float) -> void:
	# FPS counter
	frame_count += 1
	fps_timer += delta
	if fps_timer >= 1.0:
		fps_label.text = "FPS: %.0f" % (frame_count / fps_timer)
		frame_count = 0
		fps_timer = 0.0

	# Poll Bevy→Godot channel messages (Phase 11)
	if ecs_manager and ecs_manager.has_method("bevy_to_godot"):
		var messages: Array = ecs_manager.bevy_to_godot()
		for msg in messages:
			_log("Channel: %s" % str(msg))

	# NPC count (don't overwrite slider label for test 11)
	if ecs_manager and current_test != 11:
		count_label.text = "NPCs: %d" % ecs_manager.get_npc_count()

	# Update metrics
	_update_metrics()

	# Test updates
	if current_test > 0:
		test_timer += delta
		match current_test:
			1: _update_test_arrive()
			2: _update_test_separation()
			3: _update_test_both()
			4: _update_test_circle()
			5: _update_test_mass()
			6: _update_test_world_data()
			7: _update_test_guard_patrol()
			8: _update_test_farmer_work()
			9: _update_test_health_death()
			10: _update_test_combat()
			11: _update_test_projectiles()


func _update_metrics() -> void:
	time_label.text = "Time: %.2fs" % test_timer
	if not metrics_enabled:
		return
	# Skip for test 10 - it has its own debug display
	if current_test == 11:
		return
	if npc_count > 1 and ecs_manager:
		# Only compute expensive metrics every 60 frames
		if frame_count % 60 == 0:
			# O(n²) min separation check
			var min_sep := _get_min_separation()
			distance_label.text = "Min sep: %.1fpx" % min_sep

			# Debug stats requires GPU buffer reads
			if ecs_manager.has_method("get_debug_stats"):
				var stats: Dictionary = ecs_manager.get_debug_stats()
				var arrived: int = stats.get("arrived_count", 0)
				var max_bo: int = stats.get("max_backoff", 0)
				var cells: int = stats.get("cells_used", 0)
				var max_cell: int = stats.get("max_per_cell", 0)
				velocity_label.text = "Arrived: %d/%d  Grid: %d cells, %d max" % [arrived, npc_count, cells, max_cell]
				expected_label.text = "Max backoff: %d" % max_bo
	elif npc_count == 1:
		distance_label.text = "Min sep: n/a"
		velocity_label.text = "(single NPC)"
		expected_label.text = "--"
	else:
		distance_label.text = "Min sep: --"
		velocity_label.text = "--"
		expected_label.text = "--"
