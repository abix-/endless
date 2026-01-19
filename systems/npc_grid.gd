# npc_grid.gd
# Spatial grid for efficient nearby-NPC queries
extends RefCounted
class_name NPCGrid

var manager: Node

var grid_cells: PackedInt32Array
var grid_cell_counts: PackedInt32Array
var grid_cell_starts: PackedInt32Array



func _init(npc_manager: Node) -> void:
	manager = npc_manager
	_init_grid()


func _init_grid() -> void:
	var total_cells: int = Config.GRID_SIZE * Config.GRID_SIZE
	grid_cells.resize(total_cells * Config.GRID_CELL_CAPACITY)
	grid_cell_counts.resize(total_cells)
	grid_cell_starts.resize(total_cells)

	for i in total_cells:
		grid_cell_starts[i] = i * Config.GRID_CELL_CAPACITY
		grid_cell_counts[i] = 0


func rebuild() -> void:
	for i in grid_cell_counts.size():
		grid_cell_counts[i] = 0

	for i in manager.count:
		if manager.healths[i] <= 0:
			continue

		var cell_idx: int = _cell_index(manager.positions[i])
		var cell_count: int = grid_cell_counts[cell_idx]

		if cell_count < Config.GRID_CELL_CAPACITY:
			var slot: int = grid_cell_starts[cell_idx] + cell_count
			grid_cells[slot] = i
			grid_cell_counts[cell_idx] = cell_count + 1


func _cell_index(pos: Vector2) -> int:
	@warning_ignore("narrowing_conversion")
	var x: int = clampi(int(pos.x / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var y: int = clampi(int(pos.y / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)
	return y * Config.GRID_SIZE + x


func get_nearby(pos: Vector2) -> Array[int]:
	var results: Array[int] = []

	@warning_ignore("narrowing_conversion")
	var cx: int = clampi(int(pos.x / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var cy: int = clampi(int(pos.y / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)

	for dy in range(-1, 2):
		var ny: int = cy + dy
		if ny < 0 or ny >= Config.GRID_SIZE:
			continue
		for dx in range(-1, 2):
			var nx: int = cx + dx
			if nx < 0 or nx >= Config.GRID_SIZE:
				continue

			var cell_idx: int = ny * Config.GRID_SIZE + nx
			var start: int = grid_cell_starts[cell_idx]
			var cell_count: int = grid_cell_counts[cell_idx]

			for j in cell_count:
				results.append(grid_cells[start + j])

	return results


func get_nearby_in_radius(pos: Vector2, radius: float) -> Array[int]:
	var results: Array[int] = []
	var radius_cells: int = ceili(radius / Config.GRID_CELL_SIZE)

	@warning_ignore("narrowing_conversion")
	var cx: int = clampi(int(pos.x / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var cy: int = clampi(int(pos.y / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)

	for dy in range(-radius_cells, radius_cells + 1):
		var ny: int = cy + dy
		if ny < 0 or ny >= Config.GRID_SIZE:
			continue
		for dx in range(-radius_cells, radius_cells + 1):
			var nx: int = cx + dx
			if nx < 0 or nx >= Config.GRID_SIZE:
				continue

			var cell_idx: int = ny * Config.GRID_SIZE + nx
			var start: int = grid_cell_starts[cell_idx]
			var cell_count: int = grid_cell_counts[cell_idx]

			for j in cell_count:
				results.append(grid_cells[start + j])

	return results


func get_cells_in_rect(min_x: float, max_x: float, min_y: float, max_y: float) -> PackedInt32Array:
	var result: PackedInt32Array = []

	@warning_ignore("narrowing_conversion")
	var x1: int = clampi(int(min_x / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var x2: int = clampi(int(max_x / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var y1: int = clampi(int(min_y / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)
	@warning_ignore("narrowing_conversion")
	var y2: int = clampi(int(max_y / Config.GRID_CELL_SIZE), 0, Config.GRID_SIZE - 1)

	for y in range(y1, y2 + 1):
		for x in range(x1, x2 + 1):
			result.append(y * Config.GRID_SIZE + x)

	return result
