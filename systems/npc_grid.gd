# npc_grid.gd
# Sparse hash grid for efficient nearby-NPC queries
# Scales to any world size with O(npc_count) memory
extends RefCounted
class_name NPCGrid

const CELL_SIZE := 100.0
const COORD_MULTIPLIER := 1000000  # Pack x + y * this for hash key

var manager: Node
var grid: Dictionary  # {int: Array[int]} - cell key -> NPC indices

# Pre-computed neighbors for parallel access (avoids allocation during parallel phase)
var neighbor_starts: PackedInt32Array  # Start index in neighbor_data for each NPC
var neighbor_counts: PackedInt32Array  # Number of neighbors for each NPC
var neighbor_data: PackedInt32Array     # Flat array of all neighbor indices


func _init(npc_manager: Node) -> void:
	manager = npc_manager
	grid = {}
	neighbor_starts = PackedInt32Array()
	neighbor_counts = PackedInt32Array()
	neighbor_data = PackedInt32Array()


func rebuild() -> void:
	grid.clear()

	for i in manager.count:
		if manager.healths[i] <= 0:
			continue

		var key: int = _cell_key(manager.positions[i])
		if not grid.has(key):
			grid[key] = []
		grid[key].append(i)


func _cell_key(pos: Vector2) -> int:
	@warning_ignore("narrowing_conversion")
	var cx: int = int(pos.x / CELL_SIZE)
	@warning_ignore("narrowing_conversion")
	var cy: int = int(pos.y / CELL_SIZE)
	return cx + cy * COORD_MULTIPLIER


func _cell_coords(pos: Vector2) -> Vector2i:
	@warning_ignore("narrowing_conversion")
	var cx: int = int(pos.x / CELL_SIZE)
	@warning_ignore("narrowing_conversion")
	var cy: int = int(pos.y / CELL_SIZE)
	return Vector2i(cx, cy)


func get_nearby(pos: Vector2) -> Array[int]:
	var results: Array[int] = []
	var coords: Vector2i = _cell_coords(pos)

	for dy in range(-1, 2):
		var ny: int = coords.y + dy
		for dx in range(-1, 2):
			var nx: int = coords.x + dx
			var key: int = nx + ny * COORD_MULTIPLIER
			if grid.has(key):
				results.append_array(grid[key])

	return results


func rebuild_neighbor_arrays() -> void:
	var npc_count: int = manager.count
	neighbor_starts.resize(npc_count)
	neighbor_counts.resize(npc_count)
	neighbor_data.resize(0)

	for i in npc_count:
		neighbor_starts[i] = neighbor_data.size()
		if manager.healths[i] <= 0.0:
			neighbor_counts[i] = 0
			continue
		var pos: Vector2 = manager.positions[i]
		var coords: Vector2i = _cell_coords(pos)
		var start: int = neighbor_data.size()
		for dy in range(-1, 2):
			var ny: int = coords.y + dy
			for dx in range(-1, 2):
				var nx: int = coords.x + dx
				var key: int = nx + ny * COORD_MULTIPLIER
				if grid.has(key):
					for idx in grid[key]:
						if idx != i:
							neighbor_data.append(idx)
		neighbor_counts[i] = neighbor_data.size() - start


# Parallel-safe: get pre-computed neighbor count for NPC i (no allocation)
func get_neighbor_count(i: int) -> int:
	return neighbor_counts[i]


# Parallel-safe: get neighbor at index j for NPC i (no allocation)
func get_neighbor(i: int, j: int) -> int:
	return neighbor_data[neighbor_starts[i] + j]


func get_nearby_in_radius(pos: Vector2, radius: float) -> Array[int]:
	var results: Array[int] = []
	var radius_cells: int = ceili(radius / CELL_SIZE)
	var coords: Vector2i = _cell_coords(pos)

	for dy in range(-radius_cells, radius_cells + 1):
		var ny: int = coords.y + dy
		for dx in range(-radius_cells, radius_cells + 1):
			var nx: int = coords.x + dx
			var key: int = nx + ny * COORD_MULTIPLIER
			if grid.has(key):
				results.append_array(grid[key])

	return results


func get_npcs_in_rect(min_x: float, max_x: float, min_y: float, max_y: float) -> Array[int]:
	var result: Array[int] = []

	@warning_ignore("narrowing_conversion")
	var x1: int = int(min_x / CELL_SIZE)
	@warning_ignore("narrowing_conversion")
	var x2: int = int(max_x / CELL_SIZE)
	@warning_ignore("narrowing_conversion")
	var y1: int = int(min_y / CELL_SIZE)
	@warning_ignore("narrowing_conversion")
	var y2: int = int(max_y / CELL_SIZE)

	for y in range(y1, y2 + 1):
		for x in range(x1, x2 + 1):
			var key: int = x + y * COORD_MULTIPLIER
			if grid.has(key):
				result.append_array(grid[key])

	return result
