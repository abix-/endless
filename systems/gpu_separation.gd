# gpu_separation.gd
# GPU compute shader for NPC separation - async threaded with spatial grid
extends RefCounted
class_name GPUSeparation

var rd: RenderingDevice
var shader: RID
var pipeline: RID

# Buffers (bindings 0-8)
var position_buffer: RID
var size_buffer: RID
var health_buffer: RID
var output_buffer: RID
var state_buffer: RID
var target_buffer: RID
var neighbor_starts_buffer: RID
var neighbor_counts_buffer: RID
var neighbor_data_buffer: RID
var uniform_set: RID

# Buffer capacities
var _npc_capacity: int = 0
var _neighbor_data_capacity: int = 0

# Push constants (16 bytes: uint + float + float + uint)
var push_constants: PackedByteArray

# Threading
var _thread: Thread
var _semaphore: Semaphore
var _mutex: Mutex
var _exit_flag: bool = false

# Shader data (loaded on main thread, used by worker to create pipeline)
var _shader_spirv: RDShaderSPIRV

# Double-buffered results
var _result: PackedVector2Array  # Last completed result (main thread reads)

# Snapshot data for GPU thread (written by main, read by thread)
var _snap_positions: PackedByteArray
var _snap_sizes: PackedByteArray
var _snap_healths: PackedByteArray
var _snap_states: PackedByteArray
var _snap_targets: PackedByteArray
var _snap_neighbor_starts: PackedByteArray
var _snap_neighbor_counts: PackedByteArray
var _snap_neighbor_data: PackedByteArray
var _snap_count: int = 0
var _snap_neighbor_data_size: int = 0
var _snap_separation_radius: float = 0.0
var _snap_separation_strength: float = 0.0

var is_initialized := false


func _init() -> void:
	push_constants = PackedByteArray()
	push_constants.resize(16)
	_result = PackedVector2Array()
	_mutex = Mutex.new()
	_semaphore = Semaphore.new()


func initialize() -> bool:
	# Load shader on main thread (resource loading is safe here)
	var shader_file := load("res://shaders/separation_compute.glsl")
	if shader_file == null:
		push_error("GPUSeparation: Failed to load shader file")
		return false

	_shader_spirv = shader_file.get_spirv()
	if _shader_spirv == null:
		push_error("GPUSeparation: Failed to get SPIRV")
		return false

	# Start worker thread (non-blocking — thread sets is_initialized when ready)
	_thread = Thread.new()
	_thread.start(_thread_func)
	return true


func kick(
	positions: PackedVector2Array,
	sizes: PackedFloat32Array,
	healths: PackedFloat32Array,
	states: PackedInt32Array,
	targets: PackedVector2Array,
	neighbor_starts: PackedInt32Array,
	neighbor_counts: PackedInt32Array,
	neighbor_data: PackedInt32Array,
	count: int,
	separation_radius: float,
	separation_strength: float
) -> void:
	if not is_initialized:
		return

	# Snapshot data for the GPU thread (full arrays, thread uploads only count elements)
	_mutex.lock()
	_snap_positions = positions.to_byte_array()
	_snap_sizes = sizes.to_byte_array()
	_snap_healths = healths.to_byte_array()
	_snap_states = states.to_byte_array()
	_snap_targets = targets.to_byte_array()
	_snap_neighbor_starts = neighbor_starts.to_byte_array()
	_snap_neighbor_counts = neighbor_counts.to_byte_array()
	_snap_neighbor_data = neighbor_data.to_byte_array()
	_snap_count = count
	_snap_neighbor_data_size = neighbor_data.size()
	_snap_separation_radius = separation_radius
	_snap_separation_strength = separation_strength
	_mutex.unlock()

	# Signal thread to process
	_semaphore.post()


func get_result() -> PackedVector2Array:
	return _result


