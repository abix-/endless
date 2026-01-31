# location_renderer.gd
# Batches all location sprites into a single MultiMesh (1 draw call)
extends Node2D

const CELL := 17.0  # 16px sprite + 1px margin
const SPRITE_SIZE := 16.0
const SHEET_SIZE := Vector2(968, 526)

var multimesh: MultiMesh
var spritesheet: Texture2D
@onready var multimesh_instance: MultiMeshInstance2D = $MultiMeshInstance2D


func _ready() -> void:
	z_index = -100  # Behind NPCs, same as terrain
	spritesheet = preload("res://assets/roguelikeSheet_transparent.png")
	var mat: ShaderMaterial = multimesh_instance.material as ShaderMaterial
	if mat:
		mat.set_shader_parameter("spritesheet", spritesheet)


# Call this after all locations are created
# sprites: Array of {pos: Vector2, uv: Vector2i, scale: float}
func build(sprites: Array) -> void:
	if sprites.is_empty():
		return

	multimesh = MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_2D
	multimesh.use_custom_data = true
	multimesh.instance_count = sprites.size()
	multimesh.visible_instance_count = sprites.size()

	# QuadMesh sized for 16px sprite at scale 1.0
	var quad := QuadMesh.new()
	quad.size = Vector2(SPRITE_SIZE, SPRITE_SIZE)
	multimesh.mesh = quad

	for i in sprites.size():
		var s: Dictionary = sprites[i]
		var world_pos: Vector2 = s.pos
		var uv_coords: Vector2i = s.uv  # Sprite sheet grid coords
		var sprite_scale: float = s.scale

		# Transform: position + scale
		var xform := Transform2D(0, Vector2(sprite_scale, sprite_scale), 0, world_pos)
		multimesh.set_instance_transform_2d(i, xform)

		# UV: top-left corner in normalized coords
		var uv_pos := Vector2(uv_coords.x * CELL, uv_coords.y * CELL) / SHEET_SIZE
		var uv_width := SPRITE_SIZE / SHEET_SIZE.x

		# Pack into custom_data (r=u, g=v, b=width, a=1 for no tiling)
		var custom := Color(uv_pos.x, uv_pos.y, uv_width, 1.0)
		multimesh.set_instance_custom_data(i, custom)

	multimesh_instance.multimesh = multimesh
