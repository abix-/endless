# NPCBase.gd
# Base class for all AI-controlled NPCs with navigation, collision handling and needs systems
extends CharacterBody3D
class_name NPCBase

# Navigation properties
@export_group("Navigation")
@export var movement_speed: float = 10.0
@export var obstacle_avoidance_speed: float = 5.0
@export var repath_interval: float = 1.0
@export var stuck_threshold: float = 0.2
@export var arrival_distance: float = 1.0
@export var blocked_path_wait_time: float = 3.0
@export var obstacle_detection_distance: float = 2.0
@export var obstacle_avoidance_radius: float = 3.0
@export var collision_recovery_time: float = 1.0
@export var collision_cooldown: float = 2.0
@export var debug_navigation: bool = false
@export var debug_position: bool = false
@export var draw_navigation_path: bool = true

# Ignore collisions with these object names
@export var ignored_colliders: Array[String] = ["Ground", "Terrain", "Floor", "Land"]

# Needs system
@export_group("Needs")
@export var max_food: float = 100.0
@export var food: float = 50.0
@export var food_depletion_rate: float = 8.0

# Object references
@onready var nav_agent = $NavigationAgent3D
var state_machine = null  # Will be initialized by derived classes
var time_manager = null

# Navigation tracking
var last_position = Vector3.ZERO
var time_since_repath = 0.0
var same_position_time = 0.0
var debug_timer = 0.0
var path_blocked_timer = 0.0
var obstacle_check_timer = 0.0
var collision_recovery_timer = 0.0
var collision_cooldown_timer = 0.0
var startup_delay = 1.0
var current_destination = ""
var destination_position = Vector3.ZERO
var last_collider = null
var collision_direction = Vector3.ZERO
var avoidance_direction = Vector3.ZERO
var avoidance_target = Vector3.ZERO
var navigation_path_line = null
var has_moved = false
var recovery_attempts = 0
var max_recovery_attempts = 3

# Obstacle detection
@onready var obstacle_raycasts = []
var raycast_directions = [
	Vector3(1, 0, 0),    # Right
	Vector3(-1, 0, 0),   # Left
	Vector3(0, 0, 1),    # Forward
	Vector3(0, 0, -1),   # Back
	Vector3(0.7, 0, 0.7),  # Forward-Right
	Vector3(-0.7, 0, 0.7), # Forward-Left
	Vector3(0.7, 0, -0.7), # Back-Right
	Vector3(-0.7, 0, -0.7) # Back-Left
]

func _ready():
	print("NPCBase._ready called for ", get_npc_type())
	
	# Note: State machine is now initialized in derived classes
	
	# Set up navigation agent
	nav_agent.path_desired_distance = arrival_distance
	nav_agent.target_desired_distance = arrival_distance
	nav_agent.avoidance_enabled = true
	
	# Set up raycasts for obstacle detection
	_setup_obstacle_detection()
	
	# Connect navigation signals
	nav_agent.velocity_computed.connect(_on_velocity_computed)
	nav_agent.navigation_finished.connect(_on_navigation_finished)
	nav_agent.path_changed.connect(_on_path_changed)
	
	# Initialize position tracking
	last_position = global_position
	
	# Create navigation path visualization
	if draw_navigation_path:
		navigation_path_line = _create_path_visualization()
	
	# Initialize NPC-specific behavior
	print("NPCBase calling _init_npc")
	_init_npc()
	
	# Print state machine info if it exists
	if state_machine:
		print("State machine initialization complete. Current state: ", 
			 state_machine.get_state_name(state_machine.current_state))
	else:
		print("WARNING: No state machine initialized!")

# Virtual method to be overridden by derived classes
func _init_npc():
	print("NPCBase._init_npc called - should be overridden by derived class")
	pass

func _physics_process(delta):
	# Process startup delay
	if startup_delay > 0:
		startup_delay -= delta
	
	# Debug timer
	debug_timer += delta
	if debug_position and debug_timer >= 2.0:
		debug_timer = 0.0
		print("Current position: ", global_position)
	
	# Update collision cooldown
	if collision_cooldown_timer > 0:
		collision_cooldown_timer -= delta
	
	# Update needs
	_update_needs(delta)
	
	# Update state machine
	if state_machine:  # Check if state machine exists
		state_machine.update(delta)
	else:
		print("WARNING: No state machine in _physics_process!")
		return
	
	# Process behavior based on current state
	_process_current_state(delta)
	
	# Check for obstacles periodically when moving
	if state_machine and state_machine.is_movement_state():
		obstacle_check_timer += delta
		if obstacle_check_timer >= 0.5:
			obstacle_check_timer = 0
			_check_for_obstacles()
	
	# Check for collisions if we've moved
	if startup_delay <= 0 and has_moved and collision_cooldown_timer <= 0:
		_check_for_collisions()

