# npc_needs.gd
# Handles energy, hunger, time-based state changes
extends RefCounted
class_name NPCNeeds

var manager: Node

func _init(npc_manager: Node) -> void:
	manager = npc_manager

func on_time_tick(hour: int, minute: int) -> void:
	# Every 15 minutes - reconsider decisions
	if minute != 0 and minute % 15 == 0:
		for i in manager.count:
			if manager.healths[i] <= 0:
				continue
			var state: int = manager.states[i]
			if state not in [NPCState.State.FIGHTING, NPCState.State.FLEEING]:
				manager._decide_what_to_do(i)
		return
	
	# On the hour - update energy
	if minute != 0:
		return
	
	for i in manager.count:
		if manager.healths[i] <= 0:
			continue
		
		var state: int = manager.states[i]
		match state:
			NPCState.State.SLEEPING:
				manager.energies[i] = minf(100.0, manager.energies[i] + 12.0)
			NPCState.State.RESTING:
				manager.energies[i] = minf(100.0, manager.energies[i] + 2.0)
			_:
				manager.energies[i] = maxf(0.0, manager.energies[i] - 6.0)

func decide_what_to_do(i: int) -> void:
	if manager.healths[i] <= 0:
		return
	
	var job: int = manager.jobs[i]
	if job == NPCState.Job.RAIDER:
		_decide_raider(i)
		return
	
	var energy: float = manager.energies[i]
	var state: int = manager.states[i]
	
	# Low energy - go home to sleep
	if energy <= 20.0:
		if state != NPCState.State.SLEEPING:
			manager.targets[i] = manager.home_positions[i]
			manager._state.set_state(i, NPCState.State.WALKING)
		return
	
	var is_work_time: bool = _is_work_time(i)
	
	if is_work_time:
		if state not in [NPCState.State.WORKING, NPCState.State.WALKING]:
			manager.targets[i] = manager.work_positions[i]
			manager._state.set_state(i, NPCState.State.WALKING)
	else:
		if state not in [NPCState.State.RESTING, NPCState.State.WALKING]:
			manager.targets[i] = manager.home_positions[i]
			manager._state.set_state(i, NPCState.State.WALKING)

func _decide_raider(i: int) -> void:
	var energy: float = manager.energies[i]
	var state: int = manager.states[i]
	
	if energy <= 20.0:
		manager._state.set_state(i, NPCState.State.RESTING)
		return
	
	if state != NPCState.State.WANDERING:
		var angle: float = randf() * TAU
		var dist: float = randf_range(150.0, 400.0)  # Longer wander range
		var center: Vector2 = manager.wander_centers[i]
		manager.targets[i] = center + Vector2(cos(angle), sin(angle)) * dist
		manager._state.set_state(i, NPCState.State.WANDERING)

func _is_work_time(i: int) -> bool:
	var is_day: bool = WorldClock.is_daytime()
	var works_night: int = manager.works_at_night[i]
	if works_night == 1:
		return not is_day
	else:
		return is_day

func on_arrival(i: int) -> void:
	var state: int = manager.states[i]
	
	if state == NPCState.State.WANDERING:
		manager._state.set_state(i, NPCState.State.IDLE)
		decide_what_to_do(i)
	elif state == NPCState.State.WALKING:
		var target: Vector2 = manager.targets[i]
		var work_pos: Vector2 = manager.work_positions[i]
		var home_pos: Vector2 = manager.home_positions[i]
		var energy: float = manager.energies[i]
		
		if target.distance_to(work_pos) < 10:
			manager._state.set_state(i, NPCState.State.WORKING)
			# Guards update wander center to work position
			if manager.jobs[i] == NPCState.Job.GUARD:
				manager.wander_centers[i] = manager.positions[i]
		elif target.distance_to(home_pos) < 10:
			if energy <= 20:
				manager._state.set_state(i, NPCState.State.SLEEPING)
			else:
				manager._state.set_state(i, NPCState.State.RESTING)