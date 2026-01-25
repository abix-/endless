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
	5: "Mass"
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
			KEY_R: _start_test(current_test)
			KEY_ESCAPE: _show_menu()


func _start_test(test_id: int) -> void:
	if test_id < 1 or test_id > 5:
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

	if test_phase == 2 and test_timer > 3.0:
		# Assert: all NPCs within cluster radius of center
		var max_dist := _get_max_dist_from_target(CENTER)
		var expected_radius := SEP_RADIUS * sqrt(npc_count)  # Rough cluster size
		if max_dist < expected_radius + ARRIVAL_THRESHOLD:
			_pass()
			_set_phase("All arrived (max %.0fpx)" % max_dist)
		else:
			_fail("NPC too far: %.0fpx" % max_dist)


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
# DRAWING (visual markers)
# =============================================================================
func _draw() -> void:
	# Draw center crosshair
	var size := 20.0
	draw_line(CENTER - Vector2(size, 0), CENTER + Vector2(size, 0), Color.YELLOW, 2.0)
	draw_line(CENTER - Vector2(0, size), CENTER + Vector2(0, size), Color.YELLOW, 2.0)
	# Draw arrival threshold circle
	draw_arc(CENTER, 8.0, 0, TAU, 32, Color.YELLOW, 1.0)


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
			velocity_label.text = "Arrived: %d/%d" % [arrived, npc_count]
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