# To be implemented by derived classes
func _process_current_state(delta):
	pass

# Set up raycasts for obstacle detection
func _setup_obstacle_detection():
	# Remove any existing raycasts
	for raycast in obstacle_raycasts:
		if is_instance_valid(raycast):
			raycast.queue_free()
	obstacle_raycasts.clear()
	
	# Create raycasts in multiple directions
	for direction in raycast_directions:
		var ray = RayCast3D.new()
		ray.enabled = true
		ray.exclude_parent = true
		ray.target_position = direction.normalized() * obstacle_detection_distance
		ray.collision_mask = 1  # Set to appropriate collision mask for obstacles
		add_child(ray)
		obstacle_raycasts.append(ray)
	
	print("Set up ", obstacle_raycasts.size(), " obstacle detection raycasts")

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
	var immediate_mesh = navigation_path_line.mesh as ImmediateMesh
	
	immediate_mesh.clear_surfaces()
	if path.size() < 2:
		return
	
	immediate_mesh.surface_begin(Mesh.PRIMITIVE_LINE_STRIP)
	for point in path:
		# Raise the line slightly above the ground to be visible
		var point_above = point + Vector3(0, 0.1, 0)
		immediate_mesh.surface_add_vertex(point_above - global_position)
	immediate_mesh.surface_end()

# Update needs (hunger, etc.)
func _update_needs(delta):
	# Base implementation - just handle food depletion
	food -= delta * food_depletion_rate * 0.5
	food = clamp(food, 0, max_food)

# Navigate to a position
func navigate_to(target_position):
	destination_position = target_position
	nav_agent.target_position = target_position
	if debug_navigation:
		print("Setting navigation target to: ", target_position)
	
	# Update visualization
	if draw_navigation_path:
		_update_path_visualization()

# Check for collisions
func _check_for_collisions():
	if get_slide_collision_count() > 0:
		var current_collision = get_slide_collision(0)
		var collider = current_collision.get_collider()
		
		if collider:
			# Check if this is the same collision we just handled
			if collider == last_collider and collision_cooldown_timer > 0:
				return
				
			# Check if this is a ground-type object that should be ignored
			var should_ignore = false
			for ignore_name in ignored_colliders:
				if collider.name.contains(ignore_name):
					should_ignore = true
					break
			
			if not should_ignore:
				print("Collision detected with: ", collider.name)
				last_collider = collider
				collision_cooldown_timer = collision_cooldown
				
				# For vertical collisions (like jumping into ceiling or falling onto floor),
				# we don't want to trigger avoidance - only for horizontal obstacles
				var normal = current_collision.get_normal()
				if abs(normal.y) < 0.8:  # Not a mostly vertical collision
					_handle_collision_event(current_collision)

# Handle collision event
func _handle_collision_event(collision):
	# Save the direction we were moving in when the collision occurred
	collision_direction = velocity.normalized()
	if collision_direction.length() == 0:
		# If we weren't moving, use the direction from us to the collider
		collision_direction = (global_position - collision.get_position()).normalized()
	
	print("Collision occurred while moving in direction: ", collision_direction)
	
	# Check if we're already in recovery and increment attempts counter
	if state_machine and state_machine.is_in_state(NPCStates.BaseState.COLLISION_RECOVERY):
		recovery_attempts += 1
		
		# If we've tried too many times, take more drastic action
		if recovery_attempts >= max_recovery_attempts:
			print("Multiple collision recovery attempts failed. Taking evasive action!")
			_handle_persistent_collision()
			return
	else:
		recovery_attempts = 0
	
	# Enter collision recovery mode
	if state_machine:
		state_machine.change_state(NPCStates.BaseState.COLLISION_RECOVERY)

