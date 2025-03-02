extends Node

class_name Inventory

signal item_added(item, quantity)
signal item_removed(item, quantity)
signal inventory_changed()

# Dictionary to store items with their quantities
# Format: { item_id: { "item": Item, "quantity": int } }
var items = {}
@export var capacity: int = 20
var current_weight: float = 0.0
@export var max_weight: float = 50.0

# Equipment slots
var equipped_items = {
	"head": null,
	"body": null,
	"legs": null,
	"feet": null,
	"weapon": null,
	"shield": null,
	"accessory1": null,
	"accessory2": null
}

func add_item(item: Item, quantity: int = 1) -> bool:
	if item == null:
		return false
	
	# Check weight constraints
	if current_weight + (item.weight * quantity) > max_weight:
		print("Inventory: Too heavy to add item")
		return false
	
	# Check if already exists and is stackable
	if items.has(item.id) and item.stackable:
		items[item.id].quantity += quantity
	else:
		# Check capacity constraints
		if items.size() >= capacity:
			print("Inventory: No space for new item")
			return false
		
		# Add new item
		items[item.id] = {
			"item": item,
			"quantity": quantity
		}
	
	# Update weight
	current_weight += item.weight * quantity
	
	# Emit signals
	emit_signal("item_added", item, quantity)
	emit_signal("inventory_changed")
	
	return true

func remove_item(item_id: String, quantity: int = 1) -> bool:
	if not items.has(item_id):
		return false
	
	var item_data = items[item_id]
	
	# Check if we have enough
	if item_data.quantity < quantity:
		return false
	
	# Remove the quantity
	item_data.quantity -= quantity
	
	# Update weight
	current_weight -= item_data.item.weight * quantity
	
	# If quantity is 0, remove the item
	if item_data.quantity <= 0:
		items.erase(item_id)
	
	# Emit signals
	emit_signal("item_removed", item_data.item, quantity)
	emit_signal("inventory_changed")
	
	return true

func has_item(item_id: String, quantity: int = 1) -> bool:
	if not items.has(item_id):
		return false
	return items[item_id].quantity >= quantity

func get_item(item_id: String) -> Item:
	if not items.has(item_id):
		return null
	return items[item_id].item

func get_quantity(item_id: String) -> int:
	if not items.has(item_id):
		return 0
	return items[item_id].quantity

func equip_item(item_id: String) -> bool:
	if not items.has(item_id):
		return false
	
	var item = items[item_id].item
	var slot = ""
	
	# Determine slot based on item type
	match item.type:
		0: # Weapon
			slot = "weapon"
		1: # Armor
			slot = item.slot.to_lower()  # e.g., "head", "body", etc.
		_:
			# Other item types cannot be equipped
			return false
	
	# If slot is valid
	if equipped_items.has(slot):
		# Unequip current item if there is one
		if equipped_items[slot] != null:
			unequip_item(slot)
		
		# Equip new item
		equipped_items[slot] = item
		
		# Remove from inventory
		remove_item(item_id, 1)
		
		print("Equipped ", item.name, " in slot ", slot)
		emit_signal("inventory_changed")
		return true
	
	return false

func unequip_item(slot: String) -> bool:
	if not equipped_items.has(slot) or equipped_items[slot] == null:
		return false
	
	var item = equipped_items[slot]
	
	# Add back to inventory
	if add_item(item, 1):
		equipped_items[slot] = null
		print("Unequipped ", item.name, " from slot ", slot)
		emit_signal("inventory_changed")
		return true
	else:
		# If inventory is full
		print("Cannot unequip: inventory full")
		return false

func get_equipped_item(slot: String) -> Item:
	if not equipped_items.has(slot):
		return null
	return equipped_items[slot]

func clear():
	items.clear()
	current_weight = 0.0
	
	# Clear equipped items
	for slot in equipped_items:
		equipped_items[slot] = null
	
	emit_signal("inventory_changed")
