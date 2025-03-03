# NPCStates.gd
# State machine implementation and shared state definitions for NPC behaviors
class_name NPCStates

# NPC type identifier to ensure proper state separation
enum NPCType {
	FARMER = 1,
	BANDIT = 2
}

# Shared states all NPCs might use
enum BaseState {
	IDLE = 100,
	MOVING = 101,
	NAVIGATING_OBSTACLE = 102,
	COLLISION_RECOVERY = 103,
	WAITING_FOR_PATH = 104
}

# Farmer-specific states
enum FarmerState {
	SLEEPING = 200,
	WAKING_UP = 201,
	EATING = 202,
	WALKING_TO_FIELD = 203,
	WORKING = 204,
	WALKING_HOME = 205,
	GOING_TO_BED = 206
}

# Bandit-specific states
enum BanditState {
	PATROLLING = 300,
	CHASING = 301,
	ATTACKING = 302,
	FLEEING = 303
}

# State machine properties
var owner
var npc_type # Store the NPC type
var current_state
var previous_state
var state_timer = 0.0
var state_change_cooldown = 0.0
var state_names = {}

# Constructor with NPC type parameter
func _init(p_owner, p_npc_type):
	print("NPCStates constructor called with type: ", 
		  "FARMER" if p_npc_type == NPCType.FARMER else "BANDIT")
	
	owner = p_owner
	npc_type = p_npc_type
	
	# Initialize state names dictionary first
	_init_state_names()
	
	# Set an explicit default state that we know is safe
	current_state = BaseState.IDLE
	
	print("NPCStates initialized with type: ", 
		  "FARMER" if npc_type == NPCType.FARMER else "BANDIT", 
		  " and default state: ", get_state_name(current_state))

# Initialize state names for debug output
func _init_state_names():
	# Base states
	state_names[BaseState.IDLE] = "IDLE"
	state_names[BaseState.MOVING] = "MOVING"
	state_names[BaseState.NAVIGATING_OBSTACLE] = "NAVIGATING_OBSTACLE"
	state_names[BaseState.COLLISION_RECOVERY] = "COLLISION_RECOVERY"
	state_names[BaseState.WAITING_FOR_PATH] = "WAITING_FOR_PATH"
	
	# Farmer states
	state_names[FarmerState.SLEEPING] = "SLEEPING"
	state_names[FarmerState.WAKING_UP] = "WAKING_UP"
	state_names[FarmerState.EATING] = "EATING"
	state_names[FarmerState.WALKING_TO_FIELD] = "WALKING_TO_FIELD"
	state_names[FarmerState.WORKING] = "WORKING"
	state_names[FarmerState.WALKING_HOME] = "WALKING_HOME"
	state_names[FarmerState.GOING_TO_BED] = "GOING_TO_BED"
	
	# Bandit states
	state_names[BanditState.PATROLLING] = "PATROLLING"
	state_names[BanditState.CHASING] = "CHASING"
	state_names[BanditState.ATTACKING] = "ATTACKING"
	state_names[BanditState.FLEEING] = "FLEEING"
	
	print("State names dictionary initialized")

# Check if a state is valid for the current NPC type
func is_valid_state(state):
	# Base states are always valid
	if state in BaseState.values():
		return true
		
	# Check type-specific states
	if npc_type == NPCType.FARMER:
		return state in FarmerState.values()
	elif npc_type == NPCType.BANDIT:
		return state in BanditState.values()
		
	return false

# Change to a new state
func change_state(new_state):
	print("Attempting state change to: ", new_state, 
		  " (", get_state_name(new_state) if state_names.has(new_state) else "UNKNOWN", ")")
	
	# Extra validation with clear error messages and stack trace
	if npc_type == NPCType.FARMER and new_state in BanditState.values():
		print("ERROR: Farmer cannot use Bandit state: ", get_state_name(new_state))
		var stack = get_stack()
		for i in range(stack.size()):
			print("  Stack[", i, "]: ", stack[i].function, " in ", stack[i].source, ":", stack[i].line)
		return
	elif npc_type == NPCType.BANDIT and new_state in FarmerState.values():
		print("ERROR: Bandit cannot use Farmer state: ", get_state_name(new_state))
		var stack = get_stack()
		for i in range(stack.size()):
			print("  Stack[", i, "]: ", stack[i].function, " in ", stack[i].source, ":", stack[i].line)
		return
	
	# Validate the state change based on NPC type
	if not is_valid_state(new_state):
		print("ERROR: Invalid state change attempted for ", 
			  "FARMER" if npc_type == NPCType.FARMER else "BANDIT", 
			  " to state: ", get_state_name(new_state) if state_names.has(new_state) else "UNKNOWN")
		return
		
	# Don't change to the same state
	if current_state == new_state:
		print("Ignoring change to same state: ", get_state_name(current_state))
		return
		
	# Store previous state
	previous_state = current_state
	current_state = new_state
	
	# Reset state timer
	state_timer = 0.0
	
	# Notify owner of state change
	if owner.has_method("_on_state_changed"):
		owner._on_state_changed(previous_state, current_state)
	
	# Log state change
	print("NPC changing from ", get_state_name(previous_state), " to ", get_state_name(current_state))

# Update state machine
func update(delta):
	# Update state timer
	state_timer += delta
	
	# Update cooldown
	if state_change_cooldown > 0:
		state_change_cooldown -= delta

# Get state name for debugging
func get_state_name(state_value):
	if state_names.has(state_value):
		return state_names[state_value]
	return "UNKNOWN_STATE_" + str(state_value)

# Check if in a specific state
func is_in_state(state_value):
	return current_state == state_value

# Check if cooldown is active
func is_on_cooldown():
	return state_change_cooldown > 0

# Set cooldown
func set_cooldown(time):
	state_change_cooldown = time

# Debugging helper to list all state enum values
func print_state_values():
	print("BaseState values:")
	for val in BaseState.values():
		print("  ", val, ": ", get_state_name(val))
	
	print("FarmerState values:")
	for val in FarmerState.values():
		print("  ", val, ": ", get_state_name(val))
	
	print("BanditState values:")
	for val in BanditState.values():
		print("  ", val, ": ", get_state_name(val))

# Helper method to check if current state is a movement state
func is_movement_state():
	if npc_type == NPCType.FARMER:
		return current_state == BaseState.MOVING or \
			   current_state == FarmerState.WALKING_TO_FIELD or \
			   current_state == FarmerState.WALKING_HOME
	elif npc_type == NPCType.BANDIT:
		return current_state == BaseState.MOVING or \
			   current_state == BanditState.PATROLLING or \
			   current_state == BanditState.CHASING
			   
	return current_state == BaseState.MOVING
