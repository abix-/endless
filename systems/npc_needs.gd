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
	var my_pos: Vector2 = manager.positions[i]
	var health_pct: float = manager.healths[i] / manager.max_healths[i]
	var energy: float = manager.energies[i]
	var nearby_allies: int = manager.count_nearby_raiders(i, Config.RAIDER_GROUP_RADIUS)
	
	# Priority 1: Wounded - retreat away from village
	if health_pct < Config.RAIDER_WOUNDED_THRESHOLD:
		_raider_retreat(i)
		return
	
	# Priority 2: Exhausted - rest
	if energy <= Config.ENERGY_EXHAUSTED:
		manager._state.set_state(i, NPCState.State.RESTING)
		return
	
	# Priority 3: Hungry - find food at farm
	if energy < Config.RAIDER_HUNGRY_THRESHOLD:
		_raider_seek_food(i)
		return
	
	# Priority 4: Alone - find other raiders
	if nearby_allies == 0:
		_raider_seek_allies(i)
		return
	
	# Priority 5: Confident (3+ allies) - raid the village
	if nearby_allies >= Config.RAIDER_CONFIDENCE_THRESHOLD:
		_raider_raid_village(i)
		return
	
	# Priority 6: Small group - wander toward village with cohesion
	_raider_wander_with_cohesion(i)


func _raider_retreat(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var village_center: Vector2 = manager.village_center
	
	# Move away from village
	var retreat_dir: Vector2 = my_pos.direction_to(village_center) * -1
	var retreat_target: Vector2 = my_pos + retreat_dir * Config.RAIDER_RETREAT_DIST
	
	manager.targets[i] = retreat_target
	manager.wander_centers[i] = retreat_target
	manager._state.set_state(i, NPCState.State.WANDERING)


func _raider_seek_food(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	
	if manager.farm_positions.size() == 0:
		_raider_wander_with_cohesion(i)
		return
	
	var nearest_farm: Vector2 = manager.find_nearest_farm(my_pos)
	
	# Add some randomness so not all raiders go to same farm
	var offset := Vector2(randf_range(-50, 50), randf_range(-50, 50))
	manager.targets[i] = nearest_farm + offset
	manager._state.set_state(i, NPCState.State.WANDERING)


func _raider_seek_allies(i: int) -> void:
	var nearest_raider: int = manager.find_nearest_raider(i)
	
	if nearest_raider >= 0:
		# Move toward nearest raider
		var target_pos: Vector2 = manager.positions[nearest_raider]
		var my_pos: Vector2 = manager.positions[i]
		
		# Don't go exactly to them, stop nearby
		var dir: Vector2 = my_pos.direction_to(target_pos)
		var dist: float = my_pos.distance_to(target_pos)
		var move_dist: float = maxf(0, dist - 50.0)  # Stop 50px away
		
		manager.targets[i] = my_pos + dir * move_dist
		manager._state.set_state(i, NPCState.State.WANDERING)
	else:
		# No other raiders alive, wander toward village
		_raider_wander_toward_village(i)


func _raider_raid_village(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var village_center: Vector2 = manager.village_center
	
	# Move toward village center with some spread
	var offset := Vector2(randf_range(-100, 100), randf_range(-100, 100))
	manager.targets[i] = village_center + offset
	manager.wander_centers[i] = my_pos  # Update center so we don't leash back
	manager._state.set_state(i, NPCState.State.WANDERING)


func _raider_wander_with_cohesion(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var village_center: Vector2 = manager.village_center
	
	# Get group center (cohesion)
	var group_center: Vector2 = manager.get_raider_group_center(i, Config.RAIDER_GROUP_RADIUS)
	
	# Random direction
	var angle: float = randf() * TAU
	var random_target: Vector2 = my_pos + Vector2(cos(angle), sin(angle)) * randf_range(100, 200)
	
	# Blend: 50% random, 30% toward group, 20% toward village
	var cohesion_target: Vector2 = group_center
	var village_target: Vector2 = my_pos + my_pos.direction_to(village_center) * 150.0
	
	var final_target: Vector2 = random_target * 0.5 + cohesion_target * 0.3 + village_target * 0.2
	
	manager.targets[i] = final_target
	manager.wander_centers[i] = my_pos  # Update center as they move
	manager._state.set_state(i, NPCState.State.WANDERING)


func _raider_wander_toward_village(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	var village_center: Vector2 = manager.village_center
	
	# Random direction biased toward village
	var angle: float = randf() * TAU
	var random_dir := Vector2(cos(angle), sin(angle))
	var village_dir: Vector2 = my_pos.direction_to(village_center)
	
	# 70% random, 30% toward village
	var final_dir: Vector2 = (random_dir * 0.7 + village_dir * 0.3).normalized()
	var dist: float = randf_range(150.0, 300.0)
	
	manager.targets[i] = my_pos + final_dir * dist
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
		# Raiders update wander center and eat if at farm
		if job == NPCState.Job.RAIDER:
			manager.wander_centers[i] = manager.positions[i]
			_raider_check_farm_food(i)
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
			if job == NPCState.Job.GUARD:
				manager.wander_centers[i] = manager.positions[i]
		elif target.distance_to(home_pos) < 10:
			if energy <= Config.ENERGY_EXHAUSTED:
				manager._state.set_state(i, NPCState.State.SLEEPING)
			else:
				manager._state.set_state(i, NPCState.State.RESTING)


func _raider_check_farm_food(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]
	
	# Check if near any farm
	for farm_pos in manager.farm_positions:
		if my_pos.distance_to(farm_pos) < 60.0:
			# Steal food! Restore energy
			manager.energies[i] = minf(Config.ENERGY_MAX, manager.energies[i] + Config.ENERGY_FARM_RESTORE)
			return
