# upgrade_menu.gd
# Always-visible town management sidebar
extends CanvasLayer

signal upgrade_purchased(upgrade_type: String, new_level: int)

@onready var panel: PanelContainer = $Panel
@onready var food_label: Label = $Panel/MarginContainer/VBox/FoodRow/Value
@onready var farmers_label: Label = $Panel/MarginContainer/VBox/FarmersRow/Value
@onready var guards_label: Label = $Panel/MarginContainer/VBox/GuardsRow/Value
@onready var farms_label: Label = $Panel/MarginContainer/VBox/FarmsRow/Value
@onready var spawn_label: Label = $Panel/MarginContainer/VBox/SpawnRow/Value

# Upgrade rows: [level_label, button, upgrade_key, checkbox]
var upgrade_rows: Array = []

# Auto-upgrade settings key
const AUTO_UPGRADE_KEY := "auto_upgrades"

# Tooltip descriptions for each upgrade
const TOOLTIPS := {
	"guard_health": "+10% guard HP per level",
	"guard_attack": "+10% guard damage per level",
	"guard_range": "+5% guard attack range per level",
	"guard_size": "+5% guard size per level",
	"guard_attack_speed": "-8% attack cooldown per level\nGuards attack faster",
	"guard_move_speed": "+5% guard movement speed per level",
	"alert_radius": "+10% alert radius per level\nGuards detect enemies from farther",
	"farm_yield": "+15% food production per level",
	"farmer_hp": "+20% farmer HP per level",
	"healing_rate": "+20% HP regen at fountain per level",
	"food_efficiency": "10% chance per level to not consume food when eating",
	"farmer_cap": "+2 max farmers per level",
	"guard_cap": "+10 max guards per level",
	"fountain_radius": "+24px fountain healing range per level",
}

var main: Node
var npc_manager: Node
var town_idx: int = -1


func _ready() -> void:
	add_to_group("upgrade_menu")
	await get_tree().process_frame
	main = get_parent()
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	if main and "player_town_idx" in main:
		town_idx = main.player_town_idx

	# Setup upgrade rows with tooltips
	var vbox: VBoxContainer = $Panel/MarginContainer/VBox
	_setup_row(vbox.get_node("HealthRow"), "guard_health")
	_setup_row(vbox.get_node("AttackRow"), "guard_attack")
	_setup_row(vbox.get_node("RangeRow"), "guard_range")
	_setup_row(vbox.get_node("SizeRow"), "guard_size")
	_setup_row(vbox.get_node("AtkSpeedRow"), "guard_attack_speed")
	_setup_row(vbox.get_node("MoveSpeedRow"), "guard_move_speed")
	_setup_row(vbox.get_node("AlertRow"), "alert_radius")
	_setup_row(vbox.get_node("YieldRow"), "farm_yield")
	_setup_row(vbox.get_node("FarmerHpRow"), "farmer_hp")
	_setup_row(vbox.get_node("FarmerCapRow"), "farmer_cap")
	_setup_row(vbox.get_node("GuardCapRow"), "guard_cap")
	_setup_row(vbox.get_node("HealingRow"), "healing_rate")
	_setup_row(vbox.get_node("EfficiencyRow"), "food_efficiency")
	_setup_row(vbox.get_node("FountainRadiusRow"), "fountain_radius")


func _setup_row(row: HBoxContainer, upgrade_key: String) -> void:
	var level_label: Label = row.get_node("Level")
	var btn: Button = row.get_node("Button")
	btn.pressed.connect(_on_upgrade_pressed.bind(upgrade_key))
	btn.tooltip_text = TOOLTIPS.get(upgrade_key, "")

	# Add auto-upgrade checkbox
	var checkbox := CheckBox.new()
	checkbox.tooltip_text = "Auto-upgrade when food available"
	checkbox.toggled.connect(_on_auto_toggled.bind(upgrade_key))
	row.add_child(checkbox)

	# Load saved state
	var auto_settings: Dictionary = UserSettings.get_setting(AUTO_UPGRADE_KEY, {})
	checkbox.button_pressed = auto_settings.get(upgrade_key, false)

	upgrade_rows.append([level_label, btn, upgrade_key, checkbox])


