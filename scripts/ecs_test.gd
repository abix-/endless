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

# State
var frame_count := 0
var fps_timer := 0.0
var test_timer := 0.0
var current_test := 0
var test_phase := 0
var npc_count := 0
var is_running := false
var log_lines: Array[String] = []
var test_result := ""  # "PASS" or "FAIL: reason"

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
	6: "World Data"
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
	count_slider = vbox.get_node("CountSlider")
	count_slider.value_changed.connect(_on_count_changed)
	_on_count_changed(count_slider.value)  # Initialize label

	# Connect test buttons
	vbox.get_node("TestButtons/Test1").pressed.connect(_start_test.bind(1))
	vbox.get_node("TestButtons/Test2").pressed.connect(_start_test.bind(2))
	vbox.get_node("TestButtons/Test3").pressed.connect(_start_test.bind(3))
	vbox.get_node("TestButtons/Test4").pressed.connect(_start_test.bind(4))
	vbox.get_node("TestButtons/Test5").pressed.connect(_start_test.bind(5))
	vbox.get_node("TestButtons/Test6").pressed.connect(_start_test.bind(6))
	vbox.get_node("CopyButton").pressed.connect(_copy_debug_info)

	if ClassDB.class_exists("EcsNpcManager"):
		ecs_manager = ClassDB.instantiate("EcsNpcManager")
		add_child(ecs_manager)
		_log("EcsNpcManager created")

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
	print("[ECS Test] " + msg)
	log_lines.push_front(msg)
	if log_lines.size() > 3:
		log_lines.pop_back()
	log_label.text = "\n".join(log_lines)


func _copy_debug_info() -> void:
	var info := "Test: %s | Time: %.1fs | NPCs: %d\n" % [TEST_NAMES.get(current_test, "?"), test_timer, npc_count]
	info += state_label.text + "\n"
	info += phase_label.text + "\n"
	info += "Min sep: %.1fpx\n" % _get_min_separation() if npc_count > 1 else ""
	if ecs_manager and ecs_manager.has_method("get_debug_stats"):
		var stats: Dictionary = ecs_manager.get_debug_stats()
		info += "arrived: %d/%d\n" % [stats.get("arrived_count", 0), npc_count]
		info += "max_backoff: %d\n" % stats.get("max_backoff", 0)
		info += "grid_cells: %d, max_per_cell: %d\n" % [stats.get("cells_used", 0), stats.get("max_per_cell", 0)]
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


func _on_count_changed(value: float) -> void:
	count_label.text = "NPC Count: %d" % int(value)


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
			KEY_R: _start_test(current_test)
			KEY_ESCAPE: _show_menu()


func _start_test(test_id: int) -> void:
	if test_id < 1 or test_id > 6:
		return

	# Reset Rust state
	if ecs_manager and ecs_manager.has_method("reset"):
		ecs_manager.reset()

	# Reset local state
	current_test = test_id
	test_phase = 0
	test_timer = 0.0
	npc_count = 0
	pending_test = 0
	test_result = ""

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


# =============================================================================
# TEST 1: Arrive - NPCs spawn on left, move to center target
# Purpose: Verify target-seeking works (velocity toward target, stop on arrival)
# =============================================================================
func _setup_test_arrive() -> void:
	npc_count = int(count_slider.value)
	for i in npc_count:
		var y_offset := (i - npc_count / 2.0) * 25.0
		ecs_manager.spawn_npc(100, CENTER.y + y_offset, i % 3)
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
		ecs_manager.spawn_npc(CENTER.x, CENTER.y, i % 3)
	test_phase = 1
	_set_phase("Separating...")
	_log("%d NPCs at same point" % npc_count)


func _update_test_separation() -> void:
	if test_phase == 1 and test_timer > 2.0:
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
		ecs_manager.spawn_npc(side, CENTER.y + y_offset, i % 3)
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
		ecs_manager.spawn_npc(x, y, i % 3)
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
		ecs_manager.spawn_npc(CENTER.x, CENTER.y, i % 3)
	test_phase = 1
	_set_phase("Exploding outward...")
	_log("%d NPCs at center" % npc_count)


