extends Node

# This is a singleton that will store all item definitions
# Add it to your project's AutoLoad list

var items = {}

func _ready():
	# Register all items
	_register_items()

func _register_items():
	# Register weapons
	_register_weapon("sword", "Iron Sword", "A basic iron sword.", 5, 1.2, 1.5, 10)
	_register_weapon("axe", "Battle Axe", "A heavy battle axe.", 8, 0.8, 1.2, 15)
	_register_weapon("bow", "Hunting Bow", "A simple hunting bow.", 4, 1.5, 10.0, 12)
	
	# Register armor
	_register_armor("leather_helmet", "Leather Helmet", "Basic head protection.", 2, "Head", 5)
	_register_armor("leather_chest", "Leather Chestplate", "Basic torso protection.", 3, "Body", 8)
	_register_armor("iron_helmet", "Iron Helmet", "Solid head protection.", 5, "Head", 20)
	_register_armor("iron_chest", "Iron Chestplate", "Solid torso protection.", 8, "Body", 30)
	
	# Register consumables
	_register_consumable("health_potion", "Health Potion", "Restores 25 health points.", 25, 0, 0)
	_register_consumable("mana_potion", "Mana Potion", "Restores 25 mana points.", 0, 25, 0)
	_register_consumable("strength_potion", "Strength Potion", "Increases strength for 60 seconds.", 0, 0, 60)
	
	# Register materials
	_register_material("gold", "Gold Coin", "The standard currency.", 0.01, 1)
	_register_material("iron", "Iron Ingot", "A common crafting material.", 0.5, 2)
	_register_material("leather", "Leather", "Used for crafting basic armor.", 0.3, 1)
	_register_material("wood", "Wood", "A basic building material.", 0.2, 1)
	
	# Register quest items
	_register_quest_item("ancient_key", "Ancient Key", "Used to unlock mysterious doors.", 0.1, 0)
	_register_quest_item("magic_scroll", "Magic Scroll", "Contains mysterious writing.", 0.2, 0)

func get_item(id: String) -> Item:
	if items.has(id):
		return items[id]
	return null

func _register_weapon(id: String, name: String, description: String, damage: int, attack_speed: float, range: float, value: int, rarity: int = 0):
	var item = Item.new()
	item.id = id
	item.name = name
	item.description = description
	item.type = 0  # Weapon
	item.damage = damage
	item.attack_speed = attack_speed
	item.range = range
	item.value = value
	item.rarity = rarity
	item.weight = 2.0  # Default weight for weapons
	
	items[id] = item

func _register_armor(id: String, name: String, description: String, defense: int, slot: String, value: int, rarity: int = 0):
	var item = Item.new()
	item.id = id
	item.name = name
	item.description = description
	item.type = 1  # Armor
	item.defense = defense
	item.slot = slot
	item.value = value
	item.rarity = rarity
	item.weight = 3.0  # Default weight for armor
	
	items[id] = item

func _register_consumable(id: String, name: String, description: String, health_restore: int, mana_restore: int, effect_duration: float, value: int = 5, rarity: int = 0):
	var item = Item.new()
	item.id = id
	item.name = name
	item.description = description
	item.type = 2  # Consumable
	item.health_restore = health_restore
	item.mana_restore = mana_restore
	item.effect_duration = effect_duration
	item.value = value
	item.rarity = rarity
	item.weight = 0.5  # Default weight for consumables
	item.stackable = true
	item.max_stack_size = 10
	
	items[id] = item

func _register_material(id: String, name: String, description: String, weight: float, value: int, rarity: int = 0):
	var item = Item.new()
	item.id = id
	item.name = name
	item.description = description
	item.type = 3  # Material
	item.weight = weight
	item.value = value
	item.rarity = rarity
	item.stackable = true
	item.max_stack_size = 99
	
	items[id] = item

func _register_quest_item(id: String, name: String, description: String, weight: float, value: int, rarity: int = 2):
	var item = Item.new()
	item.id = id
	item.name = name
	item.description = description
	item.type = 4  # Quest
	item.weight = weight
	item.value = value
	item.rarity = rarity
	item.stackable = false
	
	items[id] = item

func create_random_loot_table(min_items: int = 1, max_items: int = 3) -> Dictionary:
	var loot_table = {
		"common": [],
		"uncommon": [],
		"rare": []
	}
	
	# Group items by rarity
	var common_items = []
	var uncommon_items = []
	var rare_items = []
	
	for id in items:
		var item = items[id]
		match item.rarity:
			0: common_items.append(id)
			1: uncommon_items.append(id)
			2, 3, 4: rare_items.append(id)
	
	# Generate random number of items for each category
	var num_items = randi_range(min_items, max_items)
	
	# Fill loot table
	for i in range(num_items):
		# Common items
		var common_id = common_items[randi() % common_items.size()]
		loot_table.common.append({
			"item": common_id,
			"weight": 70,
			"quantity": [1, 3]
		})
		
		# Uncommon items
		if uncommon_items.size() > 0:
			var uncommon_id = uncommon_items[randi() % uncommon_items.size()]
			loot_table.uncommon.append({
				"item": uncommon_id,
				"weight": 40,
				"quantity": [1, 2]
			})
		
		# Rare items
		if rare_items.size() > 0:
			var rare_id = rare_items[randi() % rare_items.size()]
			loot_table.rare.append({
				"item": rare_id,
				"weight": 10,
				"quantity": [1, 1]
			})
	
	return loot_table
