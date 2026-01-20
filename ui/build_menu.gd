# build_menu.gd
# Popup menu for building in town grid slots
extends CanvasLayer

signal build_requested(slot_key: String, building_type: String)

@onready var panel: PanelContainer = $Panel
@onready var title_label: Label = $Panel/VBox/Title
@onready var farm_btn: Button = $Panel/VBox/FarmBtn
@onready var bed_btn: Button = $Panel/VBox/BedBtn
@onready var close_btn: Button = $Panel/VBox/CloseBtn

const FARM_COST := 50
const BED_COST := 10
const MAX_BEDS_PER_SLOT := 4

var main_node: Node
var current_slot_key: String = ""
var current_town_idx: int = -1


func _ready() -> void:
	await get_tree().process_frame
	main_node = get_parent()

	farm_btn.pressed.connect(_on_farm_pressed)
	bed_btn.pressed.connect(_on_bed_pressed)
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


func open(slot_key: String, town_idx: int, screen_pos: Vector2) -> void:
	current_slot_key = slot_key
	current_town_idx = town_idx

	title_label.text = "Build in %s" % slot_key.to_upper()

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
	var town: Dictionary = main_node.towns[current_town_idx]
	var slot_contents: Array = town.slots[current_slot_key]

	# Count beds and farms in slot
	var bed_count := 0
	var has_farm := false
	for building in slot_contents:
		if building.type == "bed":
			bed_count += 1
		elif building.type == "farm":
			has_farm = true

	# Farm button - only if slot is empty (no beds or farms)
	farm_btn.text = "Farm (%d food)" % FARM_COST
	farm_btn.disabled = has_farm or bed_count > 0 or food < FARM_COST

	# Bed button - only if no farm and under 4 beds
	bed_btn.text = "Bed (%d food) [%d/4]" % [BED_COST, bed_count]
	bed_btn.disabled = has_farm or bed_count >= MAX_BEDS_PER_SLOT or food < BED_COST


func _on_farm_pressed() -> void:
	if _try_build("farm", FARM_COST):
		close()


func _on_bed_pressed() -> void:
	if _try_build("bed", BED_COST):
		_refresh_buttons()  # Stay open to build more beds


func _try_build(building_type: String, cost: int) -> bool:
	if not main_node or current_town_idx < 0:
		return false

	if main_node.town_food[current_town_idx] < cost:
		return false

	main_node.town_food[current_town_idx] -= cost
	build_requested.emit(current_slot_key, building_type)
	return true
