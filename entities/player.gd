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
			camera.zoom *= 1.0 + zoom_speed
			camera.zoom = camera.zoom.clamp(Vector2(min_zoom, min_zoom), Vector2(max_zoom, max_zoom))
		elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			camera.zoom *= 1.0 - zoom_speed
			camera.zoom = camera.zoom.clamp(Vector2(min_zoom, min_zoom), Vector2(max_zoom, max_zoom))
