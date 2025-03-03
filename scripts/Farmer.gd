# Farmer.gd
# Farmer-specific behavior for farming simulation
extends NPCBase
class_name Farmer

# Important locations
@export_group("Locations")
@export var home_position: Vector3 = Vector3.ZERO
@export var bed_position: Vector3 = Vector3.ZERO
@export var kitchen_position: Vector3 = Vector3.ZERO
@export var field_position: Vector3 = Vector3.ZERO

# Food properties
@export_group("Food")
@export var food_recovery_rate: float = 33.33  # Recover full food in ~3 hours

# Decision thresholds
@export_group("Decision Thresholds")
@export var hungry_threshold: float = 50.0
@export var starving_threshold: float = 25.0
@export var full_threshold: float = 90.0  # Consider "full" at 90% food

# Debug options
@export_group("Debug Options")
@export var enable_hourly_eating_reports: bool = false  # Toggle for hourly eating reports

# Time tracking
var last_decision_time = 0
var decision_cooldown = 0.0
var eating_start_food = 0.0
var eating_start_hour = 0
var last_eating_hour = -1
var at_field: bool = false

func _init_npc():
	print("FARMER INIT: Starting initialization")
	
	# Get time manager reference
	time_manager = Clock
	
	# Create the state machine with FARMER type explicitly
	print("FARMER INIT: Creating state machine with type: ", NPCStates.NPCType.FARMER)
	state_machine = NPCStates.new(self, NPCStates.NPCType.FARMER)
	
	# Debug: Print all enum values to check for overlap
	state_machine.print_state_values()
	
	print("FARMER INIT: Created state machine with type: ", 
		  "FARMER" if state_machine.npc_type == NPCStates.NPCType.FARMER else "BANDIT")
	print("FARMER INIT: Initial state value: ", state_machine.current_state, 
		  " (", state_machine.get_state_name(state_machine.current_state), ")")
	
	# Connect to hour changed signal
	time_manager.hour_changed.connect(_on_hour_changed)
	
	# Determine starting state based on time
	print("FARMER INIT: About to determine starting state")
	_determine_starting_state()
	
	print("FARMER INIT: Completed state determination. Current state: ", 
		  state_machine.get_state_name(state_machine.current_state))
	print("Initial state: ", state_machine.get_state_name(state_machine.current_state))
	print("Initial food level: ", "%.2f" % food)
	
	# Debug information
	if debug_position:
		print("Starting at position: ", global_position)
		print("Field is at: ", field_position)
		print("Home is at: ", home_position)
		print("Kitchen is at: ", kitchen_position)
		print("Bed is at: ", bed_position)

# Process farmer behavior based on current state
func _process_current_state(delta):
	if state_machine == null:
		print("ERROR: No state machine in _process_current_state!")
		return
		
	var current_state = state_machine.current_state
	
	# Debug state periodically
	if debug_timer >= 5.0:
		debug_timer = 0.0
		print("Current farmer state: ", state_machine.get_state_name(current_state))
	
	# Process standard navigation states with base class
	if current_state == NPCStates.BaseState.NAVIGATING_OBSTACLE:
		_handle_obstacle_avoidance(delta)
	elif current_state == NPCStates.BaseState.WAITING_FOR_PATH:
		_handle_path_waiting(delta)
	elif current_state == NPCStates.BaseState.COLLISION_RECOVERY:
		_handle_collision_recovery(delta)
	# Process farmer-specific states
	elif current_state == NPCStates.FarmerState.WALKING_TO_FIELD or \
		 current_state == NPCStates.FarmerState.WALKING_HOME:
		_handle_navigation(delta)
	elif current_state == NPCStates.FarmerState.SLEEPING:
		# Food still depletes while sleeping, but much slower
		food -= delta * food_depletion_rate * 0.1
	elif current_state == NPCStates.FarmerState.WORKING:
		# Working depletes food faster
		food -= delta * food_depletion_rate * 1.5
	elif current_state == NPCStates.FarmerState.EATING:
		_handle_eating(delta)
	elif current_state == NPCStates.FarmerState.WAKING_UP:
		_handle_waking_up(delta)
	elif current_state == NPCStates.FarmerState.GOING_TO_BED:
		_handle_going_to_bed(delta)
	elif current_state == NPCStates.BaseState.IDLE:
		# Handle idle state - consider transitioning to a more meaningful state
		if decision_cooldown <= 0:
			_make_decisions()
	# Detect if we somehow got into an invalid state (like Bandit states)
	elif current_state in NPCStates.BanditState.values():
		print("ERROR: Farmer is in Bandit state: ", state_machine.get_state_name(current_state))
		# Emergency recovery - force to IDLE state
		state_machine.current_state = NPCStates.BaseState.IDLE
		print("Emergency state recovery to IDLE")
	else:
		print("WARNING: Unhandled state in _process_current_state: ", state_machine.get_state_name(current_state))
	
	# Decrease decision cooldown if it's active
	if decision_cooldown > 0:
		decision_cooldown -= delta
	
	# Make decisions every few seconds (except during transitions or sleeping)
	if current_state not in [NPCStates.FarmerState.WAKING_UP, 
							NPCStates.FarmerState.GOING_TO_BED, 
							NPCStates.FarmerState.WALKING_HOME, 
							NPCStates.FarmerState.WALKING_TO_FIELD, 
							NPCStates.FarmerState.SLEEPING,
							NPCStates.BaseState.NAVIGATING_OBSTACLE,
							NPCStates.BaseState.WAITING_FOR_PATH,
							NPCStates.BaseState.COLLISION_RECOVERY]:
		if time_manager.minutes - last_decision_time >= 2 and decision_cooldown <= 0:  # Check every 2 game minutes
			last_decision_time = time_manager.minutes
			_make_decisions()