# Handle persistent collisions
func _handle_persistent_collision():
	# Teleport slightly away from current position in a random direction
	var random_direction = Vector3(randf_range(-1, 1), 0, randf_range(-1, 1)).normalized()
	
	# Move a decent distance away
	var teleport_distance = 3.0
	var new_position = global_position + random_direction * teleport_distance
	
	print("Emergency teleport to escape persistent collision. New position: ", new_position)
	global_position = new_position
	
	# Reset recovery attempts and proceed with previous route
	recovery_attempts = 0
	collision_cooldown_timer = collision_cooldown * 2  # Double cooldown for teleport
	
	# Return to previous movement state or idle
	if state_machine:
		state_machine.change_state(NPCStates.BaseState.IDLE)

# Handle collision recovery state
func _handle_collision_recovery(delta):
	collision_recovery_timer += delta
	
	if collision_recovery_timer >= collision_recovery_time:
		# We've backed up enough, try to navigate around the obstacle
		collision_recovery_timer = 0
		recovery_attempts = 0  # Reset attempts counter
		print("Collision recovery complete, finding new path")
		if state_machine:
			state_machine.change_state(NPCStates.BaseState.NAVIGATING_OBSTACLE)
		return
	
	# Move away from the collision in the opposite direction
	var recovery_direction = -collision_direction.normalized()
	
	# Keep the movement on the ground plane
	recovery_direction.y = 0
	if recovery_direction.length() > 0:
		recovery_direction = recovery_direction.normalized()
		
		# Move at a slower speed while recovering
		velocity = recovery_direction * obstacle_avoidance_speed
		move_and_slide()
		
		# Face the direction we're backing up
		if velocity.length() > 0.1:
			# We want to face forward, not backward, even though we're backing up
			var look_dir = -recovery_direction
			look_at(global_position + look_dir, Vector3.UP)
	
	# If we hit something else while backing up, try a different direction
	if get_slide_collision_count() > 0 and collision_cooldown_timer <= 0:
		var collision = get_slide_collision(0)
		var collider = collision.get_collider()
		
		# Ignore ground collisions during recovery
		var should_ignore = false
		for ignore_name in ignored_colliders:
			if collider and collider.name.contains(ignore_name):
				should_ignore = true
				break
				
		if not should_ignore and collider != last_collider:
			# Calculate a new avoidance direction
			_calculate_randomized_avoidance_direction()
			collision_direction = -avoidance_direction
			print("Hit something while backing up, changing direction to: ", collision_direction)
			last_collider = collider
			collision_cooldown_timer = collision_cooldown

# Calculate a randomized direction to move around an obstacle
func _calculate_randomized_avoidance_direction():
	# Add some randomness to obstacle avoidance
	var random_angle = randf_range(-PI/4, PI/4)  # Random angle between -45 and 45 degrees
	
	# Get information from raycasts to determine which directions are blocked
	var blocked_directions = []
	var clearest_direction = Vector3.ZERO
	var longest_distance = 0
	
	for i in range(obstacle_raycasts.size()):
		var ray = obstacle_raycasts[i]
		var dir = raycast_directions[i].normalized()
		
		if ray.is_colliding():
			var collider = ray.get_collider()
			
			# Ignore ground-type colliders
			var should_ignore = false
			for ignore_name in ignored_colliders:
				if collider and collider.name.contains(ignore_name):
					should_ignore = true
					break
			
			if not should_ignore:
				var collision_point = ray.get_collision_point()
				var distance = global_position.distance_to(collision_point)
				blocked_directions.append({
					"direction": dir,
					"distance": distance
				})
		else:
			# If this direction is clear and has a longer clear distance
			var distance = obstacle_detection_distance
			if distance > longest_distance:
				longest_distance = distance
				clearest_direction = dir
	
	if clearest_direction != Vector3.ZERO:
		# Move in the clearest direction with some randomness
		var rot_matrix = Basis(Vector3(0, 1, 0), random_angle)
		avoidance_direction = rot_matrix * clearest_direction
	else:
		# All directions have obstacles, try to move perpendicular to the closest one
		var closest_obstacle = null
		var closest_distance = 9999
		
		for blocked in blocked_directions:
			if blocked.distance < closest_distance:
				closest_distance = blocked.distance
				closest_obstacle = blocked.direction
		
		if closest_obstacle != null:
			# Move perpendicular to the closest obstacle with randomness
			var perp_dir = Vector3(closest_obstacle.z, 0, -closest_obstacle.x)
			var rot_matrix = Basis(Vector3(0, 1, 0), random_angle)
			avoidance_direction = rot_matrix * perp_dir
		else:
			# If all else fails, pick a completely random direction
			avoidance_direction = Vector3(randf_range(-1, 1), 0, randf_range(-1, 1)).normalized()
	
	# Calculate a target position to move toward
	avoidance_target = global_position + avoidance_direction * obstacle_avoidance_radius
	
	print("Calculated avoidance direction: ", avoidance_direction)
	print("Setting temporary avoidance target: ", avoidance_target)

