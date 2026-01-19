# sprite_browser.gd
# Debug tool to explore the spritesheet and find coordinates
# Run this scene directly to browse sprites
extends Node2D

const CELL := 17
const DISPLAY_SCALE := 4.0

var texture: Texture2D
var hover_col := -1
var hover_row := -1
var selected_col := -1
var selected_row := -1

@onready var sheet_sprite: Sprite2D = $SheetSprite
@onready var highlight: ColorRect = $Highlight
@onready var preview: Sprite2D = $Preview
@onready var info_label: Label = $InfoLabel


func _ready() -> void:
	texture = preload("res://assets/roguelikeSheet_transparent.png")
	sheet_sprite.texture = texture
	sheet_sprite.scale = Vector2(DISPLAY_SCALE, DISPLAY_SCALE)
	sheet_sprite.centered = false

	preview.texture = texture
	preview.region_enabled = true
	preview.scale = Vector2(6, 6)
	preview.centered = false

	highlight.size = Vector2(16, 16) * DISPLAY_SCALE
	highlight.color = Color(1, 1, 0, 0.3)


func _process(_delta: float) -> void:
	var mouse_pos := get_global_mouse_position()

	# Convert to grid coordinates
	hover_col = int(mouse_pos.x / (CELL * DISPLAY_SCALE))
	hover_row = int(mouse_pos.y / (CELL * DISPLAY_SCALE))

	# Clamp to valid range
	var max_col := int(texture.get_width() / CELL)
	var max_row := int(texture.get_height() / CELL)
	hover_col = clampi(hover_col, 0, max_col - 1)
	hover_row = clampi(hover_row, 0, max_row - 1)

	# Position highlight
	highlight.position = Vector2(hover_col * CELL, hover_row * CELL) * DISPLAY_SCALE

	# Update info
	var text := "Hover: (%d, %d)" % [hover_col, hover_row]
	if selected_col >= 0:
		text += "\nSelected: (%d, %d)" % [selected_col, selected_row]
		text += "\nVector2i(%d, %d)" % [selected_col, selected_row]
	info_label.text = text


func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		selected_col = hover_col
		selected_row = hover_row

		# Update preview
		preview.region_rect = Rect2(selected_col * CELL, selected_row * CELL, 16, 16)
