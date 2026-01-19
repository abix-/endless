# npc_inspector.gd
# Shows detailed stats for selected NPC
extends CanvasLayer

@onready var panel: PanelContainer = $Panel
@onready var job_level: Label = $Panel/MarginContainer/VBox/JobLevel
@onready var town_label: Label = $Panel/MarginContainer/VBox/Town
@onready var health_bar: ProgressBar = $Panel/MarginContainer/VBox/HealthRow/Bar
@onready var health_value: Label = $Panel/MarginContainer/VBox/HealthRow/Value
@onready var energy_bar: ProgressBar = $Panel/MarginContainer/VBox/EnergyRow/Bar
@onready var energy_value: Label = $Panel/MarginContainer/VBox/EnergyRow/Value
@onready var xp_label: Label = $Panel/MarginContainer/VBox/XP
@onready var state_label: Label = $Panel/MarginContainer/VBox/State
@onready var target_label: Label = $Panel/MarginContainer/VBox/Target
@onready var attack_label: Label = $Panel/MarginContainer/VBox/Attack
@onready var range_label: Label = $Panel/MarginContainer/VBox/Range
@onready var position_label: Label = $Panel/MarginContainer/VBox/Position
@onready var extra_label: Label = $Panel/MarginContainer/VBox/Extra
@onready var pin_button: Button = $Panel/MarginContainer/VBox/ButtonRow/PinButton
@onready var copy_button: Button = $Panel/MarginContainer/VBox/ButtonRow/CopyButton

var npc_manager: Node
var main_node: Node
var pinned := false
var last_idx := -1


func _ready() -> void:
	await get_tree().process_frame
	npc_manager = get_tree().get_first_node_in_group("npc_manager")
	main_node = get_parent()

	pin_button.toggled.connect(_on_pin_toggled)
	copy_button.pressed.connect(_on_copy_pressed)


func _process(_delta: float) -> void:
	if not npc_manager:
		return

	if Engine.get_process_frames() % 10 != 0:
		return

	var idx: int = npc_manager.selected_npc

	# Hide if no selection and not pinned
	if idx < 0 and not pinned:
		panel.visible = false
		return

	# Use last known index if pinned but no current selection
	if idx < 0:
		idx = last_idx

	# Still no valid index
	if idx < 0 or idx >= npc_manager.count:
		panel.visible = false
		return

	# Check if NPC is alive
	if npc_manager.healths[idx] <= 0:
		if not pinned:
			panel.visible = false
		return

	last_idx = idx
	panel.visible = true
	_update_display(idx)


func _update_display(i: int) -> void:
	var job: int = npc_manager.jobs[i]
	var level: int = npc_manager.levels[i]
	var job_name: String = NPCState.JOB_NAMES[job] if job < NPCState.JOB_NAMES.size() else "NPC"

	job_level.text = "%s Lv.%d" % [job_name, level]

	# Town name
	var town_idx: int = npc_manager.town_indices[i]
	if town_idx >= 0 and main_node and "towns" in main_node and town_idx < main_node.towns.size():
		town_label.text = "Town: %s" % main_node.towns[town_idx].name
	else:
		town_label.text = "Town: -"

	# Health
	var hp: float = npc_manager.healths[i]
	var max_hp: float = npc_manager.get_scaled_max_health(i)
	health_bar.value = hp / max_hp if max_hp > 0 else 0
	health_value.text = "%d/%d" % [int(hp), int(max_hp)]

	# Energy
	var energy: float = npc_manager.energies[i]
	energy_bar.value = energy / Config.ENERGY_MAX
	energy_value.text = "%d/%d" % [int(energy), int(Config.ENERGY_MAX)]

	# XP
	var current_xp: int = npc_manager.xp[i]
	var next_level_xp: int = npc_manager.get_xp_for_next_level(level)
	xp_label.text = "XP: %d/%d" % [current_xp, next_level_xp]

	# State
	var state: int = npc_manager.states[i]
	var state_name: String = NPCState.STATE_NAMES.get(state, "Unknown")
	state_label.text = "State: %s" % state_name

	# Target
	var target_npc: int = npc_manager.current_targets[i]
	if target_npc >= 0 and target_npc < npc_manager.count and npc_manager.healths[target_npc] > 0:
		var target_job: int = npc_manager.jobs[target_npc]
		var target_job_name: String = NPCState.JOB_NAMES[target_job] if target_job < NPCState.JOB_NAMES.size() else "NPC"
		target_label.text = "Target: %s #%d" % [target_job_name, target_npc]
	else:
		target_label.text = "Target: None"

	# Attack and Range
	var scaled_dmg: float = npc_manager.get_scaled_damage(i)
	var attack_range: float = npc_manager.attack_ranges[i]
	attack_label.text = "Attack: %.1f" % scaled_dmg
	range_label.text = "Range: %.0f" % attack_range

	# Position
	var pos: Vector2 = npc_manager.positions[i]
	position_label.text = "Pos: %d, %d" % [int(pos.x), int(pos.y)]

	# Extra info based on job
	var extra_lines: Array[String] = []
	if job == NPCState.Job.GUARD:
		var night: int = npc_manager.works_at_night[i]
		extra_lines.append("Night shift: %s" % ("Yes" if night == 1 else "No"))
		var patrol_time: int = npc_manager.patrol_timer[i]
		extra_lines.append("Patrol timer: %d" % patrol_time)
	elif job == NPCState.Job.RAIDER:
		var carrying: int = npc_manager.carrying_food[i]
		extra_lines.append("Carrying food: %s" % ("Yes" if carrying == 1 else "No"))

	if npc_manager.recovering[i] == 1:
		extra_lines.append("Recovering: Yes")

	extra_label.text = "\n".join(extra_lines)


func _on_pin_toggled(toggled_on: bool) -> void:
	pinned = toggled_on


func _on_copy_pressed() -> void:
	if last_idx < 0 or last_idx >= npc_manager.count:
		return

	var data: String = _format_npc_data(last_idx)
	DisplayServer.clipboard_set(data)


func _format_npc_data(i: int) -> String:
	var lines: Array[String] = []
	lines.append("NPC Inspector Export")
	lines.append("====================")

	# Basic info
	var job: int = npc_manager.jobs[i]
	var job_name: String = NPCState.JOB_NAMES[job] if job < NPCState.JOB_NAMES.size() else "NPC"
	lines.append("Index: %d" % i)
	lines.append("Job: %s (%d)" % [job_name, job])
	lines.append("Level: %d" % npc_manager.levels[i])

	var town_idx: int = npc_manager.town_indices[i]
	var town_name := "-"
	if town_idx >= 0 and main_node and "towns" in main_node and town_idx < main_node.towns.size():
		town_name = main_node.towns[town_idx].name
	lines.append("Town: %s (idx: %d)" % [town_name, town_idx])
	lines.append("Faction: %s (%d)" % ["Villager" if npc_manager.factions[i] == 0 else "Raider", npc_manager.factions[i]])
	lines.append("")

	# Health/Energy/XP
	var hp: float = npc_manager.healths[i]
	var base_max_hp: float = npc_manager.max_healths[i]
	var scaled_max_hp: float = npc_manager.get_scaled_max_health(i)
	lines.append("Health: %.1f / %.1f (base) / %.1f (scaled max)" % [hp, base_max_hp, scaled_max_hp])
	lines.append("Energy: %.1f / %.1f" % [npc_manager.energies[i], Config.ENERGY_MAX])
	lines.append("XP: %d" % npc_manager.xp[i])
	lines.append("")

	# State
	var state: int = npc_manager.states[i]
	var state_name: String = NPCState.STATE_NAMES.get(state, "Unknown")
	lines.append("State: %s (%d)" % [state_name, state])
	lines.append("Target NPC: %d" % npc_manager.current_targets[i])
	lines.append("Recovering: %d" % npc_manager.recovering[i])
	lines.append("")

	# Stats
	lines.append("Stats:")
	lines.append("  attack_damage: %.1f (base) / %.1f (scaled)" % [npc_manager.attack_damages[i], npc_manager.get_scaled_damage(i)])
	lines.append("  attack_range: %.1f" % npc_manager.attack_ranges[i])
	lines.append("  size_bonus: %.2f" % npc_manager.size_bonuses[i])
	lines.append("")

	# Positions
	var pos: Vector2 = npc_manager.positions[i]
	var vel: Vector2 = npc_manager.velocities[i]
	var target_pos: Vector2 = npc_manager.targets[i]
	var home: Vector2 = npc_manager.home_positions[i]
	var work: Vector2 = npc_manager.work_positions[i]
	var wander: Vector2 = npc_manager.wander_centers[i]
	lines.append("Position: (%.1f, %.1f)" % [pos.x, pos.y])
	lines.append("Velocity: (%.1f, %.1f)" % [vel.x, vel.y])
	lines.append("Target Pos: (%.1f, %.1f)" % [target_pos.x, target_pos.y])
	lines.append("Home: (%.1f, %.1f)" % [home.x, home.y])
	lines.append("Work: (%.1f, %.1f)" % [work.x, work.y])
	lines.append("Wander Center: (%.1f, %.1f)" % [wander.x, wander.y])
	lines.append("")

	# Flags
	lines.append("Flags:")
	lines.append("  will_flee: %d" % npc_manager.will_flee[i])
	lines.append("  works_at_night: %d" % npc_manager.works_at_night[i])
	lines.append("  carrying_food: %d" % npc_manager.carrying_food[i])
	lines.append("")

	# Timers
	lines.append("Timers:")
	lines.append("  attack_timer: %.2f" % npc_manager.attack_timers[i])
	lines.append("  scan_timer: %.2f" % npc_manager.scan_timers[i])
	lines.append("  patrol_timer: %d" % npc_manager.patrol_timer[i])
	lines.append("  patrol_target_idx: %d" % npc_manager.patrol_target_idx[i])
	lines.append("  patrol_last_idx: %d" % npc_manager.patrol_last_idx[i])

	return "\n".join(lines)