# Handle obstacle avoidance state
func _handle_obstacle_avoidance(delta):
	# If we don't have an avoidance direction yet, pick one
	if avoidance_direction == Vector3.ZERO:
		_calculate_randomized_avoidance_direction()
	
	# Check if we've reached our temporary avoidance target
	if global_position.distance_to(avoidance_target) < 1.0:
		print("Reached temporary avoidance target. Retrying main navigation.")
		
		# Try normal navigation again - return to previous movement state
		if state_machine:
			state_machine.change_state(NPCStates.BaseState.IDLE)
		return
	
	# Move toward our avoidance target
	var direction = (avoidance_target - global_position).normalized()
	direction.y = 0  # Stay on ground plane
	
	# Apply movement at a slower speed for more control
	velocity = direction * obstacle_avoidance_speed
	move_and_slide()
	has_moved = true
	
	# Face movement direction
	if velocity.length() > 0.1:
		var look_dir = Vector3(velocity.x, 0, velocity.z).normalized()
		if look_dir.length() > 0:
			look_at(global_position + look_dir, Vector3.UP)
	
	# Check if we hit something during avoidance
	if get_slide_collision_count() > 0 and collision_cooldown_timer <= 0:
		var collision = get_slide_collision(0)
		var collider = collision.get_collider()
		
		# Ignore ground collisions during avoidance
		var should_ignore = false
		for ignore_name in ignored_colliders:
			if collider and collider.name.contains(ignore_name):
				should_ignore = true
				break
				
		if not should_ignore and collider != last_collider:
			# If we hit something while avoiding, try a different direction
			print("Hit something while avoiding obstacles, recalculating...")
			avoidance_direction = Vector3.ZERO  # Reset so we'll recalculate
			_calculate_randomized_avoidance_direction()
			last_collider = collider
			collision_cooldown_timer = collision_cooldown
	
	# Periodically check if we can now find a path to the destination
	obstacle_check_timer += delta
	if obstacle_check_timer >= 1.0:
		obstacle_check_timer = 0
		
		# See if we can now find a path to the destination
		nav_agent.target_position = destination_position
		
		# If path is clear now, return to previous movement state
		if nav_agent.is_target_reachable():
			print("Path is now clear! Returning to normal navigation.")
			if state_machine:
				state_machine.change_state(NPCStates.BaseState.IDLE)

# Handle waiting for a path to clear
func _handle_path_waiting(delta):
	path_blocked_timer += delta
	if path_blocked_timer >= blocked_path_wait_time:
		path_blocked_timer = 0
		
		# Check if path is now clear
		print("Checking if path is clear now...")
		if nav_agent.is_target_reachable():
			print("Path is now clear! Resuming navigation.")
			
			# Return to previous movement state
			if state_machine:
				state_machine.change_state(NPCStates.BaseState.IDLE)
		else:
			print("Path is still blocked. Trying to find an alternate route...")
			if state_machine:
				state_machine.change_state(NPCStates.BaseState.NAVIGATING_OBSTACLE)

