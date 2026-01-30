# roster_panel.gd
# Shows all guards and farmers in player's faction
# ECS-only: per-NPC iteration not yet available from ECS
extends CanvasLayer

@onready var panel: PanelContainer = $Panel
@onready var close_btn: Button = $Panel/VBox/Header/CloseBtn
@onready var all_btn: Button = $Panel/VBox/FilterRow/AllBtn
@onready var farmers_btn: Button = $Panel/VBox/FilterRow/FarmersBtn
@onready var guards_btn: Button = $Panel/VBox/FilterRow/GuardsBtn
@onready var list: VBoxContainer = $Panel/VBox/Scroll/List
@onready var count_label: Label = $Panel/VBox/CountLabel

# Header buttons for sorting
@onready var name_btn: Button = $Panel/VBox/HeaderRow/Name
@onready var job_btn: Button = $Panel/VBox/HeaderRow/Job
@onready var level_btn: Button = $Panel/VBox/HeaderRow/Level
@onready var hp_btn: Button = $Panel/VBox/HeaderRow/HP
@onready var state_btn: Button = $Panel/VBox/HeaderRow/State
@onready var trait_btn: Button = $Panel/VBox/HeaderRow/Trait

var npc_manager: Node
var main_node: Node

enum Filter { ALL, FARMERS, GUARDS }
enum SortBy { NAME, JOB, LEVEL, HP, STATE, TRAIT }

var current_filter := Filter.ALL
var current_sort := SortBy.LEVEL
var sort_descending := true

# Cache for row nodes
var row_pool: Array[HBoxContainer] = []
var active_rows := 0


func _ready() -> void:
	add_to_group("roster_panel")
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	main_node = get_parent()

	close_btn.pressed.connect(close)
	all_btn.pressed.connect(_on_filter_all)
	farmers_btn.pressed.connect(_on_filter_farmers)
	guards_btn.pressed.connect(_on_filter_guards)

	name_btn.pressed.connect(_on_sort.bind(SortBy.NAME))
	job_btn.pressed.connect(_on_sort.bind(SortBy.JOB))
	level_btn.pressed.connect(_on_sort.bind(SortBy.LEVEL))
	hp_btn.pressed.connect(_on_sort.bind(SortBy.HP))
	state_btn.pressed.connect(_on_sort.bind(SortBy.STATE))
	trait_btn.pressed.connect(_on_sort.bind(SortBy.TRAIT))


func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed:
		if event.keycode == KEY_R and not event.is_echo():
			toggle()
		elif event.keycode == KEY_ESCAPE and panel.visible:
			close()
			get_viewport().set_input_as_handled()

	# Block scroll wheel when hovering over panel
	if panel.visible and event is InputEventMouseButton:
		if event.button_index == MOUSE_BUTTON_WHEEL_UP or event.button_index == MOUSE_BUTTON_WHEEL_DOWN:
			var rect := Rect2(panel.global_position, panel.size)
			if rect.has_point(event.position):
				get_viewport().set_input_as_handled()


func toggle() -> void:
	if panel.visible:
		close()
	else:
		open()


func open() -> void:
	panel.visible = true
	_refresh()


func close() -> void:
	panel.visible = false


func _process(_delta: float) -> void:
	if not panel.visible:
		return
	if Engine.get_process_frames() % 30 == 0:
		_refresh()


func _on_filter_all() -> void:
	current_filter = Filter.ALL
	_update_filter_buttons()
	_refresh()


func _on_filter_farmers() -> void:
	current_filter = Filter.FARMERS
	_update_filter_buttons()
	_refresh()


func _on_filter_guards() -> void:
	current_filter = Filter.GUARDS
	_update_filter_buttons()
	_refresh()


func _update_filter_buttons() -> void:
	all_btn.button_pressed = current_filter == Filter.ALL
	farmers_btn.button_pressed = current_filter == Filter.FARMERS
	guards_btn.button_pressed = current_filter == Filter.GUARDS


func _on_sort(sort_by: SortBy) -> void:
	if current_sort == sort_by:
		sort_descending = not sort_descending
	else:
		current_sort = sort_by
		sort_descending = true
	_update_sort_headers()
	_refresh()


func _update_sort_headers() -> void:
	var arrow := " ▼" if sort_descending else " ▲"
	name_btn.text = "Name" + (arrow if current_sort == SortBy.NAME else "")
	job_btn.text = "Job" + (arrow if current_sort == SortBy.JOB else "")
	level_btn.text = "Lv" + (arrow if current_sort == SortBy.LEVEL else "")
	hp_btn.text = "HP" + (arrow if current_sort == SortBy.HP else "")
	state_btn.text = "State" + (arrow if current_sort == SortBy.STATE else "")
	trait_btn.text = "Trait" + (arrow if current_sort == SortBy.TRAIT else "")


