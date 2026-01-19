# sprite_browser.gd
# Debug tool to explore the spritesheet and find coordinates
# Run this scene directly to browse sprites
# Controls: WASD to scroll, mouse wheel to zoom, click to select
extends Node2D

const CELL := 17
const DISPLAY_SCALE := 4.0
const SCROLL_SPEED := 400.0
const ZOOM_SPEED := 0.1
const MIN_ZOOM := 0.5
const MAX_ZOOM := 8.0

var texture: Texture2D
var hover_col := -1
var hover_row := -1
var selected_col := -1
var selected_row := -1
var camera_zoom := 1.0

@onready var sheet_sprite: Sprite2D = $SheetSprite
@onready var highlight: ColorRect = $Highlight
@onready var preview: Sprite2D = $UI/Preview
@onready var info_label: Label = $UI/InfoLabel
@onready var camera: Camera2D = $Camera2D


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

	camera.zoom = Vector2(camera_zoom, camera_zoom)


func _process(delta: float) -> void:
	# WASD scrolling
	var scroll := Vector2.ZERO
	if Input.is_key_pressed(KEY_W) or Input.is_key_pressed(KEY_UP):
		scroll.y -= 1
	if Input.is_key_pressed(KEY_S) or Input.is_key_pressed(KEY_DOWN):
		scroll.y += 1
	if Input.is_key_pressed(KEY_A) or Input.is_key_pressed(KEY_LEFT):
		scroll.x -= 1
	if Input.is_key_pressed(KEY_D) or Input.is_key_pressed(KEY_RIGHT):
		scroll.x += 1
	camera.position += scroll * SCROLL_SPEED * delta / camera_zoom

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
		text += "\n\nSelected:\nVector2i(%d, %d)" % [selected_col, selected_row]
	info_label.text = text


func _input(event: InputEvent) -> void:
	if event is InputEventMouseButton:
		if event.pressed:
			if event.button_index == MOUSE_BUTTON_LEFT:
				selected_col = hover_col
				selected_row = hover_row
				preview.region_rect = Rect2(selected_col * CELL, selected_row * CELL, 16, 16)
			elif event.button_index == MOUSE_BUTTON_WHEEL_UP:
				camera_zoom = clampf(camera_zoom * (1.0 + ZOOM_SPEED), MIN_ZOOM, MAX_ZOOM)
				camera.zoom = Vector2(camera_zoom, camera_zoom)
			elif event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
				camera_zoom = clampf(camera_zoom * (1.0 - ZOOM_SPEED), MIN_ZOOM, MAX_ZOOM)
				camera.zoom = Vector2(camera_zoom, camera_zoom)
