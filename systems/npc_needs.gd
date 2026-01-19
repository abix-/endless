# npc_needs.gd
# Handles energy, hunger, time-based state changes, and goal-based raider AI
extends RefCounted
class_name NPCNeeds

var manager: Node

func _init(npc_manager: Node) -> void:
	manager = npc_manager

func on_time_tick(hour: int, minute: int) -> void:
	# Every 15 minutes - reconsider decisions
	if minute % 15 == 0:
		for i in manager.count:
			if manager.healths[i] <= 0:
				continue
			var state: int = manager.states[i]
			if state not in [NPCState.State.FIGHTING, NPCState.State.FLEEING]:
				manager._decide_what_to_do(i)
	
	# On the hour - update energy
	if minute != 0:
		return
	
	for i in manager.count:
		if manager.healths[i] <= 0:
			continue
		
		var state: int = manager.states[i]
		match state:
			NPCState.State.SLEEPING:
				manager.energies[i] = minf(Config.ENERGY_MAX, manager.energies[i] + Config.ENERGY_SLEEP_GAIN)
			NPCState.State.RESTING:
				manager.energies[i] = minf(Config.ENERGY_MAX, manager.energies[i] + Config.ENERGY_REST_GAIN)
			_:
				manager.energies[i] = maxf(0.0, manager.energies[i] - Config.ENERGY_ACTIVITY_DRAIN)

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
	if energy <= Config.ENERGY_EXHAUSTED:
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
	var health_pct: float = manager.healths[i] / manager.max_healths[i]
	var energy: float = manager.energies[i]

	# Priority 1: Wounded - retreat to camp
	if health_pct < Config.RAIDER_WOUNDED_THRESHOLD:
		manager.carrying_food[i] = 0  # Drop food when fleeing wounded
		_raider_return_to_camp(i)
		return

	# Priority 2: Exhausted - go home to sleep
	if energy <= Config.ENERGY_EXHAUSTED:
		var state: int = manager.states[i]
		if state != NPCState.State.SLEEPING:
			manager.targets[i] = manager.home_positions[i]
			manager._state.set_state(i, NPCState.State.WALKING)
		return

	# Priority 3: Carrying food - return to camp
	if manager.carrying_food[i] == 1:
		_raider_return_to_camp(i)
		return

	# Priority 4: Go steal food from farms
	_raider_go_to_farm(i)


func _raider_return_to_camp(i: int) -> void:
	manager.targets[i] = manager.home_positions[i]
	manager.wander_centers[i] = manager.positions[i]
	manager._state.set_state(i, NPCState.State.WALKING)


func _raider_go_to_farm(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]

	if manager.farm_positions.size() == 0:
		return

	# Pick a random farm (not always nearest - spreads raiders out)
	var farm_idx: int = randi() % manager.farm_positions.size()
	var farm_pos: Vector2 = manager.farm_positions[farm_idx]

	# Add offset so raiders spread across the farm
	var offset := Vector2(randf_range(-40, 40), randf_range(-40, 40))
	manager.targets[i] = farm_pos + offset
	manager.wander_centers[i] = my_pos
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
	var job: int = manager.jobs[i]

	if state == NPCState.State.WANDERING:
		# Raiders check if at farm to steal food
		if job == NPCState.Job.RAIDER:
			manager.wander_centers[i] = manager.positions[i]
			_raider_check_steal_food(i)
		manager._state.set_state(i, NPCState.State.IDLE)
		decide_what_to_do(i)
	elif state == NPCState.State.WALKING:
		var target: Vector2 = manager.targets[i]
		var work_pos: Vector2 = manager.work_positions[i]
		var home_pos: Vector2 = manager.home_positions[i]
		var energy: float = manager.energies[i]

		if target.distance_to(work_pos) < 10:
			manager._state.set_state(i, NPCState.State.WORKING)
			if job == NPCState.Job.GUARD:
				manager.wander_centers[i] = manager.positions[i]
		elif target.distance_to(home_pos) < 10:
			if job == NPCState.Job.RAIDER:
				manager.wander_centers[i] = manager.positions[i]
				_raider_deliver_food(i)
			if energy <= Config.ENERGY_EXHAUSTED:
				manager._state.set_state(i, NPCState.State.SLEEPING)
			else:
				manager._state.set_state(i, NPCState.State.RESTING)
				decide_what_to_do(i)  # Raiders go back out after brief rest


func _raider_check_steal_food(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]

	for farm_pos in manager.farm_positions:
		if my_pos.distance_to(farm_pos) < 60.0:
			manager.carrying_food[i] = 1
			return


func _raider_deliver_food(i: int) -> void:
	if manager.carrying_food[i] == 1:
		manager.carrying_food[i] = 0
		# Restore some energy from successful raid
		manager.energies[i] = minf(Config.ENERGY_MAX, manager.energies[i] + Config.ENERGY_FARM_RESTORE)
