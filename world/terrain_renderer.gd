# terrain_renderer.gd
# Generates world terrain using MultiMesh and noise-based biomes
extends Node2D

const TILE_SIZE := 32
const GRID_WIDTH := 250   # 8000 / 32
const GRID_HEIGHT := 250
const TILE_COUNT := GRID_WIDTH * GRID_HEIGHT

# Biome types
enum Biome { GRASS, FOREST, WATER, ROCK, DIRT }

# Sprite definitions: {pos: Vector2i, size: int (cells), tile: int (repeat count)}
# pos = top-left cell in sheet
# size = 1 for 16px, 2 for 32px native sprite
# tile = 1 to stretch, 2 to tile 2x2 (4 copies of 1x1 sprite)
# Sheet is 17px grid (16px sprite + 1px margin)
const BIOME_SPRITES := {
	Biome.GRASS: [
		{"pos": Vector2i(0, 14), "size": 1, "tile": 2},
		{"pos": Vector2i(1, 14), "size": 1, "tile": 2},
	],
	Biome.FOREST: [
		{"pos": Vector2i(4, 12), "size": 1, "tile": 2},
		{"pos": Vector2i(5, 12), "size": 1, "tile": 2},
	],
	Biome.WATER: [
		{"pos": Vector2i(3, 1), "size": 1, "tile": 2},
	],
	Biome.ROCK: [
		{"pos": Vector2i(7, 13), "size": 2, "tile": 1},
	],
	Biome.DIRT: [
		{"pos": Vector2i(8, 10), "size": 1, "tile": 2},
	],
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

# Store tile data for inspection
var tile_biomes: PackedInt32Array
var tile_sprites: Array[Vector2i] = []


func _ready() -> void:
	z_index = -100
	_init_noise()
	_init_multimesh()
	_init_tile_data()


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


func _init_tile_data() -> void:
	tile_biomes.resize(TILE_COUNT)
	tile_sprites.resize(TILE_COUNT)


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
			var sprite_def := _get_sprite_def(biome)
			var uv := _get_sprite_uv(sprite_def)

			# Store tile data for inspection
			tile_biomes[idx] = biome
			tile_sprites[idx] = sprite_def.pos

			# Set tile position
			var xform := Transform2D(0, world_pos)
			multimesh.set_instance_transform_2d(idx, xform)

			# Pack UV into custom_data (r=u, g=v, b=width, a=tile_count)
			var custom := Color(uv.uv_pos.x, uv.uv_pos.y, uv.uv_width, float(uv.tile))
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


func _get_sprite_def(biome: Biome) -> Dictionary:
	var sprites: Array = BIOME_SPRITES[biome]
	return sprites[randi() % sprites.size()]


func _get_sprite_uv(sprite_def: Dictionary) -> Dictionary:
	var pos: Vector2i = sprite_def.pos
	var size: int = sprite_def.size
	var tile: int = sprite_def.tile
	# UV position: top-left of sprite in sheet
	var uv_pos := Vector2(pos.x * CELL, pos.y * CELL) / SHEET_SIZE
	# UV size: single cell size (shader handles tiling)
	var uv_width: float = (SPRITE_SIZE + (size - 1)) / SHEET_SIZE.x  # 1x1=16px, 2x2=17px per cell
	return {"uv_pos": uv_pos, "uv_width": uv_width, "tile": tile}


# Biome names for display
const BIOME_NAMES := {
	Biome.GRASS: "Grass",
	Biome.FOREST: "Forest",
	Biome.WATER: "Water",
	Biome.ROCK: "Rock",
	Biome.DIRT: "Dirt",
}


# Get tile info at world position
func get_tile_at(world_pos: Vector2) -> Dictionary:
	var grid_x := int(world_pos.x / TILE_SIZE)
	var grid_y := int(world_pos.y / TILE_SIZE)

	# Bounds check
	if grid_x < 0 or grid_x >= GRID_WIDTH or grid_y < 0 or grid_y >= GRID_HEIGHT:
		return {}

	var idx := grid_x + grid_y * GRID_WIDTH
	var biome: int = tile_biomes[idx]
	var sprite: Vector2i = tile_sprites[idx]

	return {
		"grid_x": grid_x,
		"grid_y": grid_y,
		"world_x": grid_x * TILE_SIZE,
		"world_y": grid_y * TILE_SIZE,
		"biome": biome,
		"biome_name": BIOME_NAMES.get(biome, "Unknown"),
		"sprite_col": sprite.x,
		"sprite_row": sprite.y,
	}
