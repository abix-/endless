extends StaticBody3D

class_name DroppedItem

signal item_collected(item, quantity)

@export var item_resource: Item
@export var quantity: int = 1
@export var auto_destroy_time: float = 300.0  # 5 minutes default

@onready var area = $CollectionArea
@onready var mesh = $MeshInstance3D
@onready var animation_player = $AnimationPlayer
@onready var destroy_timer = $DestroyTimer

func _ready():
	# Set up mesh based on item type
	if item_resource:
		_setup_visual()
	
	# Connect area signal
	area.body_entered.connect(_on_collection_area_body_entered)
	
	# Set timer for auto-destroy
	destroy_timer.wait_time = auto_destroy_time
	destroy_timer.start()
	
	# Play spawn animation
	if animation_player.has_animation("spawn"):
		animation_player.play("spawn")
	
	# Start floating/rotating animation
	if animation_player.has_animation("idle"):
		animation_player.queue("idle")

func _setup_visual():
	# Set color based on rarity
	var material = StandardMaterial3D.new()
	material.albedo_color = item_resource.get_rarity_color()
	
	# Set default mesh based on item type
	# In a real game, you'd have specific models for each item
	match item_resource.type:
		0: # Weapon
			mesh.mesh = CylinderMesh.new()  # Sword-like
			mesh.mesh.top_radius = 0.05
			mesh.mesh.bottom_radius = 0.2
			mesh.mesh.height = 0.8
		1: # Armor
			mesh.mesh = BoxMesh.new()  # Box-like
			mesh.mesh.size = Vector3(0.5, 0.5, 0.2)
		2: # Consumable
			mesh.mesh = SphereMesh.new()  # Potion-like
			mesh.mesh.radius = 0.3
			mesh.mesh.height = 0.5
		3: # Material
			mesh.mesh = PrismMesh.new()  # Crystal-like
			mesh.mesh.size = Vector3(0.4, 0.4, 0.4)
		4: # Quest
			mesh.mesh = TorusMesh.new()  # Scroll-like
			mesh.mesh.inner_radius = 0.2
			mesh.mesh.outer_radius = 0.3
	
	mesh.material_override = material

func _on_collection_area_body_entered(body):
	if body.is_in_group("player") and body.has_node("Inventory"):
		var inventory = body.get_node("Inventory")
		
		# Try to add to inventory
		if inventory.add_item(item_resource, quantity):
			print("Player collected: ", item_resource.name, " x", quantity)
			
			# Emit signal
			emit_signal("item_collected", item_resource, quantity)
			
			# Play collection animation and destroy
			if animation_player.has_animation("collect"):
				animation_player.play("collect")
				await animation_player.animation_finished
				queue_free()
			else:
				queue_free()

func _on_destroy_timer_timeout():
	# Play fade out animation if exists
	if animation_player.has_animation("fade_out"):
		animation_player.play("fade_out")
		await animation_player.animation_finished
		queue_free()
	else:
		queue_free()
