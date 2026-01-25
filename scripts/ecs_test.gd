# ecs_test.gd - Isolated behavior tests for EcsNpcManager
# Press 1-5 to run tests, or use Debug Panel on right
extends Node2D

# Static var persists across scene reloads
static var pending_test: int = 0  # 0 = use debug panel

var ecs_manager: Node2D
var fps_label: Label
var count_label: Label
var test_label: Label
var status_label: Label

# Debug panel controls
var overlap_slider: HSlider
var overlap_label: Label
var count_slider: HSlider
var count_label2: Label
var auto_loop: CheckBox
var time_label: Label
var distance_label: Label
var velocity_label: Label
var expected_label: Label

var frame_count := 0
var fps_timer := 0.0
var test_timer := 0.0
var current_test := 0
var test_phase := 0
var npc_count := 0

const CENTER := Vector2(400, 300)
const SEP_RADIUS := 20.0

# Test definitions
const TESTS := {
	1: "Single NPC Seek/Arrive",
	2: "Two NPCs Separation Only",
	3: "Two NPCs Same Target",
	4: "Ten NPCs Circle Inward",
	5: "Mass Separation (100 NPCs)",
}


func _ready() -> void:
	fps_label = $UI/FPSLabel
	count_label = $UI/CountLabel
	test_label = $UI/TestLabel
	status_label = $UI/StatusLabel

	# Debug panel - controls
	overlap_slider = $UI/DebugPanel/VBox/OverlapSlider
	overlap_label = $UI/DebugPanel/VBox/OverlapLabel
	count_slider = $UI/DebugPanel/VBox/CountSlider
	count_label2 = $UI/DebugPanel/VBox/CountLabel2
	auto_loop = $UI/DebugPanel/VBox/AutoLoop

	# Debug panel - metrics
	time_label = $UI/DebugPanel/VBox/TimeLabel
	distance_label = $UI/DebugPanel/VBox/DistanceLabel
	velocity_label = $UI/DebugPanel/VBox/VelocityLabel
	expected_label = $UI/DebugPanel/VBox/ExpectedLabel

	# Connect controls
	overlap_slider.value_changed.connect(_on_overlap_changed)
	count_slider.value_changed.connect(_on_count_changed)
	$UI/DebugPanel/VBox/ButtonRow/SpawnButton.pressed.connect(_on_spawn_pressed)
	$UI/DebugPanel/VBox/ButtonRow/ClearButton.pressed.connect(_on_clear_pressed)

	# Connect test buttons
	$UI/DebugPanel/VBox/TestButtons/Test1.pressed.connect(_start_test.bind(1))
	$UI/DebugPanel/VBox/TestButtons/Test2.pressed.connect(_start_test.bind(2))
	$UI/DebugPanel/VBox/TestButtons/Test3.pressed.connect(_start_test.bind(3))
	$UI/DebugPanel/VBox/TestButtons/Test4.pressed.connect(_start_test.bind(4))
	$UI/DebugPanel/VBox/TestButtons/Test5.pressed.connect(_start_test.bind(5))

	if ClassDB.class_exists("EcsNpcManager"):
		ecs_manager = ClassDB.instantiate("EcsNpcManager")
		add_child(ecs_manager)
		print("[ECS Test] EcsNpcManager created")

		# Run pending test if set, otherwise show debug panel
		if pending_test > 0:
			call_deferred("_start_test", pending_test)
		else:
			test_label.text = "Debug Mode - Use panel on right"
			status_label.text = "Or press 1-5 for preset tests"
		queue_redraw()
	else:
		print("[ECS Test] ERROR: EcsNpcManager not found in ClassDB")
		count_label.text = "ERROR: Rust DLL not loaded"


func _on_overlap_changed(value: float) -> void:
	overlap_label.text = "Overlap: %d%%" % int(value)


func _on_count_changed(value: float) -> void:
	count_label2.text = "NPC Count: %d" % int(value)


func _on_spawn_pressed() -> void:
	_spawn_debug_npcs()


func _on_clear_pressed() -> void:
	if ecs_manager:
		ecs_manager.reset()
	npc_count = 0
	test_timer = 0.0
	test_phase = 0
	current_test = 0
	status_label.text = "Cleared"


func _spawn_debug_npcs() -> void:
	# Clear first
	_on_clear_pressed()

	var overlap_pct := overlap_slider.value
	var spawn_count := int(count_slider.value)
	var distance := SEP_RADIUS * (1.0 - overlap_pct / 100.0)

	if spawn_count == 2:
		# Two NPCs: place on either side of center
		ecs_manager.spawn_npc(CENTER.x - distance / 2, CENTER.y, 1)
		ecs_manager.spawn_npc(CENTER.x + distance / 2, CENTER.y, 2)
	else:
		# Multiple NPCs: arrange in circle with specified overlap
		for i in spawn_count:
			var angle := (float(i) / spawn_count) * TAU
			var x := CENTER.x + cos(angle) * distance / 2
			var y := CENTER.y + sin(angle) * distance / 2
			ecs_manager.spawn_npc(x, y, i % 3)

	npc_count = spawn_count
	current_test = -1  # Debug mode
	test_phase = 1
	test_timer = 0.0
	status_label.text = "%d NPCs, %d%% overlap (%.1fpx apart)" % [spawn_count, int(overlap_pct), distance]


