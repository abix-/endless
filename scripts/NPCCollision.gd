# NPCCollision.gd
# Handles all collision detection and response for NPCs
extends Node
class_name NPCCollision

# Collision properties
@export_group("Collision")
@export var enabled: bool = true
@export var blocked_path_wait_time: float = 3.0
@export var obstacle_detection_distance: float = 2.0
@export var obstacle_avoidance_radius: float = 3.0
@export var collision_recovery_time: float = 1.0
@export var collision_cooldown: float = 2.0

# Ignore collisions with these object names
@export var ignored_colliders: Array[String] = ["Ground", "Terrain", "Floor", "Land"]

# Collision tracking
var owner_npc = null
var collision_cooldown_timer = 0.0
var collision_recovery_timer = 0.0
var last_collider = null
var collision_direction = Vector3.ZERO
var avoidance_direction = Vector3.ZERO
var avoidance_target = Vector3.ZERO
var recovery_attempts = 0
var max_recovery_attempts = 3

# Signals
signal collision_detected(collision, collision_direction)
signal obstacle_detected(direction, distance)

# Obstacle detection
var obstacle_raycasts = []
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

# Initialize with owner reference
func initialize(npc):
	owner_npc = npc
	
	# Set up raycasts for obstacle detection
	_setup_obstacle_detection()
	
	# Connect to state changes
	owner_npc.state_changed.connect(_on_state_changed)

# Process collisions in physics update
func process_collisions(delta):
	# Update collision cooldown
	if collision_cooldown_timer > 0:
		collision_cooldown_timer -= delta
	
	# Check for collisions if we've moved
	if owner_npc.get_slide_collision_count() > 0:
		_check_for_collisions()

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

# Check for obstacles in the path
func check_for_obstacles():
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
					var distance = owner_npc.global_position.distance_to(collision_point)
					
					if distance < obstacle_detection_distance:
						# Emit signal with obstacle information
						emit_signal("obstacle_detected", ray.target_position.normalized(), distance)
						return true
	
	# No obstacles detected
	return false

# Check for collisions
func _check_for_collisions():
	var current_collision = owner_npc.get_slide_collision(0)
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
	collision_direction = owner_npc.velocity.normalized()
	if collision_direction.length() == 0:
		# If we weren't moving, use the direction from us to the collider
		collision_direction = (owner_npc.global_position - collision.get_position()).normalized()
	
	# Emit signal with collision information
	emit_signal("collision_detected", collision, collision_direction)

# Calculate a randomized direction to move around an obstacle
func calculate_avoidance_direction():
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
				var distance = owner_npc.global_position.distance_to(collision_point)
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
		# If all else fails, pick a completely random direction
		avoidance_direction = Vector3(randf_range(-1, 1), 0, randf_range(-1, 1)).normalized()
	
	# Calculate a target position to move toward
	avoidance_target = owner_npc.global_position + avoidance_direction * obstacle_avoidance_radius
	
	return avoidance_direction

# React to state changes
func _on_state_changed(old_state, new_state):
	# Reset collision state on relevant state changes
	pass

# Get the current avoidance target
func get_avoidance_target():
	return avoidance_target

# Is a path blocked?
func is_path_blocked(target_position):
	# Cast ray towards target to check for obstacles
	var direction = (target_position - owner_npc.global_position).normalized()
	var distance = owner_npc.global_position.distance_to(target_position)
	
	var ray = RayCast3D.new()
	ray.enabled = true 
	ray.exclude_parent = true
	ray.target_position = direction * min(distance, obstacle_detection_distance * 2)
	ray.collision_mask = 1
	
	add_child(ray)
	ray.force_raycast_update()
	
	var is_blocked = ray.is_colliding()
	var result = false
	
	if is_blocked:
		var collider = ray.get_collider()
		# Check if this is an obstacle we should avoid
		var should_ignore = false
		for ignore_name in ignored_colliders:
			if collider and collider.name.contains(ignore_name):
				should_ignore = true
				break
		
		result = !should_ignore
	
	ray.queue_free()
	return result