func _refresh() -> void:
	if not npc_manager or not main_node:
		return

	# Get player's town index
	var player_town: int = 0
	if "player_town_idx" in main_node:
		player_town = main_node.player_town_idx

	# Get filter job type (-1 = all, 0 = farmer, 1 = guard)
	var filter_job: int = -1
	match current_filter:
		Filter.FARMERS:
			filter_job = 0
		Filter.GUARDS:
			filter_job = 1

	# Get NPCs from ECS
	var npcs: Array = npc_manager.get_npcs_by_town(player_town, filter_job)

	# Sort NPCs
	npcs.sort_custom(_compare_npcs)

	# Update count label
	count_label.text = "%d NPCs" % npcs.size()

	# Ensure we have enough rows
	while row_pool.size() < npcs.size():
		row_pool.append(_create_row())

	# Update visible rows
	for i in npcs.size():
		var row: HBoxContainer = row_pool[i]
		row.visible = true
		_update_row(row, npcs[i])

	# Hide unused rows
	for i in range(npcs.size(), row_pool.size()):
		row_pool[i].visible = false


func _compare_npcs(a: Dictionary, b: Dictionary) -> bool:
	var result: bool
	match current_sort:
		SortBy.NAME:
			result = a.name < b.name
		SortBy.JOB:
			result = a.job < b.job
		SortBy.LEVEL:
			result = a.level < b.level
		SortBy.HP:
			result = a.hp < b.hp
		SortBy.STATE:
			result = a.state < b.state
		SortBy.TRAIT:
			result = a.trait < b.trait
		_:
			result = a.level < b.level

	return not result if sort_descending else result


func _create_row() -> HBoxContainer:
	var row := HBoxContainer.new()
	row.add_theme_constant_override("separation", 4)

	var name_label := Label.new()
	name_label.custom_minimum_size.x = 140
	name_label.clip_text = true
	row.add_child(name_label)

	var job_label := Label.new()
	job_label.custom_minimum_size.x = 60
	row.add_child(job_label)

	var level_label := Label.new()
	level_label.custom_minimum_size.x = 50
	row.add_child(level_label)

	var hp_label := Label.new()
	hp_label.custom_minimum_size.x = 70
	row.add_child(hp_label)

	var state_label := Label.new()
	state_label.custom_minimum_size.x = 80
	state_label.clip_text = true
	row.add_child(state_label)

	var trait_label := Label.new()
	trait_label.custom_minimum_size.x = 70
	trait_label.clip_text = true
	row.add_child(trait_label)

	var select_btn := Button.new()
	select_btn.text = "◎"
	select_btn.tooltip_text = "Select"
	select_btn.custom_minimum_size.x = 30
	row.add_child(select_btn)

	var follow_btn := Button.new()
	follow_btn.text = "▶"
	follow_btn.tooltip_text = "Follow"
	follow_btn.custom_minimum_size.x = 30
	row.add_child(follow_btn)

	list.add_child(row)
	return row


func _update_row(row: HBoxContainer, npc: Dictionary) -> void:
	var children := row.get_children()
	var idx: int = npc.idx

	children[0].text = npc.name
	children[1].text = npc.job if npc.job else "?"
	children[2].text = str(npc.level)
	children[3].text = "%d/%d" % [int(npc.hp), int(npc.max_hp)]
	children[4].text = npc.state if npc.state else "?"
	children[5].text = npc.trait if npc.trait else ""

	# Reconnect buttons
	var select_btn: Button = children[6]
	var follow_btn_node: Button = children[7]

	for conn in select_btn.pressed.get_connections():
		select_btn.pressed.disconnect(conn.callable)
	for conn in follow_btn_node.pressed.get_connections():
		follow_btn_node.pressed.disconnect(conn.callable)

	select_btn.pressed.connect(_on_select.bind(idx))
	follow_btn_node.pressed.connect(_on_follow.bind(idx))


func _on_select(idx: int) -> void:
	npc_manager.set_selected_npc(idx)
	var left_panel = get_tree().get_first_node_in_group("left_panel")
	if left_panel:
		left_panel.last_idx = idx


func _on_follow(idx: int) -> void:
	npc_manager.set_selected_npc(idx)
	var left_panel = get_tree().get_first_node_in_group("left_panel")
	if left_panel:
		left_panel.last_idx = idx
		left_panel._set_following(true)
