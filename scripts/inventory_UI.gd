extends Control

var inventory = null
var item_slot_scene = preload("res://item_slot.tscn")

@onready var item_grid = $Panel/ItemGrid

func _ready():
	# Hide inventory UI by default
	visible = false

func initialize(player_inventory):
	inventory = player_inventory
	
	# Connect signals
	inventory.item_added.connect(_on_item_added)
	inventory.item_removed.connect(_on_item_removed)
	inventory.inventory_changed.connect(_on_inventory_changed)
	
	# Initial update
	update_ui()

func _input(event):
	# Toggle inventory with I key
	if event.is_action_pressed("toggle_inventory"):
		visible = !visible

func update_ui():
	# Clear existing items
	for child in item_grid.get_children():
		child.queue_free()
	
	# Add items from inventory
	for item_id in inventory.items:
		var item_data = inventory.items[item_id]
		var item = item_data.item
		var quantity = item_data.quantity
		
		var slot = item_slot_scene.instantiate()
		item_grid.add_child(slot)
		
		# Set item icon and quantity
		var texture_rect = slot.get_node("TextureRect")
		var label = slot.get_node("Label")
		
		# If item has an icon, set it
		if item.icon:
			texture_rect.texture = item.icon
		
		# Set quantity text
		if quantity > 1:
			label.text = str(quantity)
		else:
			label.text = ""

func _on_item_added(item, quantity):
	update_ui()

func _on_item_removed(item, quantity):
	update_ui()

func _on_inventory_changed():
	update_ui()
