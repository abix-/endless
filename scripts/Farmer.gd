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
var eating_start_minute: int = 0  # Track precise start time of eating in minutes
var last_eating_report_hour: int = -1  # Track the last hour an eating report was generated

# Decision thresholds
@export_group("Decision Thresholds")
@export var hungry_threshold: float = 50.0
@export var starving_threshold: float = 25.0
@export var full_threshold: float = 90.0  # Consider "full" at 90% food

# Debug options
@export_group("Debug Options")
@export var enable_hourly_eating_reports: bool = true  # Toggle for hourly eating reports

# Time tracking
var last_decision_time = 0
var decision_cooldown = 0.0
var eating_start_food = 0.0
var eating_start_hour = 0
var last_eating_hour = -1
var at_field: bool = false

# Dictionary of state handlers for more efficient state processing
var state_handlers = {}

func _init_npc():
	if debug_mode:
		print("FARMER INIT: Starting initialization")
	
	# Get time manager reference
	time_manager = Clock
	
	# Create the state machine with FARMER type explicitly
	if debug_mode:
		print("FARMER INIT: Creating state machine with type: ", NPCStates.NPCType.FARMER)
	state_machine = NPCStates.new(self, NPCStates.NPCType.FARMER)
	state_machine.verbose_debugging = debug_mode  # Set debugging based on NPC setting
	
	# Setup state handler dictionary for cleaner code
	_setup_state_handlers()
	
	# Debug: Print all enum values to check for overlap
	if debug_mode:
		state_machine.print_state_values()
		print("FARMER INIT: Created state machine with type: ", 
			"FARMER" if state_machine.npc_type == NPCStates.NPCType.FARMER else "BANDIT")
		print("FARMER INIT: Initial state value: ", state_machine.current_state, 
			" (", state_machine.get_state_name(state_machine.current_state), ")")
	
	# Connect to hour changed signal
	time_manager.hour_changed.connect(_on_hour_changed)
	
	# Determine starting state based on time
	if debug_mode:
		print("FARMER INIT: About to determine starting state")
	_determine_starting_state()
	
	if debug_mode:
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

# Setup dictionary of state handlers for cleaner code organization
func _setup_state_handlers():
	# Base states
	state_handlers[NPCStates.BaseState.NAVIGATING_OBSTACLE] = _handle_obstacle_avoidance
	state_handlers[NPCStates.BaseState.WAITING_FOR_PATH] = _handle_path_waiting
	state_handlers[NPCStates.BaseState.COLLISION_RECOVERY] = _handle_collision_recovery
	state_handlers[NPCStates.BaseState.IDLE] = _handle_idle
	
	# Farmer-specific states
	state_handlers[NPCStates.FarmerState.WALKING_TO_FIELD] = _handle_navigation
	state_handlers[NPCStates.FarmerState.WALKING_HOME] = _handle_navigation
	state_handlers[NPCStates.FarmerState.SLEEPING] = _handle_sleeping
	state_handlers[NPCStates.FarmerState.WORKING] = _handle_working
	state_handlers[NPCStates.FarmerState.EATING] = _handle_eating
	state_handlers[NPCStates.FarmerState.WAKING_UP] = _handle_waking_up
	state_handlers[NPCStates.FarmerState.GOING_TO_BED] = _handle_going_to_bed

# Process farmer behavior based on current state using state handlers dictionary
func _process_current_state(delta):
	if state_machine == null:
		print("ERROR: No state machine in _process_current_state!")
		return
		
	var current_state = state_machine.current_state
	
	# Debug state periodically
	if debug_timer >= 5.0 and debug_mode:
		debug_timer = 0.0
		print("Current farmer state: ", state_machine.get_state_name(current_state), 
			  " | Food: ", "%.2f" % food, "/", "%.2f" % max_food, 
			  " (", "%.1f" % ((food / max_food) * 100), "%)")
	
	# Use the state handler dictionary for cleaner code organization
	if state_handlers.has(current_state):
		state_handlers[current_state].call(delta)
	# Detect if we somehow got into an invalid state (like Bandit states)
	elif current_state in NPCStates.BanditState.values():
		print("ERROR: Farmer is in Bandit state: ", state_machine.get_state_name(current_state))
		# Emergency recovery - intelligently choose state based on time and needs
		if time_manager.hours >= 22 or time_manager.hours < 7:
			state_machine.change_state(NPCStates.FarmerState.SLEEPING)
		elif food < hungry_threshold:
			state_machine.change_state(NPCStates.FarmerState.EATING)
		else:
			state_machine.change_state(NPCStates.FarmerState.WORKING)
		print("Emergency state recovery completed")
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

