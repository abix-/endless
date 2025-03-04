# FarmerNPC.gd
# Farmer-specific behavior for farming simulation
extends NPCBase
class_name FarmerNPC

# Important locations
@export_group("Locations")
@export var home_position: Vector3 = Vector3.ZERO
@export var bed_position: Vector3 = Vector3.ZERO
@export var kitchen_position: Vector3 = Vector3.ZERO
@export var field_position: Vector3 = Vector3.ZERO

# Farmer-specific properties
@export_group("Farmer Properties")
@export var food_recovery_rate: float = 33.33  # Recover full food in ~3 hours
@export var hungry_threshold: float = 50.0
@export var starving_threshold: float = 25.0
@export var full_threshold: float = 90.0  # Consider "full" at 90% food
@export var enable_hourly_eating_reports: bool = true

# Time tracking
var eating_start_minute: int = 0
var eating_start_food = 0.0
var eating_start_hour = 0
var last_eating_hour = -1
var last_eating_report_hour: int = -1
var decision_cooldown = 0.0
var at_field: bool = false

# Location tracking
var current_destination = ""

# Reference to time manager
var time_manager = null

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
	state_machine.verbose_debugging = debug_mode
	
	# Setup state handler dictionary for cleaner code
	_setup_state_handlers()
	
	# Configure components
	_configure_components()
	
	# Connect to hour changed signal
	time_manager.hour_changed.connect(_on_hour_changed)
	
	# Connect to navigation completion signal
	navigation.navigation_finished.connect(_on_navigation_finished)
	
	# Connect to needs signals
	needs.food_critical.connect(_on_food_critical)
	needs.food_low.connect(_on_food_low)
	
	# Debug info
	if debug_mode:
		state_machine.print_state_values()
		print("FARMER INIT: Created state machine with type: ", 
			"FARMER" if state_machine.npc_type == NPCStates.NPCType.FARMER else "BANDIT")
	
	# Determine starting state based on time
	_determine_starting_state()
	
	if debug_mode:
		print("FARMER INIT: Completed initialization. Current state: ", 
			state_machine.get_state_name(state_machine.current_state))

# Configure components with farmer-specific settings
func _configure_components():
	# Configure needs component
	needs.food_depletion_rate = 8.0
	
	# Set navigation properties
	navigation.movement_speed = 10.0
	navigation.arrival_distance = 3.0

# Setup dictionary of state handlers
func _setup_state_handlers():
	# Farm-specific states
	state_handlers[NPCStates.FarmerState.SLEEPING] = _handle_sleeping
	state_handlers[NPCStates.FarmerState.WAKING_UP] = _handle_waking_up
	state_handlers[NPCStates.FarmerState.EATING] = _handle_eating
	state_handlers[NPCStates.FarmerState.WORKING] = _handle_working
	state_handlers[NPCStates.FarmerState.GOING_TO_BED] = _handle_going_to_bed
	
	# Navigation states handled by base handlers
	state_handlers[NPCStates.FarmerState.WALKING_TO_FIELD] = _handle_walking
	state_handlers[NPCStates.FarmerState.WALKING_HOME] = _handle_walking
	
	# Base states
	state_handlers[NPCStates.BaseState.IDLE] = _handle_idle

# Process state handler
func _process_current_state(delta):
	if state_machine == null:
		print("ERROR: No state machine in _process_current_state!")
		return
		
	var current_state = state_machine.current_state
	
	# Debug state periodically
	if debug_timer >= 5.0 and debug_mode:
		debug_timer = 0.0
		print("Current farmer state: ", state_machine.get_state_name(current_state), 
			  " | Food: ", "%.2f" % needs.food, "/", "%.2f" % needs.max_food, 
			  " (", "%.1f" % needs.get_food_percentage(), "%)")
	
	# Use the state handler dictionary
	if state_handlers.has(current_state):
		state_handlers[current_state].call(delta)
	elif current_state in NPCStates.BanditState.values():
		print("ERROR: Farmer is in Bandit state: ", state_machine.get_state_name(current_state))
		# Emergency recovery - intelligently choose state based on time and needs
		_handle_emergency_state_recovery()
	else:
		print("WARNING: Unhandled state in _process_current_state: ", state_machine.get_state_name(current_state))
	
	# Decrease decision cooldown if it's active
	if decision_cooldown > 0:
		decision_cooldown -= delta
	
	# Make decisions periodically (except during transitions or sleeping)
	if current_state not in [NPCStates.FarmerState.WAKING_UP, 
							NPCStates.FarmerState.GOING_TO_BED, 
							NPCStates.FarmerState.WALKING_HOME, 
							NPCStates.FarmerState.WALKING_TO_FIELD, 
							NPCStates.FarmerState.SLEEPING,
							NPCStates.BaseState.NAVIGATING_OBSTACLE,
							NPCStates.BaseState.WAITING_FOR_PATH,
							NPCStates.BaseState.COLLISION_RECOVERY]:
		if decision_cooldown <= 0:
			_make_decisions()
			decision_cooldown = 2.0