func _thread_func() -> void:
	# Create RenderingDevice on this thread (thread-bound requirement)
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

	# Work loop
	while true:
		_semaphore.wait()
		if _exit_flag:
			break

		# Read snapshot
		_mutex.lock()
		var pos_bytes: PackedByteArray = _snap_positions
		var size_bytes: PackedByteArray = _snap_sizes
		var health_bytes: PackedByteArray = _snap_healths
		var state_bytes: PackedByteArray = _snap_states
		var target_bytes: PackedByteArray = _snap_targets
		var ns_bytes: PackedByteArray = _snap_neighbor_starts
		var nc_bytes: PackedByteArray = _snap_neighbor_counts
		var nd_bytes: PackedByteArray = _snap_neighbor_data
		var count: int = _snap_count
		var nd_size: int = _snap_neighbor_data_size
		var sep_radius: float = _snap_separation_radius
		var sep_strength: float = _snap_separation_strength
		_mutex.unlock()

		if count == 0:
			continue

		# Ensure GPU buffers are sized
		_ensure_npc_capacity(count)
		_ensure_neighbor_data_capacity(nd_size)

		# Upload to GPU (only count elements, not full array)
		var vec2_bytes: int = count * 8
		var scalar_bytes: int = count * 4
		rd.buffer_update(position_buffer, 0, vec2_bytes, pos_bytes)
		rd.buffer_update(size_buffer, 0, scalar_bytes, size_bytes)
		rd.buffer_update(health_buffer, 0, scalar_bytes, health_bytes)
		rd.buffer_update(state_buffer, 0, scalar_bytes, state_bytes)
		rd.buffer_update(target_buffer, 0, vec2_bytes, target_bytes)
		rd.buffer_update(neighbor_starts_buffer, 0, scalar_bytes, ns_bytes)
		rd.buffer_update(neighbor_counts_buffer, 0, scalar_bytes, nc_bytes)
		if nd_size > 0:
			rd.buffer_update(neighbor_data_buffer, 0, nd_size * 4, nd_bytes)

		# Push constants
		push_constants.encode_u32(0, count)
		push_constants.encode_float(4, sep_radius)
		push_constants.encode_float(8, sep_strength)
		push_constants.encode_u32(12, 227)  # STATIONARY_MASK

		# Dispatch
		var compute_list := rd.compute_list_begin()
		rd.compute_list_bind_compute_pipeline(compute_list, pipeline)
		rd.compute_list_bind_uniform_set(compute_list, uniform_set, 0)
		rd.compute_list_set_push_constant(compute_list, push_constants, push_constants.size())
		@warning_ignore("integer_division")
		var workgroups: int = (count + 63) / 64
		rd.compute_list_dispatch(compute_list, workgroups, 1, 1)
		rd.compute_list_end()

		# Submit and wait (blocks this thread only, not main)
		rd.submit()
		rd.sync()

		# Read output
		var output_bytes := rd.buffer_get_data(output_buffer, 0, count * 8)
		var floats := output_bytes.to_float32_array()

		# Build result
		var new_result := PackedVector2Array()
		new_result.resize(count)
		for i in count:
			new_result[i] = Vector2(floats[i * 2], floats[i * 2 + 1])

		# Swap result
		_mutex.lock()
		_result = new_result
		_mutex.unlock()

	# Cleanup GPU resources on this thread (same thread that created them)
	_free_buffers()
	if pipeline.is_valid():
		rd.free_rid(pipeline)
	if shader.is_valid():
		rd.free_rid(shader)


func _ensure_npc_capacity(count: int) -> void:
	if count <= _npc_capacity:
		return

	_free_buffers()

	_npc_capacity = 1
	while _npc_capacity < count:
		_npc_capacity *= 2

	var pos_size: int = _npc_capacity * 8   # vec2 = 8 bytes
	var scalar_size: int = _npc_capacity * 4  # float/int = 4 bytes

	position_buffer = rd.storage_buffer_create(pos_size)
	size_buffer = rd.storage_buffer_create(scalar_size)
	health_buffer = rd.storage_buffer_create(scalar_size)
	output_buffer = rd.storage_buffer_create(pos_size)
	state_buffer = rd.storage_buffer_create(scalar_size)
	target_buffer = rd.storage_buffer_create(pos_size)
	neighbor_starts_buffer = rd.storage_buffer_create(scalar_size)
	neighbor_counts_buffer = rd.storage_buffer_create(scalar_size)

	# neighbor_data gets its own capacity
	if _neighbor_data_capacity == 0:
		_neighbor_data_capacity = _npc_capacity * 16
	neighbor_data_buffer = rd.storage_buffer_create(_neighbor_data_capacity * 4)

	_rebuild_uniform_set()


func _ensure_neighbor_data_capacity(size: int) -> void:
	if size <= _neighbor_data_capacity:
		return

	# Free uniform_set first (it references the buffer)
	if uniform_set.is_valid():
		rd.free_rid(uniform_set)
		uniform_set = RID()
	if neighbor_data_buffer.is_valid():
		rd.free_rid(neighbor_data_buffer)

	_neighbor_data_capacity = 1
	while _neighbor_data_capacity < size:
		_neighbor_data_capacity *= 2

	neighbor_data_buffer = rd.storage_buffer_create(_neighbor_data_capacity * 4)
	_rebuild_uniform_set()


func _rebuild_uniform_set() -> void:
	if uniform_set.is_valid():
		rd.free_rid(uniform_set)
		uniform_set = RID()

	var uniforms: Array[RDUniform] = []
	var buffers: Array[RID] = [
		position_buffer, size_buffer, health_buffer, output_buffer,
		state_buffer, target_buffer,
		neighbor_starts_buffer, neighbor_counts_buffer, neighbor_data_buffer
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
			state_buffer, target_buffer,
			neighbor_starts_buffer, neighbor_counts_buffer, neighbor_data_buffer]:
		if buf.is_valid():
			rd.free_rid(buf)

	position_buffer = RID()
	size_buffer = RID()
	health_buffer = RID()
	output_buffer = RID()
	state_buffer = RID()
	target_buffer = RID()
	neighbor_starts_buffer = RID()
	neighbor_counts_buffer = RID()
	neighbor_data_buffer = RID()
	_npc_capacity = 0


func cleanup() -> void:
	if _thread == null:
		return

	# Signal thread to exit (thread handles its own GPU cleanup)
	_exit_flag = true
	_semaphore.post()
	_thread.wait_to_finish()

	is_initialized = false