func _show_menu() -> void:
	current_test = 0
	test_label.text = "Press 1-5 to run a test"
	status_label.text = "1: Arrive  2: Separation  3: Both  4: Circle  5: Mass"


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

	# Reset Rust state before each test
	if ecs_manager.has_method("reset"):
		ecs_manager.reset()

	# If NPCs already exist in GDScript tracking, reload scene
	if npc_count > 0:
		pending_test = test_id
		get_tree().reload_current_scene()
		return

	current_test = test_id
	test_phase = 0
	test_timer = 0.0
	npc_count = 0
	test_label.text = "Test %d: %s" % [test_id, TESTS[test_id]]
	status_label.text = "Running..."

	match test_id:
		1: _setup_test_arrive()
		2: _setup_test_separation()
		3: _setup_test_both()
		4: _setup_test_circle()
		5: _setup_test_mass()


# =============================================================================
# TEST 1: Single NPC Arrive
# Expected: NPC moves from left to center, stops at target
# =============================================================================
func _setup_test_arrive() -> void:
	# Spawn one NPC on the left
	ecs_manager.spawn_npc(100, CENTER.y, 0)  # Farmer (green)
	npc_count = 1
	test_phase = 1
	status_label.text = "Phase 1: Waiting 0.5s before setting target..."


func _update_test_arrive() -> void:
	if test_phase == 1 and test_timer > 0.5:
		# Set target to center
		ecs_manager.set_target(0, CENTER.x, CENTER.y)
		test_phase = 2
		status_label.text = "Phase 2: NPC should move to center and stop"

	if test_phase == 2 and test_timer > 5.0:
		status_label.text = "DONE: NPC should be at center, stationary"


# =============================================================================
# TEST 2: Separation (uses debug panel settings)
# =============================================================================
func _setup_test_separation() -> void:
	# Use debug panel values
	overlap_slider.value = 100  # Start at 100% overlap
	count_slider.value = 2
	_spawn_debug_npcs()
	test_label.text = "Test 2: Separation - adjust sliders to test"


func _update_test_separation() -> void:
	# Auto-loop handled by debug mode
	pass


# =============================================================================
# TEST 3: Two NPCs Same Target
# Expected: Both move to target, then separate
# =============================================================================
func _setup_test_both() -> void:
	# Spawn two NPCs on opposite sides
	ecs_manager.spawn_npc(100, CENTER.y, 0)  # Farmer left
	ecs_manager.spawn_npc(700, CENTER.y, 1)  # Guard right
	npc_count = 2
	test_phase = 1
	status_label.text = "Phase 1: Waiting before setting same target..."


func _update_test_both() -> void:
	if test_phase == 1 and test_timer > 0.5:
		# Both target center
		ecs_manager.set_target(0, CENTER.x, CENTER.y)
		ecs_manager.set_target(1, CENTER.x, CENTER.y)
		test_phase = 2
		status_label.text = "Phase 2: Both moving to center..."

	if test_phase == 2 and test_timer > 5.0:
		status_label.text = "DONE: Both at center, separated, stationary"


# =============================================================================
# TEST 4: Ten NPCs Circle Inward
# Expected: All move to center, then form a ring
# =============================================================================
func _setup_test_circle() -> void:
	# Spawn 10 NPCs in a circle around center
	var radius := 200.0
	for i in 10:
		var angle := (i / 10.0) * TAU
		var x := CENTER.x + cos(angle) * radius
		var y := CENTER.y + sin(angle) * radius
		ecs_manager.spawn_npc(x, y, i % 3)
	npc_count = 10
	test_phase = 1
	status_label.text = "Phase 1: Waiting before setting targets..."


func _update_test_circle() -> void:
	if test_phase == 1 and test_timer > 0.5:
		# All target center
		for i in npc_count:
			ecs_manager.set_target(i, CENTER.x, CENTER.y)
		test_phase = 2
		status_label.text = "Phase 2: All moving to center..."

	if test_phase == 2 and test_timer > 5.0:
		status_label.text = "DONE: Should form tight ring around center"


# =============================================================================
# TEST 5: Mass Separation (100 NPCs at same point)
# Expected: Explode outward, form stable blob
# =============================================================================
func _setup_test_mass() -> void:
	# Spawn 100 NPCs at center
	for i in 100:
		ecs_manager.spawn_npc(CENTER.x, CENTER.y, i % 3)
	npc_count = 100
	test_phase = 1
	status_label.text = "NPCs should explode outward and stabilize"


func _update_test_mass() -> void:
	if test_timer > 5.0:
		status_label.text = "DONE: Should be a stable circular blob"


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

	# Debug mode auto-loop
	if current_test == -1:
		test_timer += delta
		if auto_loop.button_pressed and test_timer > 3.0:
			_spawn_debug_npcs()


func _update_metrics() -> void:
	# Time since spawn
	time_label.text = "Time: %.1fs" % test_timer

	# Calculate expected final distance
	var start_dist := SEP_RADIUS * (1.0 - overlap_slider.value / 100.0)
	expected_label.text = "Start: %.0fpx → End: %.0fpx" % [start_dist, SEP_RADIUS]

	# Distance and velocity would need GPU readback
	# For now show theoretical values
	if npc_count > 0 and test_timer > 0:
		# Estimate: should reach SEP_RADIUS quickly
		var estimated_dist := lerpf(start_dist, SEP_RADIUS, minf(test_timer * 2.0, 1.0))
		distance_label.text = "Est. Dist: %.1fpx" % estimated_dist

		if test_timer < 0.5:
			velocity_label.text = "Velocity: separating..."
		else:
			velocity_label.text = "Velocity: stable"
	else:
		distance_label.text = "Distance: --"
		velocity_label.text = "Velocity: --"
