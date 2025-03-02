extends CharacterBody3D

class_name Farmer

# States for the farmer's activities
enum State {
	SLEEPING,
	WAKING_UP,
	EATING,
	WALKING_TO_FIELD,
	WORKING,
	WALKING_HOME,
	GOING_TO_BED,
	NAVIGATING_AROUND_OBSTACLE,
	WAITING_FOR_PATH,
	COLLISION_RECOVERY
}

# Current state
var current_state = State.SLEEPING

# Farmer needs - only food now
@export_group("Needs")
@export var max_food: float = 100.0
@export var food: float = 0.0  # Start with zero food to clearly see the progression
@export var food_depletion_rate: float = 8.0
@export var food_recovery_rate: float = 33.33  # Recover full food in ~3 hours

# Decision thresholds
@export_group("Decision Thresholds")
@export var hungry_threshold: float = 50.0
@export var starving_threshold: float = 25.0
@export var full_threshold: float = 90.0  # Consider "full" at 90% food

# Debug options
@export_group("Debug Options")
@export var enable_hourly_eating_reports: bool = false  # Toggle for hourly eating reports
@export var debug_navigation: bool = true  # Added to help debug navigation issues
@export var debug_position: bool = true    # Print position updates
@export var draw_navigation_path: bool = true  # Draw the current navigation path for debugging

# Navigation options
@export_group("Navigation")
@export var movement_speed: float = 10.0  # 5x the original speed of 2.0
@export var obstacle_avoidance_speed: float = 5.0  # Slower speed when navigating obstacles
@export var repath_interval: float = 1.0  # How often to check if a new path is needed
@export var stuck_threshold: float = 0.2  # Minimum movement needed to not be considered stuck
@export var arrival_distance: float = 1.0  # How close to consider "arrived"
@export var blocked_path_wait_time: float = 3.0  # How long to wait before checking again
@export var obstacle_detection_distance: float = 2.0  # Distance to check for obstacles
@export var obstacle_avoidance_radius: float = 3.0  # How far to move away from obstacles
@export var collision_recovery_time: float = 1.0  # How long to spend in recovery mode
@export var collision_cooldown: float = 2.0  # Time to ignore collisions after handling one

# Ignore collisions with these object names
@export var ignored_colliders: Array[String] = ["Ground", "Terrain", "Floor", "Land"]

# Important locations - now exposed to the Inspector
@export_group("Locations")
@export var home_position: Vector3 = Vector3.ZERO
@export var bed_position: Vector3 = Vector3.ZERO  
@export var kitchen_position: Vector3 = Vector3.ZERO
@export var field_position: Vector3 = Vector3.ZERO

# Navigation
@onready var nav_agent = $NavigationAgent3D

# Obstacle Detection
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

# Reference to the time manager
var time_manager
var last_decision_time = 0
var state_change_timer = 0
var eating_start_food = 0.0  # Track initial food level when starting to eat
var eating_start_hour = 0     # Track hour when eating started
var last_eating_hour = -1     # Track the last hour we reported food for
var decision_cooldown = 0.0   # Prevent too-frequent decision changes
var current_destination = ""  # Track where the farmer is heading
var destination_position = Vector3.ZERO  # Store the actual destination position

# Navigation tracking
var last_position = Vector3.ZERO
var time_since_repath = 0.0
var same_position_time = 0.0
var debug_timer = 0.0
var path_blocked_timer = 0.0
var obstacle_check_timer = 0.0
var collision_recovery_timer = 0.0
var collision_cooldown_timer = 0.0  # Timer to prevent collision handling loops
var last_collider = null  # Remember what we collided with
var was_on_floor = false  # Track if we were on the floor last frame
var collision_direction = Vector3.ZERO  # Direction we were moving when collision occurred
var collision_recovery_target = Vector3.ZERO  # Position to back up to after collision
var obstacle_position = Vector3.ZERO  # Position of detected obstacle
var avoidance_direction = Vector3.ZERO
var avoidance_target = Vector3.ZERO
var original_path = []
var navigation_path_line
var has_moved = false  # Flag to check if farmer has moved intentionally
var startup_delay = 1.0  # Short delay before enabling collision detection after startup
var recovery_attempts = 0  # Track consecutive collision recovery attempts
var max_recovery_attempts = 3  # Maximum attempts before trying a different approach
var at_field: bool = false  # Track if the farmer is at the field

