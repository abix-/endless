# location.gd
# Composes locations from multiple sprite pieces
extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"

@onready var label: Label = $Label

const CELL := 17  # 16px sprite + 1px margin
const SCALE := 3.0

# Sprite coordinates: Vector2i(col, row)
# Tweak these to find the right sprites
const SPRITE_HOME := Vector2i(34, 0)    # Brown house
const SPRITE_FIELD := Vector2i(0, 7)    # Crop field
const SPRITE_CAMP := Vector2i(45, 4)    # Green tent

var texture: Texture2D


func _ready() -> void:
	texture = preload("res://assets/roguelikeSheet_transparent.png")

	# Remove the default Sprite2D
	if has_node("Sprite2D"):
		$Sprite2D.queue_free()

	_build_location()
	_setup_label()


func _build_location() -> void:
	var coords: Vector2i

	match location_type:
		"home":
			coords = SPRITE_HOME
		"field":
			coords = SPRITE_FIELD
		"camp":
			coords = SPRITE_CAMP
		_:
			coords = SPRITE_HOME

	var sprite := Sprite2D.new()
	sprite.texture = texture
	sprite.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	sprite.region_enabled = true
	sprite.region_rect = Rect2(coords.x * CELL, coords.y * CELL, 16, 16)
	sprite.scale = Vector2(SCALE, SCALE)
	add_child(sprite)


func _setup_label() -> void:
	if "Farm" in location_name or "Home" in location_name:
		label.visible = false
	else:
		label.text = location_name
		label.position = Vector2(-40, -50)
