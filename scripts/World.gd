extends Node3D

func _ready():
	# Assuming you have references to your player and inventory UI
	var player = $Player
	var inventory_ui = $CanvasLayer/InventoryUI
	var health_ui = $CanvasLayer/HealthUI  # Adjust path as needed
	
	# Connect inventory UI to player's inventory
	inventory_ui.initialize(player.inventory)
	
	# Connect health UI to player
	health_ui.initialize(player)