func _ready():
	# Get time manager reference
	time_manager = Clock
	
	# Connect to hour changed signal
	time_manager.hour_changed.connect(_on_hour_changed)
	
	# Set up navigation agent
	nav_agent.path_desired_distance = arrival_distance
	nav_agent.target_desired_distance = arrival_distance
	nav_agent.avoidance_enabled = true
	
	# Set up raycasts for obstacle detection in multiple directions
	_setup_obstacle_detection()
	
	# Connect navigation signal
	nav_agent.velocity_computed.connect(_on_velocity_computed)
	nav_agent.navigation_finished.connect(_on_navigation_finished)
	nav_agent.path_changed.connect(_on_path_changed)
	
	# Initialize position tracking
	last_position = global_position
	was_on_floor = is_on_floor()
	
	# Create a line to visualize the navigation path
	if draw_navigation_path:
		navigation_path_line = _create_path_visualization()
	
	# Set initial state based on game time
	_determine_starting_state()
	print("Initial state: ", _get_state_name(current_state))
	print("Initial food level: ", "%.2f" % food)
	
	# Debug information
	if debug_position:
		print("Starting at position: ", global_position)
		print("Field is at: ", field_position)
		print("Home is at: ", home_position)
		print("Kitchen is at: ", kitchen_position)
		print("Bed is at: ", bed_position)

# Set up multiple raycasts for better obstacle detection
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