# Handle eating behavior
func _handle_eating(delta):
	# Recover food while eating
	var old_food = food
	food = min(max_food, food + delta * food_recovery_rate)
	
	# Check if we're full and should go to work
	if food >= full_threshold * max_food / 100.0 and decision_cooldown <= 0:
		_handle_eating_complete()
	
	# Check if the hour has changed while eating for hourly reports
	var current_hour = time_manager.hours
	if current_hour != last_eating_hour and state_machine.current_state == NPCStates.FarmerState.EATING:
		last_eating_hour = current_hour
		
		# Only show hourly reports if enabled
		if enable_hourly_eating_reports:
			_show_eating_report(current_hour)

# Show eating progress report
func _show_eating_report(current_hour):
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

# Handle eating completion
func _handle_eating_complete():
	var hours_eating = time_manager.hours - eating_start_hour
	if hours_eating < 0:  # Handle day wrapping
		hours_eating += 24
	var food_gained = food - eating_start_food
	
	print("EATING COMPLETE:")
	print("  Farmer finished eating in approximately ", hours_eating, " hours")
	print("  Started with ", "%.2f" % eating_start_food, " food, now at ", "%.2f" % food)
	print("  Gained ", "%.2f" % food_gained, " food (", "%.2f" % (food_gained / max(1.0, hours_eating)), " per hour)")
	print("Farmer is full and ready to work")
	
	state_machine.change_state(NPCStates.FarmerState.WALKING_TO_FIELD)

# Handle waking up transition
func _handle_waking_up(delta):
	# Use state timer from state machine
	if state_machine.state_timer >= 1.0:
		print("Finished waking up, now deciding what to do")
		if food < hungry_threshold:
			print("Farmer is hungry, going to eat")
			state_machine.change_state(NPCStates.FarmerState.EATING)
		else:
			print("Farmer is not hungry, going to work")
			state_machine.change_state(NPCStates.FarmerState.WALKING_TO_FIELD)

# Handle going to bed transition
func _handle_going_to_bed(delta):
	# Use state timer from state machine
	if state_machine.state_timer >= 1.0:
		print("Getting into bed now")
		state_machine.change_state(NPCStates.FarmerState.SLEEPING)

# Handle navigation finished event
func _on_navigation_finished():
	var current_location = _get_location_name(global_position)
	print("Navigation finished at: ", current_location)
	
	# Take action based on where we arrived
	if state_machine.current_state == NPCStates.FarmerState.WALKING_TO_FIELD:
		# Check if we're close enough to the field position
		var distance_to_field = global_position.distance_to(field_position)
		if distance_to_field <= arrival_distance:
			at_field = true
			print("Farmer has reached the field!")
		else:
			at_field = false
			print("Farmer isn't quite at the field center. Distance: ", distance_to_field)
		
		state_machine.change_state(NPCStates.FarmerState.WORKING)
	elif state_machine.current_state == NPCStates.FarmerState.WALKING_HOME:
		at_field = false
		# When arriving home, decide whether to eat or sleep
		if time_manager.hours >= 22:
			state_machine.change_state(NPCStates.FarmerState.GOING_TO_BED)
		else:
			state_machine.change_state(NPCStates.FarmerState.EATING)

