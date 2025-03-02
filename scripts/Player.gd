extends CharacterBody3D

class_name Player

# Player stats
@export var max_health: int = 100
@export var current_health: int = 100
@export var max_mana: int = 50
@export var current_mana: int = 50
@export var base_speed: float = 5.0
@export var base_damage: int = 10
@export var attack_range: float = 1.5
@export var attack_cooldown: float = 0.5

# Movement
var direction = Vector3.ZERO
var is_attacking = false
var can_attack = true

# References
@onready var animation_player = $AnimationPlayer
@onready var attack_area = $AttackArea
@onready var attack_visual = $AttackArea/AttackVisual
@onready var inventory = $Inventory

# System access
var item_database

func _ready():
	# Access the item database singleton
	item_database = get_node("/root/ItemDatabase")
	
	# Add to player group
	add_to_group("player")
	
	# Give starting equipment
	if item_database:
		var sword = item_database.get_item("sword")
		if sword:
			inventory.add_item(sword)
	
	# Make sure attack visual is initially hidden
	if attack_visual:
		attack_visual.visible = false

func _physics_process(delta):
	# Handle input for movement
	_handle_movement(delta)
	
	# Handle attack input
	_handle_attack_input()

func _handle_movement(delta):
	# Get input direction
	var input_dir = Input.get_vector("ui_left", "ui_right", "ui_up", "ui_down")
	
	# Calculate direction in 3D space (XZ plane)
	direction = Vector3(input_dir.x, 0, input_dir.y).normalized()
	
	# Set velocity based on direction and speed
	if direction:
		velocity.x = direction.x * base_speed
		velocity.z = direction.z * base_speed
	else:
		velocity.x = move_toward(velocity.x, 0, base_speed)
		velocity.z = move_toward(velocity.z, 0, base_speed)
	
	# Facing direction
	if direction != Vector3.ZERO:
		var look_direction = Vector3(velocity.x, 0, velocity.z).normalized()
		var target_rotation = atan2(look_direction.x, look_direction.z)
		rotation.y = lerp_angle(rotation.y, target_rotation, 10 * delta)
	
	# Apply movement
	move_and_slide()

func _handle_attack_input():
	if Input.is_action_just_pressed("attack") and can_attack and not is_attacking:
		attack()

func attack():
	is_attacking = true
	can_attack = false
	
	print("Player attacks!")
	
	# Get equipped weapon
	var weapon = inventory.get_equipped_item("weapon")
	var damage = base_damage
	
	if weapon:
		damage += weapon.damage
	
	# Show attack visual
	if attack_visual:
		attack_visual.visible = true
	
	# Play attack animation
	if animation_player and animation_player.has_animation("attack"):
		animation_player.play("attack")
		# Wait for animation
		await animation_player.animation_finished
	else:
		# If no animation, just wait a bit
		await get_tree().create_timer(0.3).timeout
	
	# Check for enemies in attack range with improved debugging
	var bodies = attack_area.get_overlapping_bodies()
	print("Bodies detected in attack range: ", bodies.size())
	
	for body in bodies:
		print("Found body: ", body.name, " (", body.get_class(), ")")
		
		# Skip the player (prevent self-damage)
		if body == self:
			print("Skipping self")
			continue
		
		# Try multiple detection methods
		var is_enemy_class = body is Enemy
		var has_take_damage = body.has_method("take_damage")
		var is_in_enemy_group = body.is_in_group("enemy")
		
		print("Is Enemy class: ", is_enemy_class)
		print("Has take_damage method: ", has_take_damage)
		print("In 'enemy' group: ", is_in_enemy_group)
		
		# Try different approaches to damage
		if is_enemy_class and has_take_damage:
			body.take_damage(damage)
			print("Hit enemy via class check for ", damage, " damage!")
		elif is_in_enemy_group and has_take_damage:
			body.take_damage(damage)
			print("Hit enemy via group check for ", damage, " damage!")
		elif has_take_damage:
			body.take_damage(damage) 
			print("Hit object with take_damage for ", damage, " damage!")
	
	# Hide attack visual
	if attack_visual:
		attack_visual.visible = false
	
	# Reset attack state
	is_attacking = false
	
	# Start cooldown
	await get_tree().create_timer(attack_cooldown).timeout
	can_attack = true

func take_damage(amount):
	current_health -= amount
	print("Player took ", amount, " damage! Health: ", current_health)
	
	# Play hit animation
	if animation_player and animation_player.has_animation("hit"):
		animation_player.play("hit")
	
	if current_health <= 0:
		die()

func die():
	print("Player died!")
	
	# Play death animation
	if animation_player and animation_player.has_animation("death"):
		animation_player.play("death")
		await animation_player.animation_finished
	
	# Game over logic
	# For now, just restart the level
	get_tree().reload_current_scene()

func heal(amount):
	current_health = min(current_health + amount, max_health)
	print("Player healed for ", amount, "! Health: ", current_health)

func restore_mana(amount):
	current_mana = min(current_mana + amount, max_mana)
	print("Player restored ", amount, " mana! Mana: ", current_mana)

func use_item(item_id):
	var item = inventory.get_item(item_id)
	if item:
		if item.use(self):
			inventory.remove_item(item_id, 1)
			return true
	return false

func collect_dropped_item(dropped_item):
	if dropped_item is DroppedItem:
		var item = dropped_item.item_resource
		var quantity = dropped_item.quantity
		
		if inventory.add_item(item, quantity):
			print("Collected ", item.name, " x", quantity)
			dropped_item.queue_free()
			return true
	
	return false
