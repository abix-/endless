extends CharacterBody2D

@export var move_speed := 200.0
@export var zoom_speed := 0.1
@export var min_zoom := 0.1
@export var max_zoom := 4.0

@onready var camera: Camera2D = $Camera2D

func _ready() -> void:
	add_to_group("player")
	if not camera:
		camera = Camera2D.new()
		camera.zoom = Vector2(2, 2)
		add_child(camera)

func _process(_delta: float) -> void:
	var input := Vector2.ZERO
	
	if Input.is_action_pressed("move_left"):
		input.x -= 1
	if Input.is_action_pressed("move_right"):
		input.x += 1
	if Input.is_action_pressed("move_up"):
		input.y -= 1
	if Input.is_action_pressed("move_down"):
		input.y += 1
	
	velocity = input.normalized() * move_speed
	move_and_slide()

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_WHEEL_UP:
			_zoom_toward_mouse(1.0 + zoom_speed)
		elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			_zoom_toward_mouse(1.0 - zoom_speed)


func _zoom_toward_mouse(zoom_factor: float) -> void:
	var mouse_world_before := get_global_mouse_position()

	camera.zoom *= zoom_factor
	camera.zoom = camera.zoom.clamp(Vector2(min_zoom, min_zoom), Vector2(max_zoom, max_zoom))

	var mouse_world_after := get_global_mouse_position()
	global_position += mouse_world_before - mouse_world_after
