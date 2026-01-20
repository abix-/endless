# policies_panel.gd
# Panel for managing faction policies
extends CanvasLayer

@onready var panel: PanelContainer = $Panel
@onready var eat_food_check: CheckBox = $Panel/MarginContainer/VBox/EatFoodRow/CheckBox
@onready var farmer_flee_slider: HSlider = $Panel/MarginContainer/VBox/FarmerFleeRow/Slider
@onready var farmer_flee_label: Label = $Panel/MarginContainer/VBox/FarmerFleeRow/Value
@onready var guard_flee_slider: HSlider = $Panel/MarginContainer/VBox/GuardFleeRow/Slider
@onready var guard_flee_label: Label = $Panel/MarginContainer/VBox/GuardFleeRow/Value
@onready var recovery_slider: HSlider = $Panel/MarginContainer/VBox/RecoveryRow/Slider
@onready var recovery_label: Label = $Panel/MarginContainer/VBox/RecoveryRow/Value
@onready var guard_aggressive_check: CheckBox = $Panel/MarginContainer/VBox/AggressiveRow/CheckBox
@onready var guard_leash_check: CheckBox = $Panel/MarginContainer/VBox/LeashRow/CheckBox
@onready var farmer_fight_check: CheckBox = $Panel/MarginContainer/VBox/FarmerFightRow/CheckBox
@onready var prioritize_healing_check: CheckBox = $Panel/MarginContainer/VBox/HealingRow/CheckBox
@onready var schedule_option: OptionButton = $Panel/MarginContainer/VBox/ScheduleRow/Option
@onready var close_btn: Button = $Panel/MarginContainer/VBox/CloseBtn

var main_node: Node
var town_idx: int = -1


func _ready() -> void:
	await get_tree().process_frame
	main_node = get_parent()
	if main_node and "player_town_idx" in main_node:
		town_idx = main_node.player_town_idx

	# Connect signals
	eat_food_check.toggled.connect(_on_eat_food_toggled)
	farmer_flee_slider.value_changed.connect(_on_farmer_flee_changed)
	guard_flee_slider.value_changed.connect(_on_guard_flee_changed)
	recovery_slider.value_changed.connect(_on_recovery_changed)
	guard_aggressive_check.toggled.connect(_on_aggressive_toggled)
	guard_leash_check.toggled.connect(_on_leash_toggled)
	farmer_fight_check.toggled.connect(_on_farmer_fight_toggled)
	prioritize_healing_check.toggled.connect(_on_healing_toggled)
	schedule_option.item_selected.connect(_on_schedule_selected)
	close_btn.pressed.connect(close)

	# Setup schedule options
	schedule_option.add_item("Both Shifts", 0)
	schedule_option.add_item("Day Only", 1)
	schedule_option.add_item("Night Only", 2)

	panel.visible = false


func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed:
		if event.keycode == KEY_P and not panel.visible:
			open()
			get_viewport().set_input_as_handled()
		elif event.keycode == KEY_ESCAPE and panel.visible:
			close()
			get_viewport().set_input_as_handled()


func open() -> void:
	_refresh()
	panel.visible = true


func close() -> void:
	panel.visible = false


func _refresh() -> void:
	if not main_node or town_idx < 0:
		return
	if not "town_policies" in main_node:
		return

	var policies: Dictionary = main_node.town_policies[town_idx]

	eat_food_check.button_pressed = policies.eat_food
	farmer_flee_slider.value = policies.farmer_flee_hp * 100
	farmer_flee_label.text = "%d%%" % int(policies.farmer_flee_hp * 100)
	guard_flee_slider.value = policies.guard_flee_hp * 100
	guard_flee_label.text = "%d%%" % int(policies.guard_flee_hp * 100)
	recovery_slider.value = policies.recovery_hp * 100
	recovery_label.text = "%d%%" % int(policies.recovery_hp * 100)
	guard_aggressive_check.button_pressed = policies.guard_aggressive
	guard_leash_check.button_pressed = policies.guard_leash
	farmer_fight_check.button_pressed = policies.farmer_fight_back
	prioritize_healing_check.button_pressed = policies.prioritize_healing
	schedule_option.selected = policies.work_schedule


func _get_policies() -> Dictionary:
	if not main_node or town_idx < 0:
		return {}
	if not "town_policies" in main_node:
		return {}
	return main_node.town_policies[town_idx]


func _on_eat_food_toggled(enabled: bool) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.eat_food = enabled


func _on_farmer_flee_changed(value: float) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.farmer_flee_hp = value / 100.0
	farmer_flee_label.text = "%d%%" % int(value)


func _on_guard_flee_changed(value: float) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.guard_flee_hp = value / 100.0
	guard_flee_label.text = "%d%%" % int(value)


func _on_recovery_changed(value: float) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.recovery_hp = value / 100.0
	recovery_label.text = "%d%%" % int(value)


func _on_aggressive_toggled(enabled: bool) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.guard_aggressive = enabled


func _on_leash_toggled(enabled: bool) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.guard_leash = enabled


func _on_farmer_fight_toggled(enabled: bool) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.farmer_fight_back = enabled


func _on_healing_toggled(enabled: bool) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.prioritize_healing = enabled


func _on_schedule_selected(index: int) -> void:
	var policies := _get_policies()
	if policies.is_empty():
		return
	policies.work_schedule = index
