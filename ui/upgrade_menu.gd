# upgrade_menu.gd
# Town upgrade menu for player-controlled towns
extends CanvasLayer

signal upgrade_purchased(upgrade_type: String, new_level: int)

@onready var panel: PanelContainer = $Panel
@onready var title_label: Label = $Panel/MarginContainer/VBox/Title
@onready var food_label: Label = $Panel/MarginContainer/VBox/FoodRow/Value

@onready var health_level: Label = $Panel/MarginContainer/VBox/HealthRow/Level
@onready var health_cost: Label = $Panel/MarginContainer/VBox/HealthRow/Cost
@onready var health_btn: Button = $Panel/MarginContainer/VBox/HealthRow/Button

@onready var attack_level: Label = $Panel/MarginContainer/VBox/AttackRow/Level
@onready var attack_cost: Label = $Panel/MarginContainer/VBox/AttackRow/Cost
@onready var attack_btn: Button = $Panel/MarginContainer/VBox/AttackRow/Button

@onready var range_level: Label = $Panel/MarginContainer/VBox/RangeRow/Level
@onready var range_cost: Label = $Panel/MarginContainer/VBox/RangeRow/Cost
@onready var range_btn: Button = $Panel/MarginContainer/VBox/RangeRow/Button

@onready var size_level: Label = $Panel/MarginContainer/VBox/SizeRow/Level
@onready var size_cost: Label = $Panel/MarginContainer/VBox/SizeRow/Cost
@onready var size_btn: Button = $Panel/MarginContainer/VBox/SizeRow/Button

var main: Node
var town_idx: int = -1


func _ready() -> void:
	panel.visible = false
	health_btn.pressed.connect(_on_health_upgrade)
	attack_btn.pressed.connect(_on_attack_upgrade)
	range_btn.pressed.connect(_on_range_upgrade)
	size_btn.pressed.connect(_on_size_upgrade)


func open(main_node: Node, idx: int) -> void:
	main = main_node
	town_idx = idx
	_refresh()
	panel.visible = true


func close() -> void:
	panel.visible = false


func is_open() -> bool:
	return panel.visible


func _refresh() -> void:
	if town_idx < 0 or not main:
		return

	var town_name: String = main.towns[town_idx].name
	var food: int = main.town_food[town_idx]
	var upgrades: Dictionary = main.town_upgrades[town_idx]

	title_label.text = "%s Upgrades" % town_name
	food_label.text = str(food)

	_refresh_row(health_level, health_cost, health_btn, upgrades.guard_health, food)
	_refresh_row(attack_level, attack_cost, attack_btn, upgrades.guard_attack, food)
	_refresh_row(range_level, range_cost, range_btn, upgrades.guard_range, food)
	_refresh_row(size_level, size_cost, size_btn, upgrades.guard_size, food)


func _refresh_row(level_label: Label, cost_label: Label, btn: Button, level: int, food: int) -> void:
	level_label.text = "Lv %d" % level
	if level >= Config.UPGRADE_MAX_LEVEL:
		cost_label.text = "MAX"
		btn.disabled = true
		btn.text = "Max"
	else:
		var cost: int = Config.UPGRADE_COSTS[level]
		cost_label.text = str(cost)
		btn.disabled = food < cost
		btn.text = "Upgrade"


func _purchase_upgrade(upgrade_key: String) -> void:
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


func _on_health_upgrade() -> void:
	_purchase_upgrade("guard_health")


func _on_attack_upgrade() -> void:
	_purchase_upgrade("guard_attack")


func _on_range_upgrade() -> void:
	_purchase_upgrade("guard_range")


func _on_size_upgrade() -> void:
	_purchase_upgrade("guard_size")


func _unhandled_input(event: InputEvent) -> void:
	if panel.visible and event is InputEventMouseButton:
		if event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
			var panel_rect: Rect2 = panel.get_global_rect()
			if not panel_rect.has_point(event.position):
				close()
				get_viewport().set_input_as_handled()
