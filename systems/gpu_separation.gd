# gpu_separation.gd
# GPU compute shader for NPC separation - async threaded with grid lookup
extends RefCounted
class_name GPUSeparation

var rd: RenderingDevice
var shader: RID
var pipeline: RID

# Buffers (bindings 0-7)
var position_buffer: RID
var size_buffer: RID
var health_buffer: RID
var output_buffer: RID
var state_buffer: RID
var target_buffer: RID
var grid_counts_buffer: RID
var grid_data_buffer: RID
var uniform_set: RID

# Buffer capacities
var _npc_capacity: int = 0
var _grid_cells: int = 0

# Push constants (32 bytes)
var push_constants: PackedByteArray

# Threading
var _thread: Thread
var _semaphore: Semaphore
var _mutex: Mutex
var _exit_flag: bool = false

# Shader data (loaded on main thread, used by worker to create pipeline)
var _shader_spirv: RDShaderSPIRV

# Result (main thread reads)
var _result: PackedVector2Array

# Snapshot data for GPU thread
var _snap_positions: PackedByteArray
var _snap_sizes: PackedByteArray
var _snap_healths: PackedByteArray
var _snap_states: PackedByteArray
var _snap_targets: PackedByteArray
var _snap_grid_counts: PackedByteArray
var _snap_grid_data: PackedByteArray
var _snap_count: int = 0
var _snap_grid_cells: int = 0
var _snap_max_per_cell: int = 0
var _snap_grid_width: int = 0
var _snap_grid_height: int = 0
var _snap_separation_radius: float = 0.0
var _snap_separation_strength: float = 0.0
var _snap_cell_size: float = 0.0

var is_initialized := false


func _init() -> void:
	push_constants = PackedByteArray()
	push_constants.resize(32)
	_result = PackedVector2Array()
	_mutex = Mutex.new()
	_semaphore = Semaphore.new()


func initialize() -> bool:
	var shader_file := load("res://shaders/separation_compute.glsl")
	if shader_file == null:
		push_error("GPUSeparation: Failed to load shader file")
		return false

	_shader_spirv = shader_file.get_spirv()
	if _shader_spirv == null:
		push_error("GPUSeparation: Failed to get SPIRV")
		return false

	_thread = Thread.new()
	_thread.start(_thread_func)
	return true


func kick(
	positions: PackedVector2Array,
	sizes: PackedFloat32Array,
	healths: PackedFloat32Array,
	states: PackedInt32Array,
	targets: PackedVector2Array,
	grid_counts: PackedInt32Array,
	grid_data: PackedInt32Array,
	count: int,
	grid_width: int,
	grid_height: int,
	max_per_cell: int,
	cell_size: float,
	separation_radius: float,
	separation_strength: float
) -> void:
	if not is_initialized:
		return

	_mutex.lock()
	_snap_positions = positions.to_byte_array()
	_snap_sizes = sizes.to_byte_array()
	_snap_healths = healths.to_byte_array()
	_snap_states = states.to_byte_array()
	_snap_targets = targets.to_byte_array()
	_snap_grid_counts = grid_counts.to_byte_array()
	_snap_grid_data = grid_data.to_byte_array()
	_snap_count = count
	_snap_grid_cells = grid_width * grid_height
	_snap_max_per_cell = max_per_cell
	_snap_grid_width = grid_width
	_snap_grid_height = grid_height
	_snap_cell_size = cell_size
	_snap_separation_radius = separation_radius
	_snap_separation_strength = separation_strength
	_mutex.unlock()

	_semaphore.post()


func get_result() -> PackedVector2Array:
	return _result