# Handlers for each state
func _handle_sleeping(delta):
	# Food still depletes while sleeping, but much slower
	food -= delta * food_depletion_rate * 0.1

func _handle_working(delta):
	# Working depletes food faster
	food -= delta * food_depletion_rate * 1.5

func _handle_idle(delta):
	# Handle idle state - consider transitioning to a more meaningful state
	if decision_cooldown <= 0:
		_make_decisions()

# Handle eating behavior with improved tracking and reporting
func _handle_eating(delta):
	# Debug print to verify method is being called
	if debug_mode:
		print("DEBUG: _handle_eating called. Current state: %s" % state_machine.get_state_name(state_machine.current_state))
	
	# Recover food while eating
	var old_food = food
	food = min(max_food, food + delta * food_recovery_rate)
	
	# Check if we're full and should go to work
	if food >= full_threshold * max_food / 100.0 and decision_cooldown <= 0:
		_handle_eating_complete()
	
	# Get current time details
	var current_hour = time_manager.hours
	var current_minute = current_hour * 60 + time_manager.minutes
	
	# Ensure we're tracking eating start correctly
	if eating_start_minute == 0:
		eating_start_minute = current_minute
		eating_start_food = food
		last_eating_hour = current_hour
	
	# Calculate minutes spent eating
	var minutes_eating = current_minute - eating_start_minute
	
	# Handle day wrapping
	if minutes_eating < 0:
		minutes_eating += 24 * 60
	
	# Only process report if we've been eating for at least a minute
	# and we're still in the eating state
	if (minutes_eating > 0 and 
		state_machine.current_state == NPCStates.FarmerState.EATING and
		current_hour != last_eating_report_hour):
		
		# Update last report hour to prevent duplicate reports
		last_eating_report_hour = current_hour
		
		# Calculate detailed eating metrics
		var food_gained = food - eating_start_food
		var food_percent = (food / max_food) * 100
		var food_gain_rate = food_gained / max(1.0, minutes_eating)
		
		# Always generate a report in debug mode, respect hourly report flag otherwise
		if debug_mode or enable_hourly_eating_reports:
			print("HOURLY EATING REPORT - Hour %d: Food %.2f/%.2f (%.2f%%), Eating Time: %d mins, Food Gained: %.2f (%.2f/min), Est. Time to Full: %.2f hrs" % [
				current_hour, 
				food, 
				max_food, 
				food_percent, 
				minutes_eating, 
				food_gained, 
				food_gain_rate, 
				(max_food - food) / food_recovery_rate
			])
	
		# Additional debug information
		# Explicitly convert minutes_eating to an integer before using modulo
		if debug_mode and int(minutes_eating) % 10 == 0:
			print("EATING DEBUG: Current food: %.2f, Food gained: %.2f, Eating duration: %d mins" % [
				food, 
				food - eating_start_food, 
				int(minutes_eating)  # Explicitly convert to integer
			])

# Show eating progress report
# Optional: Update _show_eating_report method for consistency
func _show_eating_report(current_hour):
	# Calculate current total minutes
	var current_minute = current_hour * 60 + time_manager.minutes
	
	# Calculate minutes spent eating
	var minutes_eating = current_minute - eating_start_minute
	
	# Handle day wrapping
	if minutes_eating < 0:
		minutes_eating += 24 * 60
	
	# Convert minutes to fractional hours
	var hours_eating = minutes_eating / 60.0
	
	# Calculate other metrics
	var food_gained = food - eating_start_food
	var food_percent = (food / max_food) * 100
	
	# Updated print statement with minutes and hours
	print("HOURLY EATING REPORT - Hour %d: Food %.2f/%.2f (%.2f%%), Eating Time: %.2f hrs (%d mins), Food Gained: %.2f (%.2f/hr), Est. Time to Full: %.2f hrs" % [
		current_hour, 
		food, 
		max_food, 
		food_percent, 
		hours_eating, 
		minutes_eating, 
		food_gained, 
		food_gained / max(1.0, hours_eating), 
		(max_food - food) / food_recovery_rate
	])
	
