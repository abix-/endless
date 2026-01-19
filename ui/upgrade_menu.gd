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

# Upgrade rows: [level_label, button, upgrade_key]
var upgrade_rows: Array = []

# Tooltip descriptions for each upgrade
const TOOLTIPS := {
	"guard_health": "+10% guard HP per level",
	"guard_attack": "+10% guard damage per level",
	"guard_range": "+5% guard attack range per level",
	"guard_size": "+5% guard size per level",
	"alert_radius": "+10% alert radius per level\nGuards detect enemies from farther",
	"farm_yield": "+15% food production per level",
	"farmer_hp": "+20% farmer HP per level",
	"healing_rate": "+20% HP regen at fountain per level",
	"food_efficiency": "10% chance per level to not consume food when eating",
}

var main: Node
var npc_manager: Node
var town_idx: int = -1


func _ready() -> void:
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
	_setup_row(vbox.get_node("AlertRow"), "alert_radius")
	_setup_row(vbox.get_node("YieldRow"), "farm_yield")
	_setup_row(vbox.get_node("FarmerHpRow"), "farmer_hp")
	_setup_row(vbox.get_node("HealingRow"), "healing_rate")
	_setup_row(vbox.get_node("EfficiencyRow"), "food_efficiency")


func _setup_row(row: HBoxContainer, upgrade_key: String) -> void:
	var level_label: Label = row.get_node("Level")
	var btn: Button = row.get_node("Button")
	btn.pressed.connect(_on_upgrade_pressed.bind(upgrade_key))
	btn.tooltip_text = TOOLTIPS.get(upgrade_key, "")
	upgrade_rows.append([level_label, btn, upgrade_key])


func _process(_delta: float) -> void:
	if Engine.get_process_frames() % 10 != 0:
		return
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
		_refresh_row(level_label, btn, level, food)


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
		var farm_count: int = main.towns[town_idx].farms.size()
		farms_label.text = str(farm_count)

	# Time until next spawn
	if "spawn_timers" in main and town_idx < main.spawn_timers.size():
		var hours_since: int = main.spawn_timers[town_idx]
		var hours_until: int = Config.SPAWN_INTERVAL_HOURS - hours_since
		if hours_until <= 0:
			spawn_label.text = "Soon"
		else:
			spawn_label.text = "%dh" % hours_until


func _refresh_row(level_label: Label, btn: Button, level: int, food: int) -> void:
	level_label.text = str(level)
	if level >= Config.UPGRADE_MAX_LEVEL:
		btn.text = "MAX"
		btn.disabled = true
	else:
		var cost: int = Config.UPGRADE_COSTS[level]
		btn.text = str(cost)
		btn.disabled = food < cost


func _on_upgrade_pressed(upgrade_key: String) -> void:
	if town_idx < 0 or not main:
		return

	var upgrades: Dictionary = main.town_upgrades[town_idx]
	var level: int = upgrades[upgrade_key]
	if level >= Config.UPGRADE_MAX_LEVEL:
		return

	var cost: int = Config.UPGRADE_COSTS[level]
	if main.town_food[town_idx] < cost:
		return

	main.town_food[town_idx] -= cost
	upgrades[upgrade_key] = level + 1
	upgrade_purchased.emit(upgrade_key, level + 1)
	_refresh()


# Legacy API for main.gd compatibility
func open(_main_node: Node, _idx: int) -> void:
	pass

func close() -> void:
	pass

func is_open() -> bool:
	return true
