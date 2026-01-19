extends CharacterBody2D

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

	velocity = input.normalized() * UserSettings.scroll_speed
	move_and_slide()

func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_WHEEL_UP:
			_zoom_toward_mouse(1.0 + zoom_speed)
		elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			_zoom_toward_mouse(1.0 - zoom_speed)


func _zoom_toward_mouse(zoom_factor: float) -> void:
	var viewport := get_viewport()
	var mouse_screen := viewport.get_mouse_position()
	var viewport_size := viewport.get_visible_rect().size

	# Mouse offset from screen center
	var screen_center := viewport_size / 2.0
	var mouse_offset := mouse_screen - screen_center

	# World position under mouse before zoom
	var world_pos := global_position + mouse_offset / camera.zoom

	# Apply zoom
	var old_zoom := camera.zoom
	camera.zoom *= zoom_factor
	camera.zoom = camera.zoom.clamp(Vector2(min_zoom, min_zoom), Vector2(max_zoom, max_zoom))

	# Move camera so world_pos stays under mouse
	global_position = world_pos - mouse_offset / camera.zoom
