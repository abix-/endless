# start_menu.gd
# Game start menu with world configuration
extends CanvasLayer

@onready var world_size_slider: HSlider = $Panel/MarginContainer/VBox/WorldSizeRow/Slider
@onready var world_size_value: Label = $Panel/MarginContainer/VBox/WorldSizeRow/Value
@onready var towns_slider: HSlider = $Panel/MarginContainer/VBox/TownsRow/Slider
@onready var towns_value: Label = $Panel/MarginContainer/VBox/TownsRow/Value
@onready var farmers_slider: HSlider = $Panel/MarginContainer/VBox/FarmersRow/Slider
@onready var farmers_value: Label = $Panel/MarginContainer/VBox/FarmersRow/Value
@onready var guards_slider: HSlider = $Panel/MarginContainer/VBox/GuardsRow/Slider
@onready var guards_value: Label = $Panel/MarginContainer/VBox/GuardsRow/Value
@onready var raiders_slider: HSlider = $Panel/MarginContainer/VBox/RaidersRow/Slider
@onready var raiders_value: Label = $Panel/MarginContainer/VBox/RaidersRow/Value
@onready var start_button: Button = $Panel/MarginContainer/VBox/StartButton

const WORLD_SIZE_MIN := 500
const WORLD_SIZE_MAX := 32000
const WORLD_SIZE_STEP := 500
const WORLD_SIZE_DEFAULT := 8000

const TOWNS_MIN := 1
const TOWNS_MAX := 7
const TOWNS_DEFAULT := 1

const FARMERS_MIN := 0
const FARMERS_MAX := 50
const FARMERS_DEFAULT := 2  # per villager town

const GUARDS_MIN := 0
const GUARDS_MAX := 50
const GUARDS_DEFAULT := 4  # per villager town

const RAIDERS_MIN := 0
const RAIDERS_MAX := 50
const RAIDERS_DEFAULT := 6  # per raider town


func _ready() -> void:
	world_size_slider.min_value = WORLD_SIZE_MIN
	world_size_slider.max_value = WORLD_SIZE_MAX
	world_size_slider.step = WORLD_SIZE_STEP
	world_size_slider.value = WORLD_SIZE_DEFAULT
	world_size_slider.value_changed.connect(_on_world_size_changed)

	towns_slider.min_value = TOWNS_MIN
	towns_slider.max_value = TOWNS_MAX
	towns_slider.step = 1
	towns_slider.value = TOWNS_DEFAULT
	towns_slider.value_changed.connect(_on_towns_changed)

	farmers_slider.min_value = FARMERS_MIN
	farmers_slider.max_value = FARMERS_MAX
	farmers_slider.step = 1
	farmers_slider.value = FARMERS_DEFAULT
	farmers_slider.value_changed.connect(_on_farmers_changed)

	guards_slider.min_value = GUARDS_MIN
	guards_slider.max_value = GUARDS_MAX
	guards_slider.step = 1
	guards_slider.value = GUARDS_DEFAULT
	guards_slider.value_changed.connect(_on_guards_changed)

	raiders_slider.min_value = RAIDERS_MIN
	raiders_slider.max_value = RAIDERS_MAX
	raiders_slider.step = 1
	raiders_slider.value = RAIDERS_DEFAULT
	raiders_slider.value_changed.connect(_on_raiders_changed)

	start_button.pressed.connect(_on_start_pressed)
	_update_world_size_label()
	_update_towns_label()
	_update_farmers_label()
	_update_guards_label()
	_update_raiders_label()


func _update_world_size_label() -> void:
	var size := int(world_size_slider.value)
	@warning_ignore("integer_division")
	var tiles_per_side := size / Config.TILE_SIZE
	var size_name := _get_size_name(size)
	world_size_value.text = "%s (%dx%d)" % [size_name, tiles_per_side, tiles_per_side]


func _get_size_name(size: int) -> String:
	match size:
		4000: return "Tiny"
		8000: return "Small"
		12000: return "Medium"
		16000: return "Large"
		20000: return "Huge"
		24000: return "Massive"
		28000: return "Epic"
		32000: return "Endless"
		_: return "Custom"


func _on_world_size_changed(_value: float) -> void:
	_update_world_size_label()


func _on_towns_changed(_value: float) -> void:
	_update_towns_label()


func _update_towns_label() -> void:
	var towns := int(towns_slider.value)
	towns_value.text = "%d town%s" % [towns, "s" if towns > 1 else ""]


func _on_farmers_changed(_value: float) -> void:
	_update_farmers_label()


func _on_guards_changed(_value: float) -> void:
	_update_guards_label()


func _on_raiders_changed(_value: float) -> void:
	_update_raiders_label()


func _update_farmers_label() -> void:
	farmers_value.text = "%d per town" % int(farmers_slider.value)


func _update_guards_label() -> void:
	guards_value.text = "%d per town" % int(guards_slider.value)


func _update_raiders_label() -> void:
	raiders_value.text = "%d per camp" % int(raiders_slider.value)


func _on_start_pressed() -> void:
	var size := int(world_size_slider.value)
	Config.world_width = size
	Config.world_height = size
	Config.num_towns = int(towns_slider.value)

	# Values are per-town, not totals
	Config.farmers_per_town = int(farmers_slider.value)
	Config.guards_per_town = int(guards_slider.value)
	Config.raiders_per_camp = int(raiders_slider.value)
	Config.max_farmers_per_town = Config.farmers_per_town
	Config.max_guards_per_town = Config.guards_per_town

	get_tree().change_scene_to_file("res://main.tscn")
