extends Area2D

@export var location_name := "Unnamed"
@export var location_type := "generic"  # "home", "field", "guardpost"

@onready var label: Label = $Label

const SPRITE_SIZE := 16
const MARGIN := 1
const CELL := SPRITE_SIZE + MARGIN  # 17

const FRAMES := {
	"field": Vector2i(3, 10),
	"home": Vector2i(34, 1),
	"guardpost": Vector2i(51, 4),
}

func _ready() -> void:
	$Sprite2D.texture = preload("res://assets/roguelikeSheet_transparent.png")
	$Sprite2D.texture_filter = CanvasItem.TEXTURE_FILTER_NEAREST
	$Sprite2D.scale = Vector2(3, 3)
	$Sprite2D.region_enabled = true
	
	if location_type in FRAMES:
		var pos: Vector2i = FRAMES[location_type]
		$Sprite2D.region_rect = Rect2(pos.x * CELL, pos.y * CELL, SPRITE_SIZE, SPRITE_SIZE)
	
	label.text = location_name