# Handle standard navigation
func _handle_navigation(delta):
	# Check if we've reached the destination
	if nav_agent.is_navigation_finished():
		_on_navigation_finished()
		return
	
	# Update the path visualization if needed
	if draw_navigation_path:
		_update_path_visualization()
	
	# Check if the path needs to be recalculated
	time_since_repath += delta
	
	# Measure movement
	var distance_moved = global_position.distance_to(last_position)
	last_position = global_position
	
	# Check if we're stuck (not moving much)
	if distance_moved < stuck_threshold:
		same_position_time += delta
	else:
		same_position_time = 0.0
	
	# Recalculate path if we've been stuck or it's time for a regular repath
	if (same_position_time > 1.0) or (time_since_repath >= repath_interval):
		time_since_repath = 0.0
		
		# If we've been stuck for a while, something might be blocking our path
		if same_position_time > 1.0:
			print("NPC appears to be stuck, checking for obstacles...")
			print("Current position: ", global_position, " Target position: ", nav_agent.target_position)
			same_position_time = 0.0
			
			# Check if we can't find a path to the destination anymore
			if not nav_agent.is_target_reachable():
				print("Target is no longer reachable. Waiting for path to clear...")
				if state_machine:
					state_machine.change_state(NPCStates.BaseState.WAITING_FOR_PATH)
				return
			
			# Check for obstacles that might be blocking us
			_check_for_obstacles()
	
	# Get next path position
	var next_position = nav_agent.get_next_path_position()
	
	# Debug the path
	if debug_navigation:
		print("Nav debug - Target:", nav_agent.target_position, 
			  " Next:", next_position,
			  " Distance:", nav_agent.distance_to_target())
	
	# Calculate direction to the next position
	var direction = (next_position - global_position).normalized()
	
	# Calculate velocity
	var new_velocity = direction * movement_speed
	
	# Ensure we're moving on a flat plane (no flying/sinking)
	new_velocity.y = 0
	
	# Send the velocity to the navigation agent for avoidance processing
	if new_velocity.length() > 0:
		nav_agent.velocity = new_velocity
		has_moved = true
	else:
		# If we have no velocity, just move directly toward the target (fallback)
		var direct_dir = (nav_agent.target_position - global_position).normalized()
		direct_dir.y = 0
		if direct_dir.length() > 0:
			nav_agent.velocity = direct_dir * movement_speed
			has_moved = true
			
			if debug_navigation:
				print("Using direct movement as fallback")

# Check for obstacles in the path
func _check_for_obstacles():
	for ray in obstacle_raycasts:
		if ray.is_colliding():
			var collider = ray.get_collider()
			if collider:
				# Ignore ground-type colliders
				var should_ignore = false
				for ignore_name in ignored_colliders:
					if collider.name.contains(ignore_name):
						should_ignore = true
						break
				
				if not should_ignore:
					var collision_point = ray.get_collision_point()
					var distance = global_position.distance_to(collision_point)
					
					if distance < obstacle_detection_distance:
						print("Detected obstacle: ", collider.name, " at distance ", distance)
						
						# Only perform avoidance if we're actually moving
						if velocity.length() > 0.5 and state_machine:
							state_machine.change_state(NPCStates.BaseState.NAVIGATING_OBSTACLE)
							return true
	
	# No obstacles detected
	return false

# Called when navigation finishes
func _on_navigation_finished():
	print("Navigation finished at position: ", global_position)
	# To be implemented by derived classes

# Called when the navigation path changes
func _on_path_changed():
	if debug_navigation:
		print("Path changed. Path size: ", nav_agent.get_current_navigation_path().size())
	
	# Update the path visualization
	_update_path_visualization()

# Called when navigation computes a new velocity
func _on_velocity_computed(safe_velocity):
	if debug_navigation and safe_velocity.length() > 0:
		print("Computed velocity: ", safe_velocity)
	
	# Apply the computed velocity
	velocity = safe_velocity
	move_and_slide()
	has_moved = true
	
	# Face movement direction if we're moving
	if velocity.length() > 0.1:
		var look_dir = Vector3(velocity.x, 0, velocity.z).normalized()
		if look_dir.length() > 0:
			look_at(global_position + look_dir, Vector3.UP)

# Called when state changes
func _on_state_changed(old_state, new_state):
	# Reset state-specific timers and properties
	if new_state == NPCStates.BaseState.NAVIGATING_OBSTACLE:
		avoidance_direction = Vector3.ZERO
		obstacle_check_timer = 0.0
	elif new_state == NPCStates.BaseState.WAITING_FOR_PATH:
		path_blocked_timer = 0.0
	elif new_state == NPCStates.BaseState.COLLISION_RECOVERY:
		collision_recovery_timer = 0.0
	
	# Inform derived classes
	_handle_state_change(old_state, new_state)

# Handle state change (to be overridden by derived classes)
func _handle_state_change(old_state, new_state):
	pass

# Get location name based on position (to be implemented by derived classes)
func _get_location_name(position):
	return "unknown location"

# Return class name for debugging 
func get_npc_type():
	return "NPCBase"