func _physics_process(delta):
	# Process startup delay to prevent false collision detection at startup
	if startup_delay > 0:
		startup_delay -= delta
	
	# Debug timer for periodic position reporting
	debug_timer += delta
	if debug_position and debug_timer >= 2.0:
		debug_timer = 0.0
		print("Current position: ", global_position)
	
	# Update collision cooldown timer
	if collision_cooldown_timer > 0:
		collision_cooldown_timer -= delta
	
	# Update needs based on current state
	_update_needs(delta)
	
	# Decrease decision cooldown if it's active
	if decision_cooldown > 0:
		decision_cooldown -= delta
	
	# Update behavior based on current state
	match current_state:
		State.WALKING_TO_FIELD, State.WALKING_HOME:
			_handle_navigation(delta)
			has_moved = true
		State.NAVIGATING_AROUND_OBSTACLE:
			_handle_obstacle_avoidance(delta)
			has_moved = true
		State.WAITING_FOR_PATH:
			_handle_path_waiting(delta)
		State.COLLISION_RECOVERY:
			_handle_collision_recovery(delta)
			has_moved = true
		State.WORKING:
			# Working depletes food faster
			food -= delta * food_depletion_rate * 1.5
		State.EATING:
			# Recover food while eating
			var old_food = food
			food = min(max_food, food + delta * food_recovery_rate)
			
			# Check if we're full and should go to work
			if food >= full_threshold * max_food / 100.0 and decision_cooldown <= 0:
				var hours_eating = time_manager.hours - eating_start_hour
				if hours_eating < 0:  # Handle day wrapping
					hours_eating += 24
				var food_gained = food - eating_start_food
				print("EATING COMPLETE:")
				print("  Farmer finished eating in approximately ", hours_eating, " hours")
				print("  Started with ", "%.2f" % eating_start_food, " food, now at ", "%.2f" % food)
				print("  Gained ", "%.2f" % food_gained, " food (", "%.2f" % (food_gained / max(1.0, hours_eating)), " per hour)")
				print("Farmer is full and ready to work")
				change_state(State.WALKING_TO_FIELD)
			
			# Check if the hour has changed while eating for hourly reports
			var current_hour = time_manager.hours
			if current_hour != last_eating_hour and current_state == State.EATING:
				last_eating_hour = current_hour
				
				# Only show hourly reports if enabled
				if enable_hourly_eating_reports:
					var hours_eating = current_hour - eating_start_hour
					if hours_eating < 0:  # Handle day wrapping
						hours_eating += 24
					var food_gained = food - eating_start_food
					var food_percent = (food / max_food) * 100
					
					print("HOURLY EATING REPORT - Hour ", current_hour)
					print("  Food level: ", "%.2f" % food, "/", "%.2f" % max_food, " (", "%.2f" % food_percent, "%)")
					print("  Hours spent eating: ", hours_eating)
					print("  Food gained since starting: ", "%.2f" % food_gained)
					print("  Food gained per hour: ", "%.2f" % (food_gained / max(1.0, hours_eating)))
					print("  Estimated time until full: ", "%.2f" % ((max_food - food) / food_recovery_rate), " hours")
		
		State.SLEEPING:
			# Food still depletes while sleeping, but much slower
			food -= delta * food_depletion_rate * 0.1
	
	# Handle transition states with timers
	if current_state == State.WAKING_UP:
		state_change_timer += delta
		if state_change_timer >= 1.0:
			state_change_timer = 0
			print("Finished waking up, now deciding what to do")
			if food < hungry_threshold:
				print("Farmer is hungry, going to eat")
				change_state(State.EATING)
			else:
				print("Farmer is not hungry, going to work")
				change_state(State.WALKING_TO_FIELD)
	
	elif current_state == State.GOING_TO_BED:
		state_change_timer += delta
		if state_change_timer >= 1.0:
			state_change_timer = 0
			print("Getting into bed now")
			change_state(State.SLEEPING)
	
	# Make decisions every few seconds (but not during transitions or sleeping)
	if current_state not in [State.WAKING_UP, State.GOING_TO_BED, State.WALKING_HOME, State.WALKING_TO_FIELD, State.SLEEPING, State.NAVIGATING_AROUND_OBSTACLE, State.WAITING_FOR_PATH, State.COLLISION_RECOVERY]:
		if time_manager.minutes - last_decision_time >= 2 and decision_cooldown <= 0:  # Check every 2 game minutes
			last_decision_time = time_manager.minutes
			_make_decisions()
	
	# Check for obstacles periodically
	obstacle_check_timer += delta
	if obstacle_check_timer >= 0.5 and (current_state == State.WALKING_TO_FIELD or current_state == State.WALKING_HOME):
		obstacle_check_timer = 0
		_check_for_obstacles()
	
	# Reset collision detection for this frame
	if startup_delay <= 0 and has_moved and collision_cooldown_timer <= 0:
		_check_for_collisions()

# Check for collisions, filtering out the ground
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

# Handle collision recovery state
func _handle_collision_recovery(delta):
	collision_recovery_timer += delta
	
	if collision_recovery_timer >= collision_recovery_time:
		# We've backed up enough, try to navigate around the obstacle
		collision_recovery_timer = 0
		recovery_attempts = 0  # Reset attempts counter
		print("Collision recovery complete, finding new path")
		change_state(State.NAVIGATING_AROUND_OBSTACLE)
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
	
	# If we hit something else while backing up, try a different direction,
	# but only if it's not the same object and not in cooldown
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