func _thread_func() -> void:
	rd = RenderingServer.create_local_rendering_device()
	if rd == null:
		push_warning("GPUSeparation: RenderingDevice unavailable")
		return

	shader = rd.shader_create_from_spirv(_shader_spirv)
	if not shader.is_valid():
		push_error("GPUSeparation: Failed to create shader")
		return

	pipeline = rd.compute_pipeline_create(shader)
	if not pipeline.is_valid():
		push_error("GPUSeparation: Failed to create pipeline")
		return

	is_initialized = true

	while true:
		_semaphore.wait()
		if _exit_flag:
			break

		_mutex.lock()
		var pos_bytes: PackedByteArray = _snap_positions
		var size_bytes: PackedByteArray = _snap_sizes
		var health_bytes: PackedByteArray = _snap_healths
		var state_bytes: PackedByteArray = _snap_states
		var target_bytes: PackedByteArray = _snap_targets
		var gc_bytes: PackedByteArray = _snap_grid_counts
		var gd_bytes: PackedByteArray = _snap_grid_data
		var count: int = _snap_count
		var grid_cells: int = _snap_grid_cells
		var max_per_cell: int = _snap_max_per_cell
		var grid_width: int = _snap_grid_width
		var grid_height: int = _snap_grid_height
		var cell_size: float = _snap_cell_size
		var sep_radius: float = _snap_separation_radius
		var sep_strength: float = _snap_separation_strength
		_mutex.unlock()

		if count == 0:
			continue

		# Ensure buffers
		_ensure_capacity(count, grid_cells, max_per_cell)

		# Upload NPC data (only count elements)
		var vec2_bytes: int = count * 8
		var scalar_bytes: int = count * 4
		rd.buffer_update(position_buffer, 0, vec2_bytes, pos_bytes)
		rd.buffer_update(size_buffer, 0, scalar_bytes, size_bytes)
		rd.buffer_update(health_buffer, 0, scalar_bytes, health_bytes)
		rd.buffer_update(state_buffer, 0, scalar_bytes, state_bytes)
		rd.buffer_update(target_buffer, 0, vec2_bytes, target_bytes)

		# Upload grid (full grid)
		rd.buffer_update(grid_counts_buffer, 0, grid_cells * 4, gc_bytes)
		rd.buffer_update(grid_data_buffer, 0, grid_cells * max_per_cell * 4, gd_bytes)

		# Push constants (32 bytes)
		push_constants.encode_u32(0, count)
		push_constants.encode_float(4, sep_radius)
		push_constants.encode_float(8, sep_strength)
		push_constants.encode_u32(12, 227)  # STATIONARY_MASK
		push_constants.encode_u32(16, grid_width)
		push_constants.encode_u32(20, grid_height)
		push_constants.encode_float(24, cell_size)
		push_constants.encode_u32(28, max_per_cell)

		# Dispatch
		var compute_list := rd.compute_list_begin()
		rd.compute_list_bind_compute_pipeline(compute_list, pipeline)
		rd.compute_list_bind_uniform_set(compute_list, uniform_set, 0)
		rd.compute_list_set_push_constant(compute_list, push_constants, push_constants.size())
		@warning_ignore("integer_division")
		var workgroups: int = (count + 63) / 64
		rd.compute_list_dispatch(compute_list, workgroups, 1, 1)
		rd.compute_list_end()

		rd.submit()
		rd.sync()

		# Read output
		var output_bytes := rd.buffer_get_data(output_buffer, 0, count * 8)
		var floats := output_bytes.to_float32_array()

		var new_result := PackedVector2Array()
		new_result.resize(count)
		for i in count:
			new_result[i] = Vector2(floats[i * 2], floats[i * 2 + 1])

		_mutex.lock()
		_result = new_result
		_mutex.unlock()

	# Cleanup on this thread
	_free_buffers()
	if pipeline.is_valid():
		rd.free_rid(pipeline)
	if shader.is_valid():
		rd.free_rid(shader)


func _ensure_capacity(npc_count: int, grid_cells: int, max_per_cell: int) -> void:
	var need_rebuild := false

	# NPC buffers
	if npc_count > _npc_capacity:
		_free_buffers()
		_npc_capacity = 1
		while _npc_capacity < npc_count:
			_npc_capacity *= 2
		need_rebuild = true

	# Grid buffers
	if grid_cells != _grid_cells or need_rebuild:
		if grid_counts_buffer.is_valid():
			if uniform_set.is_valid():
				rd.free_rid(uniform_set)
				uniform_set = RID()
			rd.free_rid(grid_counts_buffer)
			grid_counts_buffer = RID()
		if grid_data_buffer.is_valid():
			rd.free_rid(grid_data_buffer)
			grid_data_buffer = RID()
		_grid_cells = grid_cells
		need_rebuild = true

	if need_rebuild:
		if not position_buffer.is_valid():
			var pos_size: int = _npc_capacity * 8
			var scalar_size: int = _npc_capacity * 4
			position_buffer = rd.storage_buffer_create(pos_size)
			size_buffer = rd.storage_buffer_create(scalar_size)
			health_buffer = rd.storage_buffer_create(scalar_size)
			output_buffer = rd.storage_buffer_create(pos_size)
			state_buffer = rd.storage_buffer_create(scalar_size)
			target_buffer = rd.storage_buffer_create(pos_size)
		if not grid_counts_buffer.is_valid():
			grid_counts_buffer = rd.storage_buffer_create(_grid_cells * 4)
			grid_data_buffer = rd.storage_buffer_create(_grid_cells * max_per_cell * 4)

		# Rebuild uniform set
		if uniform_set.is_valid():
			rd.free_rid(uniform_set)
			uniform_set = RID()

		var uniforms: Array[RDUniform] = []
		var buffers: Array[RID] = [
			position_buffer, size_buffer, health_buffer, output_buffer,
			state_buffer, target_buffer, grid_counts_buffer, grid_data_buffer
		]
		for binding in buffers.size():
			var u := RDUniform.new()
			u.uniform_type = RenderingDevice.UNIFORM_TYPE_STORAGE_BUFFER
			u.binding = binding
			u.add_id(buffers[binding])
			uniforms.append(u)
		uniform_set = rd.uniform_set_create(uniforms, shader, 0)


func _free_buffers() -> void:
	if uniform_set.is_valid():
		rd.free_rid(uniform_set)
		uniform_set = RID()
	for buf: RID in [position_buffer, size_buffer, health_buffer, output_buffer,
			state_buffer, target_buffer, grid_counts_buffer, grid_data_buffer]:
		if buf.is_valid():
			rd.free_rid(buf)
	position_buffer = RID()
	size_buffer = RID()
	health_buffer = RID()
	output_buffer = RID()
	state_buffer = RID()
	target_buffer = RID()
	grid_counts_buffer = RID()
	grid_data_buffer = RID()
	_npc_capacity = 0
	_grid_cells = 0


func cleanup() -> void:
	if _thread == null:
		return
	_exit_flag = true
	_semaphore.post()
	_thread.wait_to_finish()
	is_initialized = false
