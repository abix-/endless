# guard_post_menu.gd
# Popup menu for upgrading individual guard posts
extends CanvasLayer

signal upgrade_purchased(town_idx: int, slot_key: String, upgrade_type: String, new_level: int)

@onready var panel: PanelContainer = $Panel
@onready var title_label: Label = $Panel/VBox/Title
@onready var attack_btn: Button = $Panel/VBox/AttackBtn
@onready var range_btn: Button = $Panel/VBox/RangeBtn
@onready var damage_btn: Button = $Panel/VBox/DamageBtn
@onready var stats_label: Label = $Panel/VBox/Stats
@onready var close_btn: Button = $Panel/VBox/CloseBtn

var main_node: Node
var current_slot_key: String = ""
var current_town_idx: int = -1


func _ready() -> void:
	await get_tree().process_frame
	main_node = get_parent()

	attack_btn.pressed.connect(_on_attack_pressed)
	range_btn.pressed.connect(_on_range_pressed)
	damage_btn.pressed.connect(_on_damage_pressed)
	close_btn.pressed.connect(close)

	panel.visible = false


func _input(event: InputEvent) -> void:
	if not panel.visible:
		return

	if event is InputEventKey and event.pressed and event.keycode == KEY_ESCAPE:
		close()
		get_viewport().set_input_as_handled()

	# Close when clicking outside panel
	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		var rect := Rect2(panel.global_position, panel.size)
		if not rect.has_point(event.position):
			close()


func _process(_delta: float) -> void:
	if not panel.visible:
		return
	if Engine.get_process_frames() % 30 != 0:
		return
	_refresh_buttons()


func open(slot_key: String, town_idx: int, screen_pos: Vector2) -> void:
	current_slot_key = slot_key
	current_town_idx = town_idx

	title_label.text = "Guard Post %s" % slot_key.to_upper()
	_refresh_buttons()

	# Position near click, but keep on screen
	var viewport_size: Vector2 = get_viewport().get_visible_rect().size
	var panel_size := panel.size
	var pos := screen_pos + Vector2(10, 10)
	pos.x = minf(pos.x, viewport_size.x - panel_size.x - 10)
	pos.y = minf(pos.y, viewport_size.y - panel_size.y - 10)
	panel.position = pos

	panel.visible = true


func close() -> void:
	panel.visible = false
	current_slot_key = ""
	current_town_idx = -1


func _refresh_buttons() -> void:
	if not main_node or current_town_idx < 0:
		return

	var food: int = main_node.town_food[current_town_idx]
	var upgrades: Dictionary = main_node.guard_post_upgrades[current_town_idx].get(current_slot_key, {})

	var attack_enabled: bool = upgrades.get("attack_enabled", false)
	var range_level: int = upgrades.get("range_level", 0)
	var damage_level: int = upgrades.get("damage_level", 0)

	# Attack enable button
	if attack_enabled:
		attack_btn.text = "Attack: ENABLED"
		attack_btn.disabled = true
	else:
		var cost: int = Config.get_guard_post_upgrade_cost(0)
		attack_btn.text = "Enable Attack (%d food)" % cost
		attack_btn.disabled = food < cost

	# Range upgrade
	var range_cost: int = Config.get_guard_post_upgrade_cost(range_level)
	var range_stat: float = Config.GUARD_POST_BASE_RANGE * Config.get_guard_post_stat_scale(range_level)
	if range_level >= Config.GUARD_POST_MAX_LEVEL:
		range_btn.text = "Range Lv%d: %.0fpx (MAX)" % [range_level, range_stat]
		range_btn.disabled = true
	else:
		range_btn.text = "Range Lv%d: %.0fpx (%d food)" % [range_level, range_stat, range_cost]
		range_btn.disabled = not attack_enabled or food < range_cost

	# Damage upgrade
	var damage_cost: int = Config.get_guard_post_upgrade_cost(damage_level)
	var damage_stat: float = Config.GUARD_POST_BASE_DAMAGE * Config.get_guard_post_stat_scale(damage_level)
	if damage_level >= Config.GUARD_POST_MAX_LEVEL:
		damage_btn.text = "Damage Lv%d: %.1f dmg (MAX)" % [damage_level, damage_stat]
		damage_btn.disabled = true
	else:
		damage_btn.text = "Damage Lv%d: %.1f dmg (%d food)" % [damage_level, damage_stat, damage_cost]
		damage_btn.disabled = not attack_enabled or food < damage_cost

	# Stats display
	if attack_enabled:
		stats_label.text = "Range: %.0fpx | Damage: %.1f | CD: %.1fs" % [
			range_stat, damage_stat, Config.GUARD_POST_ATTACK_COOLDOWN
		]
	else:
		stats_label.text = "Inactive - enable attack first"


func _on_attack_pressed() -> void:
	var cost: int = Config.get_guard_post_upgrade_cost(0)
	if _try_purchase(cost):
		var upgrades: Dictionary = main_node.guard_post_upgrades[current_town_idx]
		if not upgrades.has(current_slot_key):
			upgrades[current_slot_key] = {"attack_enabled": false, "range_level": 0, "damage_level": 0}
		upgrades[current_slot_key].attack_enabled = true
		upgrade_purchased.emit(current_town_idx, current_slot_key, "attack_enabled", 1)
		_refresh_buttons()


func _on_range_pressed() -> void:
	var upgrades: Dictionary = main_node.guard_post_upgrades[current_town_idx].get(current_slot_key, {})
	var range_level: int = upgrades.get("range_level", 0)
	var cost: int = Config.get_guard_post_upgrade_cost(range_level)
	if _try_purchase(cost):
		main_node.guard_post_upgrades[current_town_idx][current_slot_key].range_level = range_level + 1
		upgrade_purchased.emit(current_town_idx, current_slot_key, "range_level", range_level + 1)
		_refresh_buttons()


func _on_damage_pressed() -> void:
	var upgrades: Dictionary = main_node.guard_post_upgrades[current_town_idx].get(current_slot_key, {})
	var damage_level: int = upgrades.get("damage_level", 0)
	var cost: int = Config.get_guard_post_upgrade_cost(damage_level)
	if _try_purchase(cost):
		main_node.guard_post_upgrades[current_town_idx][current_slot_key].damage_level = damage_level + 1
		upgrade_purchased.emit(current_town_idx, current_slot_key, "damage_level", damage_level + 1)
		_refresh_buttons()


func _try_purchase(cost: int) -> bool:
	if not main_node or current_town_idx < 0:
		return false

	if main_node.town_food[current_town_idx] < cost:
		return false

	main_node.town_food[current_town_idx] -= cost
	return true