func _process(_delta: float) -> void:
	if Engine.get_process_frames() % 10 != 0:
		return
	_process_auto_upgrades()
	_refresh()


func _refresh() -> void:
	if town_idx < 0 or not main:
		return
	if not "town_food" in main or not "town_upgrades" in main:
		return

	var food: int = main.town_food[town_idx]
	var upgrades: Dictionary = main.town_upgrades[town_idx]

	food_label.text = str(food)

	# Update population stats
	_refresh_stats()

	for row in upgrade_rows:
		var level_label: Label = row[0]
		var btn: Button = row[1]
		var key: String = row[2]
		var level: int = upgrades[key]
		_refresh_row(level_label, btn, level, food, key)


func _refresh_stats() -> void:
	if not npc_manager:
		return

	# Count farmers and guards for this town
	var farmer_count := 0
	var guard_count := 0
	for i in npc_manager.count:
		if npc_manager.healths[i] <= 0:
			continue
		if npc_manager.town_indices[i] != town_idx:
			continue
		var job: int = npc_manager.jobs[i]
		if job == NPCState.Job.FARMER:
			farmer_count += 1
		elif job == NPCState.Job.GUARD:
			guard_count += 1

	farmers_label.text = str(farmer_count)
	guards_label.text = str(guard_count)

	# Farm count
	if "towns" in main and town_idx < main.towns.size():
		var farm_count := 0
		for slot_key in main.towns[town_idx].slots:
			for building in main.towns[town_idx].slots[slot_key]:
				if building.type == "farm":
					farm_count += 1
		farms_label.text = str(farm_count)

	# Time until next spawn
	if "spawn_timers" in main and town_idx < main.spawn_timers.size():
		var hours_since: int = main.spawn_timers[town_idx]
		var hours_until: int = Config.SPAWN_INTERVAL_HOURS - hours_since
		if hours_until <= 0:
			spawn_label.text = "Soon"
		else:
			spawn_label.text = "%dh" % hours_until


func _refresh_row(level_label: Label, btn: Button, level: int, food: int, key: String = "") -> void:
	level_label.text = "Lv%d" % level

	if level >= Config.UPGRADE_MAX_LEVEL:
		btn.text = "MAX"
		btn.disabled = true
		btn.tooltip_text = _get_upgrade_tooltip(key, level)
	else:
		var cost: int = Config.get_upgrade_cost(level)
		btn.text = str(cost)
		btn.disabled = food < cost
		btn.tooltip_text = _get_upgrade_tooltip(key, level)