# Handle a collision event 
func _handle_collision_event(collision):
	# Save the direction we were moving in when the collision occurred
	collision_direction = velocity.normalized()
	if collision_direction.length() == 0:
		# If we weren't moving, use the direction from us to the collider
		collision_direction = (global_position - collision.get_position()).normalized()
	
	print("Collision occurred while moving in direction: ", collision_direction)
	
	# Check if we're already in recovery and increment attempts counter
	if current_state == State.COLLISION_RECOVERY:
		recovery_attempts += 1
		
		# If we've tried too many times, take more drastic action
		if recovery_attempts >= max_recovery_attempts:
			print("Multiple collision recovery attempts failed. Taking evasive action!")
			_handle_persistent_collision()
			return
	else:
		recovery_attempts = 0
	
	# Enter collision recovery mode
	change_state(State.COLLISION_RECOVERY)

# Handle persistent collisions that can't be resolved with normal recovery
func _handle_persistent_collision():
	# Teleport slightly away from current position in a random direction
	var random_direction = Vector3(randf_range(-1, 1), 0, randf_range(-1, 1)).normalized()
	
	# Move a decent distance away
	var teleport_distance = 3.0
	var new_position = global_position + random_direction * teleport_distance
	
	print("Emergency teleport to escape persistent collision. New position: ", new_position)
	global_position = new_position
	
	# Reset recovery attempts and proceed to navigate around
	recovery_attempts = 0
	collision_cooldown_timer = collision_cooldown * 2  # Double cooldown for teleport
	
	if current_destination == "Field":
		change_state(State.WALKING_TO_FIELD)
	elif current_destination == "Home":
		change_state(State.WALKING_HOME)
	else:
		change_state(State.NAVIGATING_AROUND_OBSTACLE)

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

# Handle waiting for a path to clear
func _handle_path_waiting(delta):
	path_blocked_timer += delta
	if path_blocked_timer >= blocked_path_wait_time:
		path_blocked_timer = 0
		
		# Check if path is now clear
		print("Checking if path is clear now...")
		if nav_agent.is_target_reachable():
			print("Path is now clear! Resuming navigation.")
			
			# Return to normal navigation state
			if current_destination == "Field":
				change_state(State.WALKING_TO_FIELD)
			elif current_destination == "Home":
				change_state(State.WALKING_HOME)
		else:
			print("Path is still blocked. Trying to find an alternate route...")
			change_state(State.NAVIGATING_AROUND_OBSTACLE)

# Handle obstacle avoidance maneuvers
func _handle_obstacle_avoidance(delta):
	# If we don't have an avoidance direction yet, pick one
	if avoidance_direction == Vector3.ZERO:
		_calculate_randomized_avoidance_direction()
	
	# Check if we've reached our temporary avoidance target
	if global_position.distance_to(avoidance_target) < 1.0:
		print("Reached temporary avoidance target. Retrying main navigation.")
		
		# Try normal navigation again
		if current_destination == "Field":
			change_state(State.WALKING_TO_FIELD)
		elif current_destination == "Home":
			change_state(State.WALKING_HOME)
		return
	
	# Move toward our avoidance target
	var direction = (avoidance_target - global_position).normalized()
	direction.y = 0  # Stay on ground plane
	
	# Apply movement at a slower speed for more control
	velocity = direction * obstacle_avoidance_speed
	move_and_slide()
	
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
	
	# Periodically check if we can see the destination now
	obstacle_check_timer += delta
	if obstacle_check_timer >= 1.0:
		obstacle_check_timer = 0
		
		# See if we can now find a path to the destination
		if current_destination == "Field":
			nav_agent.target_position = field_position
		elif current_destination == "Home":
			nav_agent.target_position = home_position
		
		if nav_agent.is_target_reachable():
			print("Path is now clear! Returning to normal navigation.")
			if current_destination == "Field":
				change_state(State.WALKING_TO_FIELD)
			elif current_destination == "Home":
				change_state(State.WALKING_HOME)