# Handle emergency state recovery
func _handle_emergency_state_recovery():
	if time_manager.hours >= 22 or time_manager.hours < 7:
		state_machine.change_state(NPCStates.FarmerState.SLEEPING)
	elif needs.food < hungry_threshold:
		state_machine.change_state(NPCStates.FarmerState.EATING)
	else:
		state_machine.change_state(NPCStates.FarmerState.WORKING)
	print("Emergency state recovery completed")

# Handlers for each state
func _handle_sleeping(delta):
	# Handled by needs component with sleeping modifier
	needs.set_depletion_modifier(0.1)

func _handle_working(delta):
	# Working depletes food faster
	needs.set_depletion_modifier(1.5)

func _handle_idle(delta):
	# Handle idle state - consider transitioning to a more meaningful state
	if decision_cooldown <= 0:
		_make_decisions()

# Handle walking state using navigation component
func _handle_walking(delta):
	# Navigation is handled by navigation component
	pass

# Handle eating behavior
func _handle_eating(delta):
	if debug_mode:
		print("DEBUG: _handle_eating called. Current state: %s" % state_machine.get_state_name(state_machine.current_state))
	
	# Recover food while eating (handled through needs component)
	var old_food = needs.food
	needs.eat(delta * food_recovery_rate)
	
	# Check if we're full and should go to work
	if needs.food >= full_threshold * needs.max_food / 100.0 and decision_cooldown <= 0:
		_handle_eating_complete()
	
	# Get current time details
	var current_hour = time_manager.hours
	var current_minute = current_hour * 60 + time_manager.minutes
	
	# Ensure we're tracking eating start correctly
	if eating_start_minute == 0:
		eating_start_minute = current_minute
		eating_start_food = needs.food
		last_eating_hour = current_hour
	
	# Handle eating reports (hourly)
	_process_eating_reports(current_hour, current_minute)

# Process eating reports
func _process_eating_reports(current_hour, current_minute):
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
		var food_gained = needs.food - eating_start_food
		var food_percent = needs.get_food_percentage()
		var food_gain_rate = food_gained / max(1.0, minutes_eating)
		
		# Always generate a report in debug mode, respect hourly report flag otherwise
		if debug_mode or enable_hourly_eating_reports:
			print("HOURLY EATING REPORT - Hour %d: Food %.2f/%.2f (%.2f%%), Eating Time: %d mins, Food Gained: %.2f (%.2f/min), Est. Time to Full: %.2f hrs" % [
				current_hour, 
				needs.food, 
				needs.max_food, 
				food_percent, 
				minutes_eating, 
				food_gained, 
				food_gain_rate, 
				(needs.max_food - needs.food) / food_recovery_rate
			])

# Handle eating complete
func _handle_eating_complete():
	# Calculate current total minutes
	var current_minute = time_manager.hours * 60 + time_manager.minutes
	
	# Calculate minutes spent eating
	var minutes_eating = current_minute - eating_start_minute
	
	# Handle day wrapping (if eating spans midnight)
	if minutes_eating < 0:
		minutes_eating += 24 * 60
	
	# Calculate food gained
	var food_gained = needs.food - eating_start_food
	
	# Comprehensive debug output focused on minutes
	print("EATING COMPLETE: Farmer finished eating in %d minutes, started with %.2f food, now at %.2f food, gained %.2f food (%.2f per minute). Farmer is full and ready to work" % [
		minutes_eating, 
		eating_start_food, 
		needs.food, 
		food_gained, 
		food_gained / max(1.0, minutes_eating)
	])
	
	state_machine.change_state(NPCStates.FarmerState.WALKING_TO_FIELD)

# Handle waking up transition
func _handle_waking_up(delta):
	# Use state timer from state machine
	if state_machine.state_timer >= 1.0:
		print("Finished waking up, now deciding what to do")
		if needs.is_hungry():
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

# Called when navigation finishes (connected to signal)
func _on_navigation_finished():
	var current_location = _get_location_name(global_position)
	if debug_mode:
		print("Navigation finished at: ", current_location)
	
	# Take action based on where we arrived
	if state_machine.current_state == NPCStates.FarmerState.WALKING_TO_FIELD:
		_handle_arrived_at_field()
	elif state_machine.current_state == NPCStates.FarmerState.WALKING_HOME:
		_handle_arrived_at_home()

# Handle arrival at field
func _handle_arrived_at_field():
	# Check if we're close enough to the field position
	var distance_to_field = global_position.distance_to(field_position)
	if distance_to_field <= navigation.arrival_distance:
		at_field = true
		if debug_mode:
			print("Farmer has reached the field!")
	else:
		at_field = false
		if debug_mode:
			print("Farmer isn't quite at the field center. Distance: ", distance_to_field)
	
	state_machine.change_state(NPCStates.FarmerState.WORKING)

