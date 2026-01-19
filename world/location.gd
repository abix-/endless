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
	"farm": {"pos": Vector2i(2, 15), "size": Vector2i(3, 3)},
	"tent": {"pos": Vector2i(48, 10), "size": Vector2i(2, 2)},
	"fountain": {"pos": Vector2i(50, 9), "size": Vector2i(1, 1)},
	"bed": {"pos": Vector2i(15, 2), "size": Vector2i(1, 1)},
	"guard_post": {"pos": Vector2i(20, 20), "size": Vector2i(1, 1)},
}

const HOME_PIECES := [
	{"sprite": "bed", "offset": Vector2(0, 0)},
]

const CAMP_PIECES := [
	{"sprite": "tent", "offset": Vector2(0, 0)},
]

const GUARD_POST_PIECES := [
	{"sprite": "guard_post", "offset": Vector2(0, 0)},
]

const FOUNTAIN_PIECES := [
	{"sprite": "fountain", "offset": Vector2(0, 0)},
]

# Map location types to their primary sprite for radius calculation
const LOCATION_SPRITES := {
	"field": "farm",
	"camp": "tent",
	"home": "bed",
	"fountain": "fountain",
	"guard_post": "guard_post",
}

var texture: Texture2D


# Calculate visual radius for a sprite (center to corner, scaled)
static func get_sprite_radius(sprite_name: String) -> float:
	if sprite_name not in SPRITES:
		return 48.0  # Default fallback
	var size: Vector2i = SPRITES[sprite_name].size
	var max_cells := maxi(size.x, size.y)
	# Half the diagonal: (cells * 16px * scale) / 2 * sqrt(2)
	return (max_cells * 16.0 * SCALE) / 2.0 * sqrt(2.0)


# Calculate edge radius (center to edge, not corner) - used for arrival
static func get_sprite_edge_radius(sprite_name: String) -> float:
	if sprite_name not in SPRITES:
		return 24.0  # Default fallback
	var size: Vector2i = SPRITES[sprite_name].size
	var max_cells := maxi(size.x, size.y)
	# Half width: (cells * 16px * scale) / 2
	return (max_cells * 16.0 * SCALE) / 2.0


# Get arrival radius for a location type (edge-based, for entering sprite)
static func get_arrival_radius(location_type: String) -> float:
	var sprite_name: String = LOCATION_SPRITES.get(location_type, "bed")
	return get_sprite_edge_radius(sprite_name)


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
			_build_from_pieces(CAMP_PIECES)
		"home":
			_build_from_pieces(HOME_PIECES)
		"field":
			_add_named_sprite("farm", Vector2.ZERO)
		"guard_post":
			_build_from_pieces(GUARD_POST_PIECES)
		"fountain":
			_build_from_pieces(FOUNTAIN_PIECES)
		_:
			_build_from_pieces(HOME_PIECES)


func _build_from_pieces(pieces: Array) -> void:
	var z := 0
	for piece in pieces:
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
	if "Farm" in location_name or "Home" in location_name or "Post" in location_name:
		label.visible = false
	else:
		label.text = location_name
		label.position = Vector2(-40, -50)
