# NPCNeeds.gd
# Handles all need-based behavior for NPCs (hunger, energy, etc.)
extends Node
class_name NPCNeeds

# Needs configuration
@export_group("Needs")
@export var enabled: bool = true
@export var max_food: float = 100.0
@export var food: float = 50.0
@export var food_depletion_rate: float = 8.0

# Reference to owner
var owner_npc = null

# Depletion modifier (based on activity)
var depletion_modifier: float = 1.0

# Signals
signal food_critical(current_food, max_food)
signal food_low(current_food, max_food)
signal food_normal(current_food, max_food)
signal food_full(current_food, max_food)

# Initialize with owner reference
func initialize(npc):
	owner_npc = npc
	
	# Connect to state changes
	owner_npc.state_changed.connect(_on_state_changed)

# Process needs update in physics update
func process_needs(delta):
	# Base implementation - handle food depletion with state modifier
	update_food(-delta * food_depletion_rate * 0.5 * depletion_modifier)

# Update food level
func update_food(amount):
	var old_food = food
	food += amount
	food = clamp(food, 0, max_food)
	
	# Emit signals for food level changes if we crossed a threshold
	if old_food > 25.0 and food <= 25.0:
		emit_signal("food_critical", food, max_food)
	elif old_food > 50.0 and food <= 50.0:
		emit_signal("food_low", food, max_food)
	elif old_food <= 90.0 and food > 90.0:
		emit_signal("food_full", food, max_food)

# Eat food to recover
func eat(amount):
	food += amount
	food = clamp(food, 0, max_food)
	return food

# Get current food percentage
func get_food_percentage():
	return (food / max_food) * 100.0

# Get hunger state
func is_hungry():
	return food < 50.0

func is_starving():
	return food < 25.0

func is_full():
	return food >= 90.0

# Set the depletion modifier based on activity
func set_depletion_modifier(modifier):
	depletion_modifier = modifier

# React to state changes
func _on_state_changed(old_state, new_state):
	# Adjust need depletion rates based on state
	# This will be handled by the NPC script when it calls set_depletion_modifier
	pass

# Get a text description of the current need state
func get_need_description():
	if is_starving():
		return "Starving"
	elif is_hungry():
		return "Hungry"
	elif is_full():
		return "Full"
	else:
		return "Satisfied"