# Standard avoidance direction calculation 
func _calculate_avoidance_direction():
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
		# Move in the clearest direction
		avoidance_direction = clearest_direction
	else:
		# All directions have obstacles, try to move perpendicular to the closest one
		var closest_obstacle = null
		var closest_distance = 9999
		
		for blocked in blocked_directions:
			if blocked.distance < closest_distance:
				closest_distance = blocked.distance
				closest_obstacle = blocked.direction
		
		if closest_obstacle != null:
			# Move perpendicular to the closest obstacle
			avoidance_direction = Vector3(closest_obstacle.z, 0, -closest_obstacle.x)
		else:
			# If all else fails, pick a completely random direction
			avoidance_direction = Vector3(randf_range(-1, 1), 0, randf_range(-1, 1)).normalized()
	
	# Calculate a target position to move toward
	avoidance_target = global_position + avoidance_direction * obstacle_avoidance_radius
	
	print("Calculated avoidance direction: ", avoidance_direction)
	print("Setting temporary avoidance target: ", avoidance_target)

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
						obstacle_position = collision_point
						
						# Only perform avoidance if we're actually moving
						if velocity.length() > 0.5:
							change_state(State.NAVIGATING_AROUND_OBSTACLE)
							return true
	
	# No obstacles detected
	return false

# Called when navigation finishes
func _on_navigation_finished():
	var current_location = _get_location_name(global_position)
	print("Navigation finished at: ", current_location)
	
	# Take action based on where we arrived
	if current_state == State.WALKING_TO_FIELD:
		# Check if we're close enough to the field position
		var distance_to_field = global_position.distance_to(field_position)
		if distance_to_field <= arrival_distance:
			at_field = true
			print("Farmer has reached the field!")
		else:
			at_field = false
			print("Farmer isn't quite at the field center. Distance: ", distance_to_field)
		
		change_state(State.WORKING)
	elif current_state == State.WALKING_HOME:
		at_field = false
		# When arriving home, decide whether to eat or sleep
		if time_manager.hours >= 22:
			change_state(State.GOING_TO_BED)
		else:
			change_state(State.EATING)

# Called when the navigation path changes
func _on_path_changed():
	if debug_navigation:
		print("Path changed. Path size: ", nav_agent.get_current_navigation_path().size())
	
	# Store the original path for reference
	original_path = nav_agent.get_current_navigation_path()
	
	# Update the path visualization
	_update_path_visualization()

# Called when navigation computes a new velocity
func _on_velocity_computed(safe_velocity):
	if debug_navigation and safe_velocity.length() > 0:
		print("Computed velocity: ", safe_velocity)
	
	# Apply the computed velocity
	velocity = safe_velocity
	move_and_slide()
	
	# Face movement direction if we're moving
	if velocity.length() > 0.1:
		var look_dir = Vector3(velocity.x, 0, velocity.z).normalized()
		if look_dir.length() > 0:
			look_at(global_position + look_dir, Vector3.UP)

# Update needs based on the current state
func _update_needs(delta):
	# Food naturally depletes over time (basic metabolism)
	food -= delta * food_depletion_rate * 0.5
	
	# Clamp values to valid ranges
	food = clamp(food, 0, max_food)

