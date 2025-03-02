extends CharacterBody3D

class_name Enemy

# Enemy properties
@export var max_health: int = 50
@export var current_health: int = 50 
@export var move_speed: float = 3.0
@export var detection_radius: float = 5.0
@export var attack_radius: float = 1.5
@export var attack_damage: int = 10
@export var attack_cooldown: float = 1.0

# Enemy states
enum State {IDLE, PATROL, CHASE, ATTACK, DEAD}
var current_state = State.IDLE

# Navigation
var patrol_points = []
var current_patrol_index = 0
var path = []
var path_index = 0

# Target
var player = null
var can_attack = true

# Inventory and Loot
var inventory = []
@export var loot_table = {
	"common": [
		{"item": "health_potion", "weight": 70, "quantity": [1, 2]},
		{"item": "gold", "weight": 100, "quantity": [5, 20]}
	],
	"uncommon": [
		{"item": "mana_potion", "weight": 50, "quantity": [1, 1]},
		{"item": "arrows", "weight": 60, "quantity": [5, 10]}
	],
	"rare": [
		{"item": "sword", "weight": 10, "quantity": [1, 1]},
		{"item": "shield", "weight": 15, "quantity": [1, 1]}
	]
}
@export var drop_chance = {
	"common": 0.8,
	"uncommon": 0.4,
	"rare": 0.1
}

# References
@onready var detection_area = $DetectionArea

func _ready():
	# Initialize inventory based on enemy type
	_generate_inventory()
	
	# Connect detection area signal
	detection_area.body_entered.connect(_on_detection_area_body_entered)
	detection_area.body_exited.connect(_on_detection_area_body_exited)

func _physics_process(delta):
	match current_state:
		State.IDLE:
			_idle_behavior(delta)
		State.PATROL:
			_patrol_behavior(delta)
		State.CHASE:
			_chase_behavior(delta)
		State.ATTACK:
			_attack_behavior(delta)
		State.DEAD:
			pass  # No behavior when dead

func _idle_behavior(delta):
	# Simply stay in place, maybe look around occasionally
	pass

func _patrol_behavior(delta):
	# Move between patrol points
	# Implementation will depend on navigation system
	pass

func _chase_behavior(delta):
	if player:
		# Calculate direction to player
		var direction = (player.global_position - global_position).normalized()
		direction.y = 0  # Keep movement on the XZ plane
		
		# Set velocity and move
		velocity = direction * move_speed
		move_and_slide()
		
		# Check if close enough to attack
		if global_position.distance_to(player.global_position) <= attack_radius:
			current_state = State.ATTACK

func _attack_behavior(delta):
	if player and can_attack:
		# Face the player
		look_at(Vector3(player.global_position.x, global_position.y, player.global_position.z))
		
		# If in range, attack
		if global_position.distance_to(player.global_position) <= attack_radius:
			_attack_player()
		else:
			current_state = State.CHASE

func _attack_player():
	if can_attack:
		# Simple attack implementation
		print("Enemy attacks player for ", attack_damage, " damage!")
		
		# Signal or direct call to damage player
		# This depends on how you implement player health
		if player.has_method("take_damage"):
			player.take_damage(attack_damage)
		
		# Start cooldown
		can_attack = false
		await get_tree().create_timer(attack_cooldown).timeout
		can_attack = true

func take_damage(amount):
	current_health -= amount
	print("Enemy took ", amount, " damage! Health: ", current_health)
	
	if current_health <= 0:
		die()
	else:
		# When hit, always chase the attacker
		current_state = State.CHASE

func die():
	current_state = State.DEAD
	print("Enemy died!")
	
	# Drop loot
	_drop_loot()
	
	# Play death animation, then queue_free()
	# For now, just remove after a delay
	await get_tree().create_timer(1.0).timeout
	queue_free()

func _generate_inventory():
	# Random inventory based on loot tables
	# For now, just add some random items
	for category in loot_table:
		if randf() < drop_chance[category]:
			var category_items = loot_table[category]
			var total_weight = 0
			
			# Calculate total weight
			for item in category_items:
				total_weight += item.weight
				
			# Random roll
			var roll = randf() * total_weight
			var current_weight = 0
			
			# Select item based on weight
			for item in category_items:
				current_weight += item.weight
				if roll <= current_weight:
					# Generate random quantity
					var min_qty = item.quantity[0]
					var max_qty = item.quantity[1]
					var quantity = randi_range(min_qty, max_qty)
					
					# Add to inventory
					inventory.append({
						"item": item.item,
						"quantity": quantity
					})
					break

func _drop_loot():
	# For each item in inventory, create a collectible in the world
	print("Enemy drops: ", inventory)
	# Here you would instance actual item scenes
	# For now, we'll just print what was dropped

func _on_detection_area_body_entered(body):
	if body.is_in_group("player"):
		print("Player detected!")
		player = body
		current_state = State.CHASE

func _on_detection_area_body_exited(body):
	if body.is_in_group("player"):
		print("Player lost!")
		player = null
		current_state = State.PATROL  # Or back to IDLE