# Handle arrival at home
func _handle_arrived_at_home():
	at_field = false
	# When arriving home, decide whether to eat or sleep
	if time_manager.hours >= 22:
		state_machine.change_state(NPCStates.FarmerState.GOING_TO_BED)
	else:
		state_machine.change_state(NPCStates.FarmerState.EATING)

# Respond to food critical signal
func _on_food_critical(current_food, max_food):
	if debug_mode:
		print("CRITICAL FOOD LEVEL: %.2f/%.2f (%.2f%%)" % [
			current_food, max_food, (current_food/max_food) * 100
		])
	
	# If not already handling the starvation
	if state_machine.current_state != NPCStates.FarmerState.WALKING_HOME and state_machine.current_state != NPCStates.FarmerState.EATING:
		print("Farmer is starving and needs to eat urgently!")
		decision_cooldown = 0 # Allow immediate decision
		_make_decisions() # This will handle the starvation

# Respond to food low signal
func _on_food_low(current_food, max_food):
	if debug_mode:
		print("LOW FOOD LEVEL: %.2f/%.2f (%.2f%%)" % [
			current_food, max_food, (current_food/max_food) * 100
		])

# Handle state changes
func _handle_state_change(old_state, new_state):
	# Set a cooldown to prevent rapid state changes
	decision_cooldown = 1.0
	
	# Reset needs depletion modifier
	needs.set_depletion_modifier(1.0)
	
	# Handle state entry actions
	match new_state:
		NPCStates.FarmerState.WALKING_TO_FIELD:
			current_destination = "Field"
			if debug_mode:
				print("Setting destination to field: ", field_position)
			navigation.navigate_to(field_position)
			
		NPCStates.FarmerState.WALKING_HOME:
			current_destination = "Home"
			if debug_mode:
				print("Setting destination to home: ", home_position)
			navigation.navigate_to(home_position)
			
		NPCStates.FarmerState.EATING:
			# Only log eating start if not already logged this hour
			if time_manager.hours != last_eating_report_hour:
				eating_start_food = needs.food
				eating_start_hour = time_manager.hours
				last_eating_hour = time_manager.hours
				
				# Track precise start minute of eating
				eating_start_minute = time_manager.hours * 60 + time_manager.minutes
				
				print("Starting to eat at hour %d: Initial food level %.2f/%.2f (%.2f%%)" % [
					last_eating_hour, 
					needs.food, 
					needs.max_food, 
					needs.get_food_percentage()
				])
				
				# Reset the last eating report hour to prevent duplicate reports
				last_eating_report_hour = time_manager.hours

# Make decisions based on current needs
func _make_decisions():
	# Print current stats only for active states (not sleeping)
	if state_machine.current_state != NPCStates.FarmerState.SLEEPING and debug_mode:
		print("Making decisions. Current state: ", state_machine.get_state_name(state_machine.current_state), 
			" Food: ", "%.2f" % needs.food, "/", "%.2f" % needs.max_food)
	
	# Don't make decisions on cooldown
	if state_machine.is_on_cooldown():
		return
	
	# Handle extreme need
	if needs.is_starving() and state_machine.current_state != NPCStates.FarmerState.EATING:
		# Too hungry, need to eat
		if debug_mode:
			print("Farmer is too hungry to continue working")
		if state_machine.current_state != NPCStates.FarmerState.WALKING_HOME and state_machine.current_state != NPCStates.FarmerState.EATING:
			state_machine.change_state(NPCStates.FarmerState.WALKING_HOME)
		return
	
	# Normal decision making based on state
	match state_machine.current_state:
		NPCStates.FarmerState.EATING:
			# If done eating (full), go to work
			if needs.is_full():
				_handle_eating_complete()
		
		NPCStates.FarmerState.WORKING:
			# If getting hungry, go home to eat
			if needs.is_hungry():
				if debug_mode:
					print("Farmer is getting hungry and heading home")
				state_machine.change_state(NPCStates.FarmerState.WALKING_HOME)
				
		NPCStates.BaseState.IDLE:
			# If in idle state, transition to appropriate state
			_handle_idle_decision()

# Handle decisions when in idle state
func _handle_idle_decision():
	if time_manager.hours >= 22 or time_manager.hours < 7:
		# It's nighttime
		state_machine.change_state(NPCStates.FarmerState.GOING_TO_BED)
	elif needs.is_hungry():
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
		if needs.is_hungry():
			global_position = kitchen_position
			if debug_mode:
				print("FARMER: Daytime and hungry, should be eating")
			state_machine.change_state(NPCStates.FarmerState.EATING)
			eating_start_food = needs.food
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
