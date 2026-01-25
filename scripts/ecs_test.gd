# ecs_test.gd - Test scene for EcsNpcManager (Chunk 1)
extends Node2D

var ecs_manager: Node2D
var fps_label: Label
var count_label: Label
var frame_count := 0
var fps_timer := 0.0

func _ready() -> void:
	fps_label = $UI/FPSLabel
	count_label = $UI/CountLabel

	# Create EcsNpcManager
	if ClassDB.class_exists("EcsNpcManager"):
		ecs_manager = ClassDB.instantiate("EcsNpcManager")
		add_child(ecs_manager)
		print("[ECS Test] EcsNpcManager created")

		# Spawn test NPCs
		_spawn_test_npcs()
	else:
		print("[ECS Test] ERROR: EcsNpcManager not found in ClassDB")
		count_label.text = "ERROR: Rust DLL not loaded"


func _spawn_test_npcs() -> void:
	# Spawn 1000 NPCs in a grid
	var count := 1000
	var grid_size := int(sqrt(count))
	var spacing := 20.0

	for y in grid_size:
		for x in grid_size:
			var pos_x := x * spacing + 100.0
			var pos_y := y * spacing + 100.0
			var job := (x + y) % 3  # Cycle through jobs
			ecs_manager.spawn_npc(pos_x, pos_y, job)

	print("[ECS Test] Spawned %d NPCs" % count)


func _process(delta: float) -> void:
	frame_count += 1
	fps_timer += delta

	if fps_timer >= 1.0:
		var fps := frame_count / fps_timer
		fps_label.text = "FPS: %.0f" % fps
		frame_count = 0
		fps_timer = 0.0

	if ecs_manager:
		count_label.text = "NPCs: %d" % ecs_manager.get_npc_count()
