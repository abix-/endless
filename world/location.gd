# location.gd
# Composes locations from multiple sprite pieces
extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"

@onready var label: Label = $Label

const CELL := 17  # 16px sprite + 1px margin
const SCALE := 3.0

# Sprite pieces: {name: Vector2i(col, row)}
const SPRITES := {
	# Buildings
	"house_brown": Vector2i(34, 0),
	"house_gray": Vector2i(34, 6),
	"tent_green": Vector2i(45, 4),
	"tent_tan": Vector2i(46, 4),

	# Terrain
	"dirt": Vector2i(2, 2),
	"grass": Vector2i(0, 4),
	"crops": Vector2i(0, 7),
	"tilled": Vector2i(4, 4),

	# Objects
	"tree_green": Vector2i(14, 2),
	"tree_round": Vector2i(15, 2),
	"bush": Vector2i(17, 2),
	"fence_h": Vector2i(49, 2),
	"fence_v": Vector2i(50, 2),
	"crate": Vector2i(51, 5),
	"barrel": Vector2i(52, 5),
	"campfire": Vector2i(19, 3),
	"well": Vector2i(18, 3),
}

# Compositions: array of {sprite_name, offset}
# Offsets are in pixels (before scaling)
const COMPOSITIONS := {
	"home": [
		{"sprite": "grass", "offset": Vector2(-8, 8)},
		{"sprite": "grass", "offset": Vector2(8, 8)},
		{"sprite": "house_brown", "offset": Vector2(0, 0)},
		{"sprite": "tree_round", "offset": Vector2(-20, -4)},
	],
	"field": [
		{"sprite": "crops", "offset": Vector2(-16, 0)},
		{"sprite": "crops", "offset": Vector2(0, 0)},
		{"sprite": "crops", "offset": Vector2(16, 0)},
		{"sprite": "crops", "offset": Vector2(-16, 16)},
		{"sprite": "crops", "offset": Vector2(0, 16)},
		{"sprite": "crops", "offset": Vector2(16, 16)},
		{"sprite": "crops", "offset": Vector2(-16, -16)},
		{"sprite": "crops", "offset": Vector2(0, -16)},
		{"sprite": "crops", "offset": Vector2(16, -16)},
	],
	"camp": [
		{"sprite": "dirt", "offset": Vector2(-16, 0)},
		{"sprite": "dirt", "offset": Vector2(0, 0)},
		{"sprite": "dirt", "offset": Vector2(16, 0)},
		{"sprite": "dirt", "offset": Vector2(0, 16)},
		{"sprite": "dirt", "offset": Vector2(0, -16)},
		{"sprite": "tent_green", "offset": Vector2(-12, -8)},
		{"sprite": "tent_green", "offset": Vector2(12, 4)},
		{"sprite": "tent_tan", "offset": Vector2(-8, 12)},
		{"sprite": "campfire", "offset": Vector2(0, 0)},
		{"sprite": "crate", "offset": Vector2(20, -12)},
		{"sprite": "barrel", "offset": Vector2(-20, 8)},
	],
	"town": [
		{"sprite": "grass", "offset": Vector2(-8, -8)},
		{"sprite": "grass", "offset": Vector2(8, -8)},
		{"sprite": "grass", "offset": Vector2(-8, 8)},
		{"sprite": "grass", "offset": Vector2(8, 8)},
		{"sprite": "well", "offset": Vector2(0, 0)},
	],
}

var texture: Texture2D


func _ready() -> void:
	texture = preload("res://assets/roguelikeSheet_transparent.png")

	# Remove the default Sprite2D (we'll create our own)
	if has_node("Sprite2D"):
		$Sprite2D.queue_free()

	_build_composition()
	_setup_label()


func _build_composition() -> void:
	var comp_key := location_type
	if comp_key == "home" and not ("Farm" in location_name or "Home" in location_name):
		comp_key = "town"  # Town centers use "town" composition

	if comp_key not in COMPOSITIONS:
		comp_key = "home"  # Fallback

	var pieces: Array = COMPOSITIONS[comp_key]

	for i in pieces.size():
		var piece: Dictionary = pieces[i]
		var sprite_name: String = piece.sprite
		var offset: Vector2 = piece.offset

		if sprite_name not in SPRITES:
			continue

		var coords: Vector2i = SPRITES[sprite_name]
		var sprite := Sprite2D.new()
		sprite.texture = texture
		sprite.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
		sprite.region_enabled = true
		sprite.region_rect = Rect2(coords.x * CELL, coords.y * CELL, 16, 16)
		sprite.scale = Vector2(SCALE, SCALE)
		sprite.position = offset * SCALE
		sprite.z_index = i  # Layer by order in array
		add_child(sprite)


func _setup_label() -> void:
	if "Farm" in location_name or "Home" in location_name:
		label.visible = false
	else:
		label.text = location_name
		label.position = Vector2(-40, -60)
