extends Resource

class_name Item

@export var id: String = "item_id"
@export var name: String = "Item Name"
@export var description: String = "Item description"
@export var icon: Texture
@export var stackable: bool = false
@export var max_stack_size: int = 1
@export var weight: float = 1.0
@export var value: int = 0
@export_enum("Common", "Uncommon", "Rare", "Epic", "Legendary") var rarity: int = 0
@export_enum("Weapon", "Armor", "Consumable", "Material", "Quest") var type: int = 0

# Properties specific to item types
@export_group("Weapon Properties")
@export var damage: int = 0
@export var attack_speed: float = 1.0
@export var range: float = 1.0

@export_group("Armor Properties")
@export var defense: int = 0
@export var slot: String = "Body"

@export_group("Consumable Properties")
@export var health_restore: int = 0
@export var mana_restore: int = 0
@export var effect_duration: float = 0.0

func use(entity):
	# Base use function
	match type:
		0: # Weapon
			return _use_weapon(entity)
		1: # Armor
			return _use_armor(entity)
		2: # Consumable
			return _use_consumable(entity)
		3: # Material
			return false # Materials typically can't be used directly
		4: # Quest
			return false # Quest items typically can't be used directly
	
	return false

func _use_weapon(entity):
	print(entity.name, " equipped weapon: ", name)
	# Equip logic would go here
	return true

func _use_armor(entity):
	print(entity.name, " equipped armor: ", name)
	# Equip logic would go here
	return true

func _use_consumable(entity):
	print(entity.name, " used consumable: ", name)
	
	# Apply effects
	if health_restore > 0 and entity.has_method("heal"):
		entity.heal(health_restore)
	
	if mana_restore > 0 and entity.has_method("restore_mana"):
		entity.restore_mana(mana_restore)
	
	# Return true to indicate item should be consumed
	return true

func get_rarity_color():
	match rarity:
		0: # Common
			return Color(0.7, 0.7, 0.7) # Light Gray
		1: # Uncommon
			return Color(0.0, 0.8, 0.0) # Green
		2: # Rare
			return Color(0.0, 0.5, 1.0) # Blue
		3: # Epic
			return Color(0.7, 0.0, 1.0) # Purple
		4: # Legendary
			return Color(1.0, 0.5, 0.0) # Orange
	
	return Color.WHITE
