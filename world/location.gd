# location.gd
# Composes locations from multiple sprite pieces
extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"

@onready var label: Label = $Label

const CELL := 17  # 16px sprite + 1px margin
const SCALE := 1.0

# ============================================================
# SPRITE DEFINITIONS - discovered via sprite_browser tool
# ============================================================
# Format: "name": {top_left, size} where size is grid cells (1x1, 2x2, etc.)
const SPRITES := {
	"farm": {"pos": Vector2i(2, 15), "size": Vector2i(2, 2)},
	"tent": {"pos": Vector2i(48, 10), "size": Vector2i(2, 2), "scale": 3.0},
	"fountain": {"pos": Vector2i(50, 9), "size": Vector2i(1, 1), "scale": 2.0},
	"bed": {"pos": Vector2i(15, 2), "size": Vector2i(1, 1)},
	"guard_post": {"pos": Vector2i(20, 20), "size": Vector2i(1, 1), "scale": 2.0},
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

# Calculate visual radius for a sprite (center to corner, scaled)
static func get_sprite_radius(sprite_name: String) -> float:
	if sprite_name not in SPRITES:
		return 48.0  # Default fallback
	var def: Dictionary = SPRITES[sprite_name]
	var size: Vector2i = def.size
	var extra_scale: float = def.get("scale", 1.0)
	var max_cells := maxi(size.x, size.y)
	# Half the diagonal: (cells * 16px * scale * extra_scale) / 2 * sqrt(2)
	return (max_cells * 16.0 * SCALE * extra_scale) / 2.0 * sqrt(2.0)


# Calculate edge radius (center to edge, not corner) - used for arrival
static func get_sprite_edge_radius(sprite_name: String) -> float:
	if sprite_name not in SPRITES:
		return 24.0  # Default fallback
	var def: Dictionary = SPRITES[sprite_name]
	var size: Vector2i = def.size
	var extra_scale: float = def.get("scale", 1.0)
	var max_cells := maxi(size.x, size.y)
	# Half width: (cells * 16px * scale * extra_scale) / 2
	return (max_cells * 16.0 * SCALE * extra_scale) / 2.0


# Get arrival radius for a location type (edge-based, for entering sprite)
static func get_arrival_radius(loc_type: String) -> float:
	var sprite_name: String = LOCATION_SPRITES.get(loc_type, "bed")
	return get_sprite_edge_radius(sprite_name)


func _ready() -> void:
	add_to_group("location")
	_setup_label()


# Get sprite instances for batching into MultiMesh
# Returns Array of {pos: Vector2, uv: Vector2i, scale: float}
func get_sprite_data() -> Array:
	var sprites: Array = []
	var pieces: Array
	match location_type:
		"camp":
			pieces = CAMP_PIECES
		"home":
			pieces = HOME_PIECES
		"field":
			pieces = [{"sprite": "farm", "offset": Vector2.ZERO}]
		"guard_post":
			pieces = GUARD_POST_PIECES
		"fountain":
			pieces = FOUNTAIN_PIECES
		_:
			pieces = HOME_PIECES

	for piece in pieces:
		var sprite_name: String = piece.sprite
		if sprite_name not in SPRITES:
			continue

		var def: Dictionary = SPRITES[sprite_name]
		var sheet_pos: Vector2i = def.pos
		var size: Vector2i = def.size
		var extra_scale: float = def.get("scale", 1.0)
		var total_scale: float = SCALE * extra_scale
		var offset: Vector2 = piece.offset

		# Build grid of sprites for multi-cell definitions
		for row in size.y:
			for col in size.x:
				var uv_coords := Vector2i(sheet_pos.x + col, sheet_pos.y + row)
				# Offset each cell: center the whole sprite, then position each cell
				var cell_offset := Vector2(
					(col - (size.x - 1) / 2.0) * 16,
					(row - (size.y - 1) / 2.0) * 16
				)
				var world_pos: Vector2 = global_position + (offset + cell_offset) * total_scale
				sprites.append({"pos": world_pos, "uv": uv_coords, "scale": total_scale})

	return sprites


func _setup_label() -> void:
	if "Farm" in location_name or "Bed" in location_name or "Post" in location_name:
		label.visible = false
	else:
		label.text = location_name
		label.position = Vector2(-40, -50)
