# terrain_renderer.gd
# Generates world terrain using MultiMesh and noise-based biomes
extends Node2D

const TILE_SIZE := 32
const GRID_WIDTH := 250   # 8000 / 32
const GRID_HEIGHT := 250
const TILE_COUNT := GRID_WIDTH * GRID_HEIGHT

# Biome types
enum Biome { GRASS, FOREST, WATER, ROCK, DIRT }

# Sprite positions in Kenney roguelike sheet (column, row)
# Sheet is 17px grid (16px sprite + 1px margin)
const BIOME_SPRITES := {
	Biome.GRASS: [Vector2i(0, 14), Vector2i(1, 14)],
	Biome.FOREST: [Vector2i(4, 12), Vector2i(5, 12)],
	Biome.WATER: [Vector2i(7, 14), Vector2i(8, 14)],
	Biome.ROCK: [Vector2i(10, 13), Vector2i(11, 13)],
	Biome.DIRT: [Vector2i(2, 14), Vector2i(3, 14)],
}

# Sheet dimensions
const CELL := 17.0
const SPRITE_SIZE := 16.0
const SHEET_SIZE := Vector2(968, 526)

var multimesh: MultiMesh
var spritesheet: Texture2D
@onready var multimesh_instance: MultiMeshInstance2D = $MultiMeshInstance2D

var noise: FastNoiseLite
var town_positions: Array[Vector2] = []
var camp_positions: Array[Vector2] = []


func _ready() -> void:
	z_index = -100
	_init_noise()
	_init_multimesh()


func _init_noise() -> void:
	noise = FastNoiseLite.new()
	noise.noise_type = FastNoiseLite.TYPE_SIMPLEX_SMOOTH
	noise.seed = randi()
	noise.frequency = 0.003  # Large biome patches


func _init_multimesh() -> void:
	multimesh = MultiMesh.new()
	multimesh.transform_format = MultiMesh.TRANSFORM_2D
	multimesh.use_custom_data = true
	multimesh.instance_count = TILE_COUNT
	multimesh.visible_instance_count = TILE_COUNT

	var quad := QuadMesh.new()
	quad.size = Vector2(TILE_SIZE, TILE_SIZE)
	multimesh.mesh = quad

	multimesh_instance.multimesh = multimesh

	# Load and set spritesheet texture
	spritesheet = load("res://assets/roguelikeSheet_transparent.png")
	var mat: ShaderMaterial = multimesh_instance.material as ShaderMaterial
	if mat:
		mat.set_shader_parameter("spritesheet", spritesheet)


func generate(towns: Array[Vector2], camps: Array[Vector2]) -> void:
	town_positions = towns
	camp_positions = camps
	_generate_terrain()


func _generate_terrain() -> void:
	for y in GRID_HEIGHT:
		for x in GRID_WIDTH:
			var idx := x + y * GRID_WIDTH
			var world_pos := Vector2(x * TILE_SIZE + TILE_SIZE / 2, y * TILE_SIZE + TILE_SIZE / 2)

			var biome := _get_biome(world_pos)
			var sprite_pos := _get_sprite_pos(biome)
			var uv := _get_sprite_uv(sprite_pos)

			# Set tile position
			var xform := Transform2D(0, world_pos)
			multimesh.set_instance_transform_2d(idx, xform)

			# Pack UV into custom_data (r=u, g=v, b=width, a=height)
			var custom := Color(uv.position.x, uv.position.y, uv.size.x, uv.size.y)
			multimesh.set_instance_custom_data(idx, custom)


func _get_biome(pos: Vector2) -> Biome:
	# Clear around towns - dirt
	for town_pos in town_positions:
		if pos.distance_to(town_pos) < 200:
			return Biome.DIRT

	# Clear around camps - dirt
	for camp_pos in camp_positions:
		if pos.distance_to(camp_pos) < 150:
			return Biome.DIRT

	# Use noise for natural biomes
	var n := noise.get_noise_2d(pos.x, pos.y)

	if n < -0.3:
		return Biome.WATER
	elif n < 0.1:
		return Biome.GRASS
	elif n < 0.4:
		return Biome.FOREST
	else:
		return Biome.ROCK


func _get_sprite_pos(biome: Biome) -> Vector2i:
	var sprites: Array = BIOME_SPRITES[biome]
	return sprites[randi() % sprites.size()]


func _get_sprite_uv(sprite_pos: Vector2i) -> Rect2:
	var uv_pos := Vector2(sprite_pos.x * CELL, sprite_pos.y * CELL) / SHEET_SIZE
	var uv_size := Vector2(SPRITE_SIZE, SPRITE_SIZE) / SHEET_SIZE
	return Rect2(uv_pos, uv_size)