func _update_test_mass() -> void:
	if test_timer > 3.0:
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

	# Initialize world with 1 town
	ecs_manager.init_world(1)

	# Add town
	ecs_manager.add_town("TestTown", CENTER.x, CENTER.y, CENTER.x + 200, CENTER.y)

	# Add 2 farms (west and east of center)
	ecs_manager.add_farm(CENTER.x - 100, CENTER.y, 0)
	ecs_manager.add_farm(CENTER.x + 100, CENTER.y, 0)

	# Add 4 beds (corners)
	ecs_manager.add_bed(CENTER.x - 50, CENTER.y - 50, 0)
	ecs_manager.add_bed(CENTER.x + 50, CENTER.y - 50, 0)
	ecs_manager.add_bed(CENTER.x - 50, CENTER.y + 50, 0)
	ecs_manager.add_bed(CENTER.x + 50, CENTER.y + 50, 0)

	# Add 4 guard posts (clockwise from top-left)
	ecs_manager.add_guard_post(CENTER.x - 80, CENTER.y - 80, 0, 0)
	ecs_manager.add_guard_post(CENTER.x + 80, CENTER.y - 80, 0, 1)
	ecs_manager.add_guard_post(CENTER.x + 80, CENTER.y + 80, 0, 2)
	ecs_manager.add_guard_post(CENTER.x - 80, CENTER.y + 80, 0, 3)

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

		if town_count != 1:
			_fail("town_count=%d expected 1" % town_count)
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

		# Test camp position query
		var camp_pos: Vector2 = ecs_manager.get_camp_position(0)
		var expected_camp := Vector2(CENTER.x + 200, CENTER.y)
		if camp_pos.distance_to(expected_camp) > 1.0:
			_fail("camp_pos wrong: %s" % camp_pos)
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

		# Camp (red X)
		var camp_pos: Vector2 = ecs_manager.get_camp_position(0)
		if camp_pos != Vector2.ZERO:
			draw_line(camp_pos - Vector2(15, 15), camp_pos + Vector2(15, 15), Color.RED, 2.0)
			draw_line(camp_pos - Vector2(15, -15), camp_pos + Vector2(15, -15), Color.RED, 2.0)
			draw_string(ThemeDB.fallback_font, camp_pos + Vector2(-15, -20), "CAMP", HORIZONTAL_ALIGNMENT_LEFT, -1, 12, Color.RED)

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

	# NPC count
	if ecs_manager:
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


func _update_metrics() -> void:
	time_label.text = "Time: %.2fs" % test_timer
	if npc_count > 1 and ecs_manager:
		var min_sep := _get_min_separation()
		distance_label.text = "Min sep: %.1fpx" % min_sep

		# Get debug stats from Rust
		if ecs_manager.has_method("get_debug_stats"):
			var stats: Dictionary = ecs_manager.get_debug_stats()
			var arrived: int = stats.get("arrived_count", 0)
			var max_bo: int = stats.get("max_backoff", 0)
			var cells: int = stats.get("cells_used", 0)
			var max_cell: int = stats.get("max_per_cell", 0)
			velocity_label.text = "Arrived: %d/%d  Grid: %d cells, %d max" % [arrived, npc_count, cells, max_cell]
			expected_label.text = "Max backoff: %d" % max_bo
		else:
			velocity_label.text = "Pass if >= %.0fpx" % SEP_RADIUS
			expected_label.text = "Target: %dpx sep" % int(SEP_RADIUS)
	elif npc_count == 1:
		distance_label.text = "Min sep: n/a"
		velocity_label.text = "(single NPC)"
		expected_label.text = "--"
	else:
		distance_label.text = "Min sep: --"
		velocity_label.text = "--"
		expected_label.text = "--"
