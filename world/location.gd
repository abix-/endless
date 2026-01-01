extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"  # "home", "work", "field", "guardpost"

@onready var label: Label = $Label
@onready var sprite: Sprite2D = $Sprite2D

const SPRITE_SIZE := 16
const MARGIN := 1
const CELL := SPRITE_SIZE + MARGIN  # 17

# Frame positions: column, row (0-indexed)
const FRAMES := {
	"field": Vector2i(3, 10),
	"home": Vector2i(34, 1),      # adjust these
	"guardpost": Vector2i(51, 4), # to your sheet
}

func _ready() -> void:
	$Sprite2D.texture = preload("res://assets/roguelikeSheet_transparent.png")
	$Sprite2D.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	$Sprite2D.scale = Vector2(3, 3)
	
	if location_type in FRAMES:
		var pos: Vector2i = FRAMES[location_type]
		$Sprite2D.region_enabled = true
		$Sprite2D.region_rect = Rect2(pos.x * CELL, pos.y * CELL, SPRITE_SIZE, SPRITE_SIZE)
	_update_label()

func _update_label() -> void:
	label.text = location_name

func set_color(color: Color) -> void:
	print("set_color called with: ", color)
	print("Sprite2D: ", $Sprite2D)
	$Sprite2D.modulate = color