# Make decisions based on current needs
func _make_decisions():
	# Print current stats only for active states (not sleeping)
	if current_state != State.SLEEPING:
		print("Making decisions. Current state: ", _get_state_name(current_state), 
			  " Food: ", "%.2f" % food, "/", "%.2f" % max_food)
	
	# Don't make decisions while in transition states or on cooldown
	var transition_states = [
		State.WALKING_TO_FIELD, 
		State.WALKING_HOME, 
		State.WAKING_UP, 
		State.GOING_TO_BED, 
		State.SLEEPING, 
		State.NAVIGATING_AROUND_OBSTACLE, 
		State.WAITING_FOR_PATH,
		State.COLLISION_RECOVERY
	]
	if current_state in transition_states or decision_cooldown > 0:
		return
	
	# Handle extreme need
	if food <= starving_threshold and current_state != State.EATING:
		# Too hungry, need to eat
		print("Farmer is too hungry to continue working")
		if current_state != State.WALKING_HOME and current_state != State.EATING:
			change_state(State.WALKING_HOME)
		return
	
	# Normal decision making
	match current_state:
		State.EATING:
			# If done eating (full), go to work
			if food >= full_threshold:
				var hours_eating = time_manager.hours - eating_start_hour
				if hours_eating < 0:  # Handle day wrapping
					hours_eating += 24
				var food_gained = food - eating_start_food
				print("EATING COMPLETE:")
				print("  Farmer finished eating in approximately ", hours_eating, " hours")
				print("  Started with ", "%.2f" % eating_start_food, " food, now at ", "%.2f" % food)
				print("  Gained ", "%.2f" % food_gained, " food (", "%.2f" % (food_gained / max(1.0, hours_eating)), " per hour)")
				print("Farmer is full and ready to work")
				change_state(State.WALKING_TO_FIELD)
		
		State.WORKING:
			# If getting hungry, go home to eat
			if food < hungry_threshold:
				print("Farmer is getting hungry and heading home")
				change_state(State.WALKING_HOME)

# Called when the game time changes hours
func _on_hour_changed(hour):
	print("Hour changed to: ", hour)
	
	# Only handle wake up and sleep times
	match hour:
		7:  # 7 AM - Wake up
			if current_state == State.SLEEPING:
				change_state(State.WAKING_UP)
		22: # 10 PM - Bed time
			if current_state != State.SLEEPING and current_state != State.GOING_TO_BED:
				print("It's getting late, farmer is heading to bed")
				if current_state != State.WALKING_HOME:
					change_state(State.WALKING_HOME)
				else:
					# Already heading home, will go to bed when arrives
					pass

# Get state name for debugging
func _get_state_name(state_value):
	var state_names = {
		State.SLEEPING: "SLEEPING",
		State.WAKING_UP: "WAKING_UP",
		State.EATING: "EATING",
		State.WALKING_TO_FIELD: "WALKING_TO_FIELD",
		State.WORKING: "WORKING",
		State.WALKING_HOME: "WALKING_HOME",
		State.GOING_TO_BED: "GOING_TO_BED",
		State.NAVIGATING_AROUND_OBSTACLE: "NAVIGATING_AROUND_OBSTACLE",
		State.WAITING_FOR_PATH: "WAITING_FOR_PATH",
		State.COLLISION_RECOVERY: "COLLISION_RECOVERY"
	}
	return state_names[state_value]

# Get location name based on position
func _get_location_name(position):
	# Check which location this position is closest to
	var locations = {
		"Home": home_position,
		"Bed": bed_position,
		"Kitchen": kitchen_position,
		"Field": field_position
	}
	
	var closest_location = "Unknown"
	var closest_distance = 999999.0
	
	for location_name in locations:
		var location_pos = locations[location_name]
		var distance = position.distance_to(location_pos)
		
		if distance < closest_distance:
			closest_distance = distance
			closest_location = location_name
	
	# If we're very close to a known location (within 2 units)
	if closest_distance < 2.0:
		return closest_location
	else:
		return "somewhere between locations"

