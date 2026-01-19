# settings_menu.gd
# In-game settings menu
extends CanvasLayer

@onready var panel: PanelContainer = $Panel
@onready var hp_bars_checkbox: CheckBox = $Panel/MarginContainer/VBox/HpBarsCheck

func _ready() -> void:
	panel.visible = false
	hp_bars_checkbox.button_pressed = UserSettings.show_hp_bars_always
	hp_bars_checkbox.toggled.connect(_on_hp_bars_toggled)


func _unhandled_key_input(event: InputEvent) -> void:
	if event.keycode == KEY_ESCAPE and event.pressed:
		panel.visible = not panel.visible
		get_tree().paused = panel.visible
		get_viewport().set_input_as_handled()


func _on_hp_bars_toggled(pressed: bool) -> void:
	UserSettings.set_show_hp_bars_always(pressed)