# Handle state changes
func _handle_state_change(old_state, new_state):
	# Set a cooldown to prevent rapid state changes
	decision_cooldown = 1.0
	
	# Handle state entry actions
	match new_state:
		NPCStates.FarmerState.WALKING_TO_FIELD:
			current_destination = "Field"
			print("Setting destination to field: ", field_position)
			navigate_to(field_position)
		NPCStates.FarmerState.WALKING_HOME:
			current_destination = "Home"
			print("Setting destination to home: ", home_position)
			navigate_to(home_position)
		NPCStates.FarmerState.EATING:
			eating_start_food = food  # Track starting food level
			eating_start_hour = time_manager.hours  # Track hour when eating started
			last_eating_hour = time_manager.hours
			print("Starting to eat at hour ", last_eating_hour)
			print("  Initial food level: ", "%.2f" % food, "/", "%.2f" % max_food, " (", "%.2f" % (food/max_food*100), "%)")

# Make decisions based on current needs
func _make_decisions():
	# Print current stats only for active states (not sleeping)
	if state_machine.current_state != NPCStates.FarmerState.SLEEPING:
		print("Making decisions. Current state: ", state_machine.get_state_name(state_machine.current_state), 
			  " Food: ", "%.2f" % food, "/", "%.2f" % max_food)
	
	# Don't make decisions on cooldown
	if state_machine.is_on_cooldown():
		return
	
	# Handle extreme need
	if food <= starving_threshold and state_machine.current_state != NPCStates.FarmerState.EATING:
		# Too hungry, need to eat
		print("Farmer is too hungry to continue working")
		if state_machine.current_state != NPCStates.FarmerState.WALKING_HOME and state_machine.current_state != NPCStates.FarmerState.EATING:
			state_machine.change_state(NPCStates.FarmerState.WALKING_HOME)
		return
	
	# Normal decision making
	match state_machine.current_state:
		NPCStates.FarmerState.EATING:
			# If done eating (full), go to work
			if food >= full_threshold:
				_handle_eating_complete()
		
		NPCStates.FarmerState.WORKING:
			# If getting hungry, go home to eat
			if food < hungry_threshold:
				print("Farmer is getting hungry and heading home")
				state_machine.change_state(NPCStates.FarmerState.WALKING_HOME)
				
		NPCStates.BaseState.IDLE:
			# If in idle state, transition to a more appropriate state
			if time_manager.hours >= 22 or time_manager.hours < 7:
				# It's nighttime
				state_machine.change_state(NPCStates.FarmerState.GOING_TO_BED)
			elif food < hungry_threshold:
				# Need to eat
				state_machine.change_state(NPCStates.FarmerState.WALKING_HOME)
			else:
				# Otherwise, go to work
				state_machine.change_state(NPCStates.FarmerState.WALKING_TO_FIELD)

# Called when the game time changes hours
func _on_hour_changed(hour):
	print("Hour changed to: ", hour)
	
	# Only handle wake up and sleep times
	match hour:
		7:  # 7 AM - Wake up
			if state_machine.current_state == NPCStates.FarmerState.SLEEPING:
				state_machine.change_state(NPCStates.FarmerState.WAKING_UP)
		22: # 10 PM - Bed time
			if state_machine.current_state != NPCStates.FarmerState.SLEEPING and state_machine.current_state != NPCStates.FarmerState.GOING_TO_BED:
				print("It's getting late, farmer is heading to bed")
				if state_machine.current_state != NPCStates.FarmerState.WALKING_HOME:
					state_machine.change_state(NPCStates.FarmerState.WALKING_HOME)
				# Already heading home, will go to bed when arrives

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

# Set initial state based on game time
func _determine_starting_state():
	var hour = time_manager.hours
	
	print("FARMER: Determining starting state based on hour: ", hour)
	
	# Place character in appropriate location & state for the time
	if hour >= 0 and hour < 7:
		global_position = bed_position
		print("FARMER: It's nighttime (", hour, "), should be sleeping")
		state_machine.change_state(NPCStates.FarmerState.SLEEPING)
	elif hour >= 7 and hour < 22:
		# During the day, decide based on needs
		if food < hungry_threshold:
			global_position = kitchen_position
			print("FARMER: Daytime and hungry, should be eating")
			state_machine.change_state(NPCStates.FarmerState.EATING)
			eating_start_food = food
			eating_start_hour = hour
			last_eating_hour = hour
		else:
			global_position = field_position
			print("FARMER: Daytime and not hungry, should be working")
			state_machine.change_state(NPCStates.FarmerState.WORKING)
	else: # 22 or 23
		global_position = home_position
		print("FARMER: Evening, should be going to bed")
		state_machine.change_state(NPCStates.FarmerState.GOING_TO_BED)
		
	print("FARMER: After determining state: ", 
		  state_machine.get_state_name(state_machine.current_state))
			
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

# Return specific NPC type for debugging
func get_npc_type():
	return "Farmer"
