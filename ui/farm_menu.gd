# farm_menu.gd
# Simple popup showing farm occupancy when clicked
extends CanvasLayer

var panel: PanelContainer
var title_label: Label
var farmer_label: Label

var npc_manager: Node
var current_town_idx: int = -1
var current_farm_idx: int = -1


func _ready() -> void:
	await get_tree().process_frame
	npc_manager = get_parent().npc_manager

	panel = PanelContainer.new()
	panel.custom_minimum_size = Vector2(160, 0)
	panel.visible = false
	add_child(panel)

	var vbox := VBoxContainer.new()
	vbox.add_theme_constant_override("separation", 4)
	panel.add_child(vbox)

	var margin := MarginContainer.new()
	margin.add_theme_constant_override("margin_left", 8)
	margin.add_theme_constant_override("margin_right", 8)
	margin.add_theme_constant_override("margin_top", 6)
	margin.add_theme_constant_override("margin_bottom", 6)
	panel.add_child(margin)

	var inner_vbox := VBoxContainer.new()
	inner_vbox.add_theme_constant_override("separation", 4)
	margin.add_child(inner_vbox)

	# Remove the first vbox since we're using margin container
	panel.remove_child(vbox)
	vbox.queue_free()

	title_label = Label.new()
	title_label.horizontal_alignment = HORIZONTAL_ALIGNMENT_CENTER
	inner_vbox.add_child(title_label)

	farmer_label = Label.new()
	inner_vbox.add_child(farmer_label)


func _input(event: InputEvent) -> void:
	if not panel.visible:
		return

	if event is InputEventKey and event.pressed and event.keycode == KEY_ESCAPE:
		close()
		get_viewport().set_input_as_handled()

	if event is InputEventMouseButton and event.pressed and event.button_index == MOUSE_BUTTON_LEFT:
		var rect := Rect2(panel.global_position, panel.size)
		if not rect.has_point(event.position):
			close()


func _process(_delta: float) -> void:
	if panel.visible:
		_refresh()


func open(town_idx: int, farm_idx: int, screen_pos: Vector2) -> void:
	current_town_idx = town_idx
	current_farm_idx = farm_idx

	title_label.text = "Farm %d" % (farm_idx + 1)
	_refresh()

	var viewport_size: Vector2 = get_viewport().get_visible_rect().size
	panel.position = Vector2(
		clampf(screen_pos.x - 80, 0, viewport_size.x - 170),
		clampf(screen_pos.y + 10, 0, viewport_size.y - 80)
	)
	panel.visible = true


func close() -> void:
	panel.visible = false


func _refresh() -> void:
	if current_town_idx < 0 or current_farm_idx < 0:
		return

	var count: int = npc_manager.farm_occupant_counts[current_town_idx][current_farm_idx]
	if count == 0:
		farmer_label.text = "Farmer: none"
	else:
		# Find which NPC is working this farm
		var farmer_name := ""
		for i in npc_manager.count:
			if npc_manager.healths[i] <= 0:
				continue
			if npc_manager.town_indices[i] != current_town_idx:
				continue
			if npc_manager.current_farm_idx[i] == current_farm_idx:
				farmer_name = npc_manager.npc_names[i]
				break
		if farmer_name != "":
			farmer_label.text = "Farmer: %s" % farmer_name
		else:
			farmer_label.text = "Farmer: %d/1" % count
