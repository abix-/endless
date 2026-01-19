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

func _ready() -> void:
	$Sprite2D.texture = preload("res://assets/roguelikeSheet_transparent.png")
	$Sprite2D.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	$Sprite2D.scale = Vector2(3, 3)
	$Sprite2D.region_enabled = true

	if location_type in FRAMES:
		var pos: Vector2i = FRAMES[location_type]
		if location_type in LARGE_LOCATIONS:
			# 2x2 grid of sprites
			var size := SPRITE_SIZE * 2 + MARGIN
			$Sprite2D.region_rect = Rect2(pos.x * CELL, pos.y * CELL, size, size)
		else:
			$Sprite2D.region_rect = Rect2(pos.x * CELL, pos.y * CELL, SPRITE_SIZE, SPRITE_SIZE)

	label.text = location_name
