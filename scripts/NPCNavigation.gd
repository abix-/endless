# NPCNavigation.gd
# Handles all navigation-related behavior for NPCs
extends Node
class_name NPCNavigation

# Navigation properties
@export_group("Navigation")
@export var enabled: bool = true
@export var movement_speed: float = 10.0
@export var obstacle_avoidance_speed: float = 5.0
@export var repath_interval: float = 2.5
@export var stuck_threshold: float = 0.2
@export var arrival_distance: float = 1.0
@export var draw_navigation_path: bool = true

# Navigation agent node
@onready var nav_agent = $NavigationAgent3D

# Navigation tracking
var owner_npc = null
var last_position = Vector3.ZERO
var time_since_repath = 0.0
var same_position_time = 0.0
var current_destination = ""
var destination_position = Vector3.ZERO
var has_moved = false
var navigation_path_line = null

# Signals
signal navigation_finished

# Initialize with owner reference
func initialize(npc):
	owner_npc = npc
	
	# Set up navigation agent
	nav_agent.path_desired_distance = arrival_distance
	nav_agent.target_desired_distance = arrival_distance
	nav_agent.avoidance_enabled = true
	
	# Connect navigation signals
	nav_agent.velocity_computed.connect(_on_velocity_computed)
	nav_agent.navigation_finished.connect(_on_navigation_finished)
	nav_agent.path_changed.connect(_on_path_changed)
	
	# Initialize position tracking
	last_position = owner_npc.global_position
	
	# Create navigation path visualization
	if draw_navigation_path:
		navigation_path_line = _create_path_visualization()
	
	# Connect to state changes
	owner_npc.state_changed.connect(_on_state_changed)

# Process navigation in physics update
func process_navigation(delta):
	# Update path visualization if needed
	if draw_navigation_path:
		_update_path_visualization()
	
	# Check if we've reached the destination
	if nav_agent.is_navigation_finished():
		return
	
	# Check if the path needs to be recalculated
	time_since_repath += delta
	
	# Measure movement
	var distance_moved = owner_npc.global_position.distance_to(last_position)
	last_position = owner_npc.global_position
	
	# Check if we're stuck (not moving much)
	if distance_moved < stuck_threshold:
		same_position_time += delta
	else:
		same_position_time = 0.0
	
	# Recalculate path if we've been stuck or it's time for a regular repath
	if (same_position_time > 1.0) or (time_since_repath >= repath_interval):
		time_since_repath = 0.0
		# Logic for handling being stuck would go here
	
	# Get next path position
	var next_position = nav_agent.get_next_path_position()
	
	# Calculate direction to the next position
	var direction = (next_position - owner_npc.global_position).normalized()
	
	# Calculate velocity
	var new_velocity = direction * movement_speed
	
	# Ensure we're moving on a flat plane (no flying/sinking)
	new_velocity.y = 0
	
	# Send the velocity to the navigation agent for avoidance processing
	if new_velocity.length() > 0:
		nav_agent.velocity = new_velocity
		has_moved = true

# Navigate to a position
func navigate_to(target_position):
	destination_position = target_position
	nav_agent.target_position = target_position
	
	# Update visualization
	if draw_navigation_path:
		_update_path_visualization()

# Create a line to visualize the navigation path
func _create_path_visualization():
	var line = MeshInstance3D.new()
	var immediate_mesh = ImmediateMesh.new()
	line.mesh = immediate_mesh
	
	var material = StandardMaterial3D.new()
	material.albedo_color = Color(0, 1, 0)  # Green line
	material.flags_unshaded = true
	material.vertex_color_use_as_albedo = true
	line.material_override = material
	
	add_child(line)
	return line

# Update the navigation path visualization
func _update_path_visualization():
	if not draw_navigation_path or not navigation_path_line:
		return
	
	var path = nav_agent.get_current_navigation_path()
	if path.size() < 2:
		return
	
	var immediate_mesh = navigation_path_line.mesh as ImmediateMesh
	immediate_mesh.clear_surfaces()
	
	immediate_mesh.surface_begin(Mesh.PRIMITIVE_LINE_STRIP)
	for point in path:
		# Raise the line slightly above the ground to be visible
		var point_above = point + Vector3(0, 0.1, 0)
		immediate_mesh.surface_add_vertex(point_above - owner_npc.global_position)
	immediate_mesh.surface_end()

# Called when the navigation path changes
func _on_path_changed():
	# Update the path visualization right away when the path changes
	if draw_navigation_path:
		_update_path_visualization()

# Called when navigation computes a new velocity
func _on_velocity_computed(safe_velocity):
	# Apply the computed velocity
	owner_npc.velocity = safe_velocity
	owner_npc.move_and_slide()
	has_moved = true
	
	# Face movement direction if we're moving
	if owner_npc.velocity.length() > 0.1:
		var look_dir = Vector3(owner_npc.velocity.x, 0, owner_npc.velocity.z).normalized()
		if look_dir.length() > 0:
			owner_npc.look_at(owner_npc.global_position + look_dir, Vector3.UP)

# Called when navigation finishes
func _on_navigation_finished():
	# Emit signal to notify the owner NPC
	emit_signal("navigation_finished")

# React to state changes
func _on_state_changed(old_state, new_state):
	# Could adjust navigation parameters based on state
	pass

# Check if a position is reachable
func is_position_reachable(target_position):
	var temp_target = nav_agent.target_position
	nav_agent.target_position = target_position
	var reachable = nav_agent.is_target_reachable()
	nav_agent.target_position = temp_target
	return reachable
