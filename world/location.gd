extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"  # "home", "field", "guardpost"

@onready var label: Label = $Label

const SPRITE_SIZE := 16
const MARGIN := 1
const CELL := SPRITE_SIZE + MARGIN  # 17

const FRAMES := {
	"field": Vector2i(0, 7),      # Crop field (2x2)
	"home": Vector2i(34, 0),      # House with roof
	"guardpost": Vector2i(50, 3), # Banner/flag
	"camp": Vector2i(45, 4),      # Green tent
}

const LARGE_LOCATIONS := ["field"]  # Use 2x2 grid
const XLARGE_LOCATIONS := ["camp"]  # Use 4x4 grid

func _ready() -> void:
	$Sprite2D.texture = preload("res://assets/roguelikeSheet_transparent.png")
	$Sprite2D.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	$Sprite2D.scale = Vector2(3, 3)
	$Sprite2D.region_enabled = true

	if location_type in FRAMES:
		var pos: Vector2i = FRAMES[location_type]
		if location_type in XLARGE_LOCATIONS:
			# 4x4 grid of sprites (covers ~200px with 3x scale)
			var size := SPRITE_SIZE * 4 + MARGIN * 3
			$Sprite2D.region_rect = Rect2(pos.x * CELL, pos.y * CELL, size, size)
		elif location_type in LARGE_LOCATIONS:
			# 2x2 grid of sprites
			var size := SPRITE_SIZE * 2 + MARGIN
			$Sprite2D.region_rect = Rect2(pos.x * CELL, pos.y * CELL, size, size)
		else:
			$Sprite2D.region_rect = Rect2(pos.x * CELL, pos.y * CELL, SPRITE_SIZE, SPRITE_SIZE)

	# Only show labels for major locations (towns/camps), not individual homes/farms
	if "Farm" in location_name or "Home" in location_name:
		label.visible = false
	else:
		label.text = location_name
