# location.gd
# Composes locations from multiple sprite pieces
extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"

@onready var label: Label = $Label

const CELL := 17  # 16px sprite + 1px margin
const SCALE := 3.0

# Sprite coordinates: Vector2i(col, row)
const SPRITE_HOME := Vector2i(34, 0)    # Brown house
const SPRITE_FIELD := Vector2i(0, 7)    # Crop field

# ============================================================
# SPRITE REFERENCE - discovered via sprite_browser tool
# ============================================================
# Tent (2x2):
#   (48, 10) top-left     (49, 10) top-right
#   (48, 11) bottom-left  (49, 11) bottom-right
# ============================================================

# Camp pieces - build this up
const CAMP_PIECES := [
	# Tent (2x2 grid, offset so pieces align)
	{"coords": Vector2i(48, 10), "offset": Vector2(-8, -8)},   # tent top-left
	{"coords": Vector2i(49, 10), "offset": Vector2(8, -8)},    # tent top-right
	{"coords": Vector2i(48, 11), "offset": Vector2(-8, 8)},    # tent bottom-left
	{"coords": Vector2i(49, 11), "offset": Vector2(8, 8)},     # tent bottom-right
]

var texture: Texture2D


func _ready() -> void:
	texture = preload("res://assets/roguelikeSheet_transparent.png")

	# Remove the default Sprite2D
	if has_node("Sprite2D"):
		$Sprite2D.queue_free()

	_build_location()
	_setup_label()


func _build_location() -> void:
	match location_type:
		"camp":
			_build_camp()
		"home":
			_add_sprite(SPRITE_HOME, Vector2.ZERO)
		"field":
			_add_sprite(SPRITE_FIELD, Vector2.ZERO)
		_:
			_add_sprite(SPRITE_HOME, Vector2.ZERO)


func _build_camp() -> void:
	for i in CAMP_PIECES.size():
		var piece: Dictionary = CAMP_PIECES[i]
		var sprite := _add_sprite(piece.coords, piece.offset)
		sprite.z_index = i


func _add_sprite(coords: Vector2i, offset: Vector2) -> Sprite2D:
	var sprite := Sprite2D.new()
	sprite.texture = texture
	sprite.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	sprite.region_enabled = true
	sprite.region_rect = Rect2(coords.x * CELL, coords.y * CELL, 16, 16)
	sprite.scale = Vector2(SCALE, SCALE)
	sprite.position = offset * SCALE
	add_child(sprite)
	return sprite


func _setup_label() -> void:
	if "Farm" in location_name or "Home" in location_name:
		label.visible = false
	else:
		label.text = location_name
		label.position = Vector2(-40, -50)