# Handle state changes
func change_state(new_state):
	print("Farmer changing from ", _get_state_name(current_state), " to ", _get_state_name(new_state))
	
	# Prevent changing from collision recovery to collision recovery
	if current_state == State.COLLISION_RECOVERY and new_state == State.COLLISION_RECOVERY:
		print("Already in collision recovery, ignoring redundant state change")
		return
	
	var previous_state = current_state
	current_state = new_state
	
	# Reset state timer when changing states
	state_change_timer = 0
	
	# Set a cooldown to prevent rapid state changes
	decision_cooldown = 1.0
	
	# Reset navigation tracking when changing to a walking state
	if new_state == State.WALKING_TO_FIELD or new_state == State.WALKING_HOME:
		time_since_repath = 0.0
		same_position_time = 0.0
		avoidance_direction = Vector3.ZERO
		avoidance_target = Vector3.ZERO
		recovery_attempts = 0
	
	# Reset obstacle avoidance state
	if new_state == State.NAVIGATING_AROUND_OBSTACLE:
		avoidance_direction = Vector3.ZERO
		obstacle_check_timer = 0.0
		recovery_attempts = 0
	
	# Reset waiting timer
	if new_state == State.WAITING_FOR_PATH:
		path_blocked_timer = 0.0
	
	# Reset collision recovery timer
	if new_state == State.COLLISION_RECOVERY:
		collision_recovery_timer = 0.0
	
	# Handle state entry actions
	match new_state:
		State.WALKING_TO_FIELD:
			current_destination = "Field"
			destination_position = field_position
			print("Setting destination to field: ", field_position)
			nav_agent.target_position = field_position
		State.WALKING_HOME:
			current_destination = "Home"
			destination_position = home_position
			print("Setting destination to home: ", home_position)
			nav_agent.target_position = home_position
		State.EATING:
			eating_start_food = food  # Track starting food level
			eating_start_hour = time_manager.hours  # Track hour when eating started
			last_eating_hour = time_manager.hours
			print("Starting to eat at hour ", last_eating_hour)
			print("  Initial food level: ", "%.2f" % food, "/", "%.2f" % max_food, " (", "%.2f" % (food/max_food*100), "%)")

# Handle normal navigation to destinations
func _handle_navigation(delta):
	# Check if we've reached the destination
	if nav_agent.is_navigation_finished():
		_on_navigation_finished()
		return
	
	# Periodically update the path visualization
	if draw_navigation_path and time_since_repath >= repath_interval:
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
			print("Farmer appears to be stuck, checking for obstacles...")
			print("Current position: ", global_position, " Target position: ", nav_agent.target_position)
			same_position_time = 0.0
			
			# Check if we can't find a path to the destination anymore
			if not nav_agent.is_target_reachable():
				print("Target is no longer reachable. Waiting for path to clear...")
				change_state(State.WAITING_FOR_PATH)
				return
			
			# Check for obstacles that might be blocking us
			if _check_for_obstacles():
				return
			
			# Force a path recalculation
			if current_state == State.WALKING_TO_FIELD:
				nav_agent.target_position = field_position
			elif current_state == State.WALKING_HOME:
				nav_agent.target_position = home_position
	
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
	else:
		# If we have no velocity, just move directly toward the target (fallback)
		var direct_dir = (nav_agent.target_position - global_position).normalized()
		direct_dir.y = 0
		if direct_dir.length() > 0:
			nav_agent.velocity = direct_dir * movement_speed
			
			if debug_navigation:
				print("Using direct movement as fallback")

# Set initial state based on game time
func _determine_starting_state():
	var hour = time_manager.hours
	
	# Place character in appropriate location & state for the time
	if hour >= 0 and hour < 7:
		global_position = bed_position
		current_state = State.SLEEPING
	elif hour >= 7 and hour < 22:
		# During the day, decide based on needs
		if food < hungry_threshold:
			global_position = kitchen_position
			current_state = State.EATING
			eating_start_food = food
			eating_start_hour = hour
			last_eating_hour = hour
		else:
			global_position = field_position
			current_state = State.WORKING
	else: # 22 or 23
		if current_state != State.SLEEPING:
			global_position = home_position
			current_state = State.GOING_TO_BED
			
# Return the current time as a formatted string (for debugging or UI)
func get_formatted_time() -> String:
	var hour_display = time_manager.hours
	var period = "AM"
	
	# Convert to 12-hour format
	if hour_display >= 12:
		period = "PM"
		if hour_display > 12:
			hour_display -= 12
	elif hour_display == 0:
		hour_display = 12
	
	return "%d:%02d %s" % [hour_display, int(time_manager.minutes), period]
