# settings_menu.gd
# In-game settings menu
extends CanvasLayer

@onready var panel: PanelContainer = $Panel
@onready var hp_bars_dropdown: OptionButton = $Panel/MarginContainer/VBox/HpBarsRow/Dropdown
@onready var death_log_dropdown: OptionButton = $Panel/MarginContainer/VBox/DeathLogRow/Dropdown
@onready var level_log_dropdown: OptionButton = $Panel/MarginContainer/VBox/LevelLogRow/Dropdown
@onready var spawn_log_dropdown: OptionButton = $Panel/MarginContainer/VBox/SpawnLogRow/Dropdown
@onready var scroll_slider: HSlider = $Panel/MarginContainer/VBox/ScrollRow/Slider
@onready var scroll_label: Label = $Panel/MarginContainer/VBox/ScrollRow/Value

func _ready() -> void:
	panel.visible = false

	# HP bar dropdown
	hp_bars_dropdown.add_item("Off", 0)
	hp_bars_dropdown.add_item("When Damaged", 1)
	hp_bars_dropdown.add_item("Always", 2)
	hp_bars_dropdown.selected = UserSettings.hp_bar_mode
	hp_bars_dropdown.item_selected.connect(_on_hp_bars_selected)

	# Death log dropdown
	death_log_dropdown.add_item("Off", 0)
	death_log_dropdown.add_item("Own Faction", 1)
	death_log_dropdown.add_item("All", 2)
	death_log_dropdown.selected = UserSettings.death_log_mode
	death_log_dropdown.item_selected.connect(_on_death_log_selected)

	# Level log dropdown
	level_log_dropdown.add_item("Off", 0)
	level_log_dropdown.add_item("Own Faction", 1)
	level_log_dropdown.add_item("All", 2)
	level_log_dropdown.selected = UserSettings.level_log_mode
	level_log_dropdown.item_selected.connect(_on_level_log_selected)

	# Spawn log dropdown
	spawn_log_dropdown.add_item("Off", 0)
	spawn_log_dropdown.add_item("Own Faction", 1)
	spawn_log_dropdown.add_item("All", 2)
	spawn_log_dropdown.selected = UserSettings.spawn_log_mode
	spawn_log_dropdown.item_selected.connect(_on_spawn_log_selected)

	# Scroll speed slider
	scroll_slider.min_value = 100
	scroll_slider.max_value = 2000
	scroll_slider.step = 50
	scroll_slider.value = UserSettings.scroll_speed
	scroll_label.text = str(int(UserSettings.scroll_speed))
	scroll_slider.value_changed.connect(_on_scroll_changed)


func _unhandled_key_input(event: InputEvent) -> void:
	if event.keycode == KEY_ESCAPE and event.pressed:
		panel.visible = not panel.visible
		get_tree().paused = panel.visible
		get_viewport().set_input_as_handled()


func _on_hp_bars_selected(index: int) -> void:
	UserSettings.set_hp_bar_mode(index)


func _on_death_log_selected(index: int) -> void:
	UserSettings.set_death_log_mode(index)


func _on_level_log_selected(index: int) -> void:
	UserSettings.set_level_log_mode(index)


func _on_spawn_log_selected(index: int) -> void:
	UserSettings.set_spawn_log_mode(index)


func _on_scroll_changed(value: float) -> void:
	scroll_label.text = str(int(value))
	UserSettings.set_scroll_speed(value)
