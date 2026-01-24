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

# GPU grid: flat cell structure for compute shader upload
const GPU_MAX_PER_CELL := 48
var gpu_grid_width: int = 0
var gpu_grid_height: int = 0
var gpu_grid_counts: PackedInt32Array   # [grid_width * grid_height] counts per cell
var gpu_grid_data: PackedInt32Array     # [grid_width * grid_height * MAX_PER_CELL] NPC indices


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


func rebuild_gpu_grid() -> void:
	# Initialize grid dimensions on first call
	if gpu_grid_width == 0:
		@warning_ignore("integer_division")
		gpu_grid_width = int(Config.world_width / CELL_SIZE) + 1
		@warning_ignore("integer_division")
		gpu_grid_height = int(Config.world_height / CELL_SIZE) + 1
		var total_cells: int = gpu_grid_width * gpu_grid_height
		gpu_grid_counts.resize(total_cells)
		gpu_grid_data.resize(total_cells * GPU_MAX_PER_CELL)

	# Zero counts
	gpu_grid_counts.fill(0)

	# One O(n) pass: assign each NPC to its cell
	var positions: PackedVector2Array = manager.positions
	var healths: PackedFloat32Array = manager.healths
	var gw: int = gpu_grid_width
	var max_pc: int = GPU_MAX_PER_CELL

	for i in manager.count:
		if healths[i] <= 0.0:
			continue
		var pos: Vector2 = positions[i]
		@warning_ignore("narrowing_conversion")
		var cx: int = clampi(int(pos.x / CELL_SIZE), 0, gpu_grid_width - 1)
		@warning_ignore("narrowing_conversion")
		var cy: int = clampi(int(pos.y / CELL_SIZE), 0, gpu_grid_height - 1)
		var cell_idx: int = cy * gw + cx
		var c: int = gpu_grid_counts[cell_idx]
		if c < max_pc:
			gpu_grid_data[cell_idx * max_pc + c] = i
			gpu_grid_counts[cell_idx] = c + 1


func rebuild_neighbor_arrays() -> void:
	var npc_count: int = manager.count
	neighbor_starts.resize(npc_count)
	neighbor_counts.resize(npc_count)

	# Pre-allocate with previous capacity (avoids repeated reallocation)
	var write_idx: int = 0
	var capacity: int = neighbor_data.size()
	if capacity < npc_count * 8:
		capacity = npc_count * 16
		neighbor_data.resize(capacity)

	for i in npc_count:
		neighbor_starts[i] = write_idx
		if manager.healths[i] <= 0.0:
			neighbor_counts[i] = 0
			continue
		var pos: Vector2 = manager.positions[i]
		var coords: Vector2i = _cell_coords(pos)
		var start: int = write_idx
		for dy in range(-1, 2):
			var ny: int = coords.y + dy
			for dx in range(-1, 2):
				var nx: int = coords.x + dx
				var key: int = nx + ny * COORD_MULTIPLIER
				if grid.has(key):
					for idx in grid[key]:
						if idx != i:
							if write_idx >= capacity:
								capacity *= 2
								neighbor_data.resize(capacity)
							neighbor_data[write_idx] = idx
							write_idx += 1
		neighbor_counts[i] = write_idx - start

	neighbor_data.resize(write_idx)


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