# Updated _handle_eating_complete method for precise time tracking
# Updated _handle_eating_complete method
func _handle_eating_complete():
	# Calculate current total minutes
	var current_minute = time_manager.hours * 60 + time_manager.minutes
	
	# Calculate minutes spent eating
	var minutes_eating = current_minute - eating_start_minute
	
	# Handle day wrapping (if eating spans midnight)
	if minutes_eating < 0:
		minutes_eating += 24 * 60
	
	# Calculate food gained
	var food_gained = food - eating_start_food
	
	# Comprehensive debug output focused on minutes
	print("EATING COMPLETE: Farmer finished eating in %d minutes, started with %.2f food, now at %.2f food, gained %.2f food (%.2f per minute). Farmer is full and ready to work" % [
		minutes_eating, 
		eating_start_food, 
		food, 
		food_gained, 
		food_gained / max(1.0, minutes_eating)
	])
	
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
	if debug_mode:
		print("Navigation finished at: ", current_location)
	
	# Take action based on where we arrived
	if state_machine.current_state == NPCStates.FarmerState.WALKING_TO_FIELD:
		# Check if we're close enough to the field position
		var distance_to_field = global_position.distance_to(field_position)
		if distance_to_field <= arrival_distance:
			at_field = true
			if debug_mode:
				print("Farmer has reached the field!")
		else:
			at_field = false
			if debug_mode:
				print("Farmer isn't quite at the field center. Distance: ", distance_to_field)
		
		state_machine.change_state(NPCStates.FarmerState.WORKING)
	elif state_machine.current_state == NPCStates.FarmerState.WALKING_HOME:
		at_field = false
		# When arriving home, decide whether to eat or sleep
		if time_manager.hours >= 22:
			state_machine.change_state(NPCStates.FarmerState.GOING_TO_BED)
		else:
			state_machine.change_state(NPCStates.FarmerState.EATING)

func _handle_state_change(old_state, new_state):
	# Set a cooldown to prevent rapid state changes
	decision_cooldown = 1.0
	
	# Handle state entry actions
	match new_state:
		NPCStates.FarmerState.WALKING_TO_FIELD:
			current_destination = "Field"
			if debug_mode:
				print("Setting destination to field: ", field_position)
			navigate_to(field_position)
		NPCStates.FarmerState.WALKING_HOME:
			current_destination = "Home"
			if debug_mode:
				print("Setting destination to home: ", home_position)
			navigate_to(home_position)
		NPCStates.FarmerState.EATING:
			# Only log eating start if not already logged this hour
			if time_manager.hours != last_eating_report_hour:
				eating_start_food = food  # Track starting food level
				eating_start_hour = time_manager.hours  # Track hour when eating started
				last_eating_hour = time_manager.hours
				
				# Track precise start minute of eating
				eating_start_minute = time_manager.hours * 60 + time_manager.minutes
				
				print("Starting to eat at hour %d: Initial food level %.2f/%.2f (%.2f%%)" % [last_eating_hour, food, max_food, food/max_food*100])
				
				# Reset the last eating report hour to prevent duplicate reports
				last_eating_report_hour = time_manager.hours

# Make decisions based on current needs
func _make_decisions():
	# Print current stats only for active states (not sleeping)
	if state_machine.current_state != NPCStates.FarmerState.SLEEPING and debug_mode:
		print("Making decisions. Current state: ", state_machine.get_state_name(state_machine.current_state), 
			" Food: ", "%.2f" % food, "/", "%.2f" % max_food)
	
	# Don't make decisions on cooldown
	if state_machine.is_on_cooldown():
		return
	
	# Handle extreme need
	if food <= starving_threshold and state_machine.current_state != NPCStates.FarmerState.EATING:
		# Too hungry, need to eat
		if debug_mode:
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
				if debug_mode:
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
	
	if debug_mode:
		print("FARMER: Determining starting state based on hour: ", hour)
	
	# Place character in appropriate location & state for the time
	if hour >= 0 and hour < 7:
		global_position = bed_position
		if debug_mode:
			print("FARMER: It's nighttime (", hour, "), should be sleeping")
		state_machine.change_state(NPCStates.FarmerState.SLEEPING)
	elif hour >= 7 and hour < 22:
		# During the day, decide based on needs
		if food < hungry_threshold:
			global_position = kitchen_position
			if debug_mode:
				print("FARMER: Daytime and hungry, should be eating")
			state_machine.change_state(NPCStates.FarmerState.EATING)
			eating_start_food = food
			eating_start_hour = hour
			last_eating_hour = hour
		else:
			global_position = field_position
			if debug_mode:
				print("FARMER: Daytime and not hungry, should be working")
			state_machine.change_state(NPCStates.FarmerState.WORKING)
	else: # 22 or 23
		global_position = home_position
		if debug_mode:
			print("FARMER: Evening, should be going to bed")
		state_machine.change_state(NPCStates.FarmerState.GOING_TO_BED)
		
	if debug_mode:
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
