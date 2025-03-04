# NPCBase.gd
# Base class for all AI-controlled NPCs with component-based system
extends CharacterBody3D
class_name NPCBase

# Debug flags to control verbosity and optimization
@export var debug_mode: bool = false

# Reference to components
@onready var navigation = $Navigation
@onready var collision = $Collision
@onready var needs = $Needs
var state_machine = null  # Will be initialized by derived classes

# Signals that components can connect to
signal state_changed(old_state, new_state)

# Debug timer
var debug_timer = 0.0

func _ready():
	if debug_mode:
		print("NPCBase._ready called for ", get_npc_type())
	
	# Initialize components
	navigation.initialize(self)
	collision.initialize(self)
	needs.initialize(self)
	
	# Note: State machine is now initialized in derived classes
	
	# Initialize NPC-specific behavior
	if debug_mode:
		print("NPCBase calling _init_npc")
	_init_npc()
	
	# Print state machine info if it exists
	if state_machine and debug_mode:
		print("State machine initialization complete. Current state: ", 
			 state_machine.get_state_name(state_machine.current_state))
	elif not state_machine:
		print("WARNING: No state machine initialized!")

# Virtual method to be overridden by derived classes
func _init_npc():
	if debug_mode:
		print("NPCBase._init_npc called - should be overridden by derived class")
	pass

func _physics_process(delta):
	# Update debug timer
	debug_timer += delta
	
	# Update components
	if navigation and navigation.enabled:
		navigation.process_navigation(delta)
	
	if collision and collision.enabled:
		collision.process_collisions(delta)
	
	if needs and needs.enabled:
		needs.process_needs(delta)
	
	# Update state machine
	if state_machine:
		state_machine.update(delta)
	else:
		if debug_mode: 
			print("WARNING: No state machine in _physics_process!")
		return
	
	# Process behavior based on current state
	_process_current_state(delta)

# To be implemented by derived classes
func _process_current_state(delta):
	pass

# Called when state changes
func _on_state_changed(old_state, new_state):
	# Emit signal so components can respond
	emit_signal("state_changed", old_state, new_state)
	
	# Inform derived classes
	_handle_state_change(old_state, new_state)

# Handle state change (to be overridden by derived classes)
func _handle_state_change(old_state, new_state):
	pass

# Get location name based on position (to be implemented by derived classes)
func _get_location_name(position):
	return "unknown location"

# Return specific NPC type for debugging
func get_npc_type():
	return "NPCBase"
