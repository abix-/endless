# location.gd
# Composes locations from multiple sprite pieces
extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"

@onready var label: Label = $Label

const CELL := 17  # 16px sprite + 1px margin
const SCALE := 3.0

# ============================================================
# SPRITE DEFINITIONS - discovered via sprite_browser tool
# ============================================================
# Format: "name": {top_left, size} where size is grid cells (1x1, 2x2, etc.)
const SPRITES := {
	"home": {"pos": Vector2i(34, 0), "size": Vector2i(1, 1)},
	"field": {"pos": Vector2i(0, 7), "size": Vector2i(1, 1)},
	"tent": {"pos": Vector2i(48, 10), "size": Vector2i(2, 2)},
}

# Camp composition - list of {sprite_name, offset}
const CAMP_PIECES := [
	{"sprite": "tent", "offset": Vector2(0, 0)},
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
			_add_named_sprite("home", Vector2.ZERO)
		"field":
			_add_named_sprite("field", Vector2.ZERO)
		_:
			_add_named_sprite("home", Vector2.ZERO)


func _build_camp() -> void:
	var z := 0
	for piece in CAMP_PIECES:
		z = _add_named_sprite(piece.sprite, piece.offset, z)


func _add_named_sprite(sprite_name: String, offset: Vector2, z_start: int = 0) -> int:
	if sprite_name not in SPRITES:
		return z_start

	var def: Dictionary = SPRITES[sprite_name]
	var pos: Vector2i = def.pos
	var size: Vector2i = def.size

	# Build grid of sprites for multi-cell definitions
	var z := z_start
	for row in size.y:
		for col in size.x:
			var coords := Vector2i(pos.x + col, pos.y + row)
			# Offset each cell: center the whole sprite, then position each cell
			var cell_offset := Vector2(
				(col - (size.x - 1) / 2.0) * 16,
				(row - (size.y - 1) / 2.0) * 16
			)
			_add_sprite_at(coords, offset + cell_offset, z)
			z += 1
	return z


func _add_sprite_at(coords: Vector2i, offset: Vector2, z: int) -> void:
	var sprite := Sprite2D.new()
	sprite.texture = texture
	sprite.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	sprite.region_enabled = true
	sprite.region_rect = Rect2(coords.x * CELL, coords.y * CELL, 16, 16)
	sprite.scale = Vector2(SCALE, SCALE)
	sprite.position = offset * SCALE
	sprite.z_index = -100 + z  # Behind NPCs
	add_child(sprite)


func _setup_label() -> void:
	if "Farm" in location_name or "Home" in location_name:
		label.visible = false
	else:
		label.text = location_name
		label.position = Vector2(-40, -50)