func _get_effective_stat(key: String, level: int) -> String:
	if level == 0:
		return ""

	match key:
		"guard_health":
			var mult: float = 1.0 + level * Config.UPGRADE_GUARD_HEALTH_BONUS
			return "%.0f HP (+%d%%)" % [Config.GUARD_HP * mult, int(level * Config.UPGRADE_GUARD_HEALTH_BONUS * 100)]
		"guard_attack":
			var mult: float = 1.0 + level * Config.UPGRADE_GUARD_ATTACK_BONUS
			return "%.1f dmg (+%d%%)" % [Config.GUARD_DAMAGE * mult, int(level * Config.UPGRADE_GUARD_ATTACK_BONUS * 100)]
		"guard_range":
			var mult: float = 1.0 + level * Config.UPGRADE_GUARD_RANGE_BONUS
			return "%.0f rng (+%d%%)" % [Config.GUARD_RANGE * mult, int(level * Config.UPGRADE_GUARD_RANGE_BONUS * 100)]
		"guard_size":
			var bonus: float = level * Config.UPGRADE_GUARD_SIZE_BONUS
			return "+%d%% size" % int(bonus * 100)
		"guard_attack_speed":
			var mult: float = 1.0 - level * Config.UPGRADE_GUARD_ATTACK_SPEED
			return "%.2fs cd (-%d%%)" % [Config.ATTACK_COOLDOWN * mult, int(level * Config.UPGRADE_GUARD_ATTACK_SPEED * 100)]
		"guard_move_speed":
			var mult: float = 1.0 + level * Config.UPGRADE_GUARD_MOVE_SPEED
			return "%.0f spd (+%d%%)" % [Config.MOVE_SPEED * mult, int(level * Config.UPGRADE_GUARD_MOVE_SPEED * 100)]
		"alert_radius":
			var mult: float = 1.0 + level * Config.UPGRADE_ALERT_RADIUS_BONUS
			return "%.0f rad (+%d%%)" % [Config.ALERT_RADIUS * mult, int(level * Config.UPGRADE_ALERT_RADIUS_BONUS * 100)]
		"farm_yield":
			return "+%d%% yield" % int(level * Config.UPGRADE_FARM_YIELD_BONUS * 100)
		"farmer_hp":
			var mult: float = 1.0 + level * Config.UPGRADE_FARMER_HP_BONUS
			return "%.0f HP (+%d%%)" % [Config.FARMER_HP * mult, int(level * Config.UPGRADE_FARMER_HP_BONUS * 100)]
		"healing_rate":
			return "+%d%% regen" % int(level * Config.UPGRADE_HEALING_RATE_BONUS * 100)
		"food_efficiency":
			var chance: float = level * Config.UPGRADE_FOOD_EFFICIENCY
			return "%d%% free meals" % int(chance * 100)
		"farmer_cap":
			var cap: int = Config.max_farmers_per_town + level * Config.UPGRADE_FARMER_CAP_BONUS
			return "%d max farmers" % cap
		"guard_cap":
			var cap: int = Config.max_guards_per_town + level * Config.UPGRADE_GUARD_CAP_BONUS
			return "%d max guards" % cap
		"fountain_radius":
			var radius: float = Config.BASE_FOUNTAIN_RADIUS + level * Config.UPGRADE_FOUNTAIN_RADIUS_BONUS
			return "%.0fpx range" % radius

	return ""


func _get_upgrade_tooltip(key: String, level: int) -> String:
	var base_desc: String = TOOLTIPS.get(key, "")
	var current_stat := _get_effective_stat(key, level)

	if level >= Config.UPGRADE_MAX_LEVEL:
		return "%s\nCurrent: %s (MAX)" % [base_desc, current_stat]

	var next_stat := _get_effective_stat(key, level + 1)
	if level > 0:
		return "%s\nCurrent: %s\nNext: %s" % [base_desc, current_stat, next_stat]
	return "%s\nNext: %s" % [base_desc, next_stat]


func _on_upgrade_pressed(upgrade_key: String) -> void:
	_try_purchase(upgrade_key)


func _on_auto_toggled(enabled: bool, upgrade_key: String) -> void:
	var auto_settings: Dictionary = UserSettings.get_setting(AUTO_UPGRADE_KEY, {})
	auto_settings[upgrade_key] = enabled
	UserSettings.set_setting(AUTO_UPGRADE_KEY, auto_settings)


func _try_purchase(upgrade_key: String) -> bool:
	if town_idx < 0 or not main:
		return false

	var upgrades: Dictionary = main.town_upgrades[town_idx]
	var level: int = upgrades[upgrade_key]
	if level >= Config.UPGRADE_MAX_LEVEL:
		return false

	var cost: int = Config.get_upgrade_cost(level)
	if main.town_food[town_idx] < cost:
		return false

	main.town_food[town_idx] -= cost
	upgrades[upgrade_key] = level + 1
	upgrade_purchased.emit(upgrade_key, level + 1)
	return true


func _process_auto_upgrades() -> void:
	if town_idx < 0 or not main:
		return

	# Process in order (top to bottom)
	for row in upgrade_rows:
		var upgrade_key: String = row[2]
		var checkbox: CheckBox = row[3]
		if checkbox.button_pressed:
			_try_purchase(upgrade_key)


# Toggle visibility
func open() -> void:
	panel.visible = true

func close() -> void:
	panel.visible = false

func toggle() -> void:
	panel.visible = not panel.visible

func is_open() -> bool:
	return panel.visible
