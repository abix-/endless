# npc_needs.gd
# Handles energy, hunger, time-based state changes, and goal-based raider AI
extends RefCounted
class_name NPCNeeds

var manager: Node


func _init(npc_manager: Node) -> void:
	manager = npc_manager


func on_time_tick(_hour: int, minute: int) -> void:
	# Every 15 minutes - reconsider decisions and update patrol timers
	if minute % 15 == 0:
		for i in manager.count:
			if manager.healths[i] <= 0:
				continue
			var state: int = manager.states[i]
			var job: int = manager.jobs[i]

			# Increment patrol timer for on-duty guards
			if job == NPCState.Job.GUARD and state == NPCState.State.ON_DUTY:
				manager.patrol_timer[i] += 15

			if state not in [NPCState.State.FIGHTING, NPCState.State.FLEEING]:
				manager._decide_what_to_do(i)

	# On the hour - update energy
	if minute != 0:
		return

	for i in manager.count:
		if manager.healths[i] <= 0:
			continue

		var state: int = manager.states[i]
		var max_hp: float = manager.get_scaled_max_health(i)

		# Energy update
		match state:
			NPCState.State.SLEEPING:
				manager.energies[i] = minf(Config.ENERGY_MAX, manager.energies[i] + Config.ENERGY_SLEEP_GAIN)
			NPCState.State.OFF_DUTY, NPCState.State.RESTING:
				manager.energies[i] = minf(Config.ENERGY_MAX, manager.energies[i] + Config.ENERGY_REST_GAIN)
			_:
				manager.energies[i] = maxf(0.0, manager.energies[i] - Config.ENERGY_ACTIVITY_DRAIN)

		# HP regen (3x faster when sleeping)
		if manager.healths[i] < max_hp:
			var regen: float = Config.HP_REGEN_SLEEP if state == NPCState.State.SLEEPING else Config.HP_REGEN_AWAKE
			manager.healths[i] = minf(max_hp, manager.healths[i] + regen)
			manager.mark_health_dirty(i)


func decide_what_to_do(i: int) -> void:
	if manager.healths[i] <= 0:
		return

	var job: int = manager.jobs[i]
	if job == NPCState.Job.RAIDER:
		_decide_raider(i)
		return
	if job == NPCState.Job.GUARD:
		_decide_guard(i)
		return

	# Farmer logic
	var energy: float = manager.energies[i]
	var state: int = manager.states[i]

	# Low energy - go home to sleep
	if energy <= Config.ENERGY_EXHAUSTED:
		if state != NPCState.State.SLEEPING:
			manager.targets[i] = manager.home_positions[i]
			manager.arrival_radii[i] = manager._arrival_home
			manager._state.set_state(i, NPCState.State.WALKING)
		return

	var is_work_time: bool = _is_work_time(i)

	if is_work_time:
		if state not in [NPCState.State.FARMING, NPCState.State.WALKING]:
			manager.targets[i] = manager.work_positions[i]
			manager.arrival_radii[i] = manager._arrival_farm
			manager._state.set_state(i, NPCState.State.WALKING)
	else:
		if state not in [NPCState.State.OFF_DUTY, NPCState.State.WALKING]:
			manager.targets[i] = manager.home_positions[i]
			manager.arrival_radii[i] = manager._arrival_home
			manager._state.set_state(i, NPCState.State.WALKING)


func _decide_guard(i: int) -> void:
	var energy: float = manager.energies[i]
	var state: int = manager.states[i]

	# Low energy - go home to sleep
	if energy <= Config.ENERGY_EXHAUSTED:
		if state != NPCState.State.SLEEPING:
			manager.targets[i] = manager.home_positions[i]
			manager.arrival_radii[i] = manager._arrival_home
			manager._state.set_state(i, NPCState.State.WALKING)
		return

	var is_work_time: bool = _is_work_time(i)

	if is_work_time:
		# Check if at post and waited long enough - move to next
		if state == NPCState.State.ON_DUTY and manager.patrol_timer[i] >= Config.GUARD_PATROL_WAIT:
			_guard_go_to_next_post(i)
		elif state not in [NPCState.State.ON_DUTY, NPCState.State.PATROLLING]:
			_guard_go_to_next_post(i)
	else:
		if state not in [NPCState.State.OFF_DUTY, NPCState.State.WALKING]:
			manager.targets[i] = manager.home_positions[i]
			manager.arrival_radii[i] = manager._arrival_home
			manager._state.set_state(i, NPCState.State.WALKING)


func _guard_go_to_next_post(i: int) -> void:
	var town_idx: int = manager.town_indices[i]
	if town_idx < 0 or town_idx >= manager.guard_posts_by_town.size():
		return

	var posts: Array = manager.guard_posts_by_town[town_idx]
	if posts.size() == 0:
		return

	var current_idx: int = manager.patrol_target_idx[i]
	var last_idx: int = manager.patrol_last_idx[i]

	# Find next post (not the one we just came from)
	var next_idx: int = (current_idx + 1) % posts.size()
	if next_idx == last_idx and posts.size() > 2:
		next_idx = (next_idx + 1) % posts.size()

	manager.patrol_last_idx[i] = current_idx
	manager.patrol_target_idx[i] = next_idx
	manager.patrol_timer[i] = 0

	manager.targets[i] = posts[next_idx]
	manager.arrival_radii[i] = manager._arrival_guard_post
	manager._state.set_state(i, NPCState.State.PATROLLING)


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
			manager.arrival_radii[i] = manager._arrival_camp
			manager._state.set_state(i, NPCState.State.RETURNING)
		return

	# Priority 3: Carrying food - return to camp
	if manager.carrying_food[i] == 1:
		_raider_return_to_camp(i)
		return

	# Priority 4: Go steal food from farms
	_raider_go_to_farm(i)


func _raider_return_to_camp(i: int) -> void:
	var home_pos: Vector2 = manager.home_positions[i]
	var my_pos: Vector2 = manager.positions[i]

	# If already at camp, deliver immediately instead of walking
	if my_pos.distance_to(home_pos) < manager._arrival_camp:
		_raider_deliver_food(i)
		manager.wander_centers[i] = my_pos
		manager._state.set_state(i, NPCState.State.RESTING)
		return

	manager.targets[i] = home_pos
	manager.arrival_radii[i] = manager._arrival_camp
	manager.wander_centers[i] = my_pos
	manager._state.set_state(i, NPCState.State.RETURNING)


func _raider_go_to_farm(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]

	if manager.farm_positions.size() == 0:
		return

	# Find nearest farm
	var best_farm: Vector2 = manager.farm_positions[0]
	var best_dist_sq: float = my_pos.distance_squared_to(best_farm)
	for farm_pos in manager.farm_positions:
		var dist_sq: float = my_pos.distance_squared_to(farm_pos)
		if dist_sq < best_dist_sq:
			best_dist_sq = dist_sq
			best_farm = farm_pos

	manager.targets[i] = best_farm
	manager.arrival_radii[i] = manager._arrival_farm
	manager.wander_centers[i] = my_pos
	manager._state.set_state(i, NPCState.State.RAIDING)


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

	if state == NPCState.State.RAIDING:
		# Raiders check if at farm to steal food
		manager.wander_centers[i] = manager.positions[i]
		_raider_check_steal_food(i)
		manager._state.set_state(i, NPCState.State.IDLE)
		decide_what_to_do(i)
	elif state in [NPCState.State.WALKING, NPCState.State.PATROLLING, NPCState.State.RETURNING]:
		var my_pos: Vector2 = manager.positions[i]
		var home_pos: Vector2 = manager.home_positions[i]
		var energy: float = manager.energies[i]
		var radius: float = manager.arrival_radii[i]

		# Guard arrived at patrol post
		if job == NPCState.Job.GUARD:
			var target: Vector2 = manager.targets[i]
			if state == NPCState.State.PATROLLING and my_pos.distance_to(target) < radius:
				manager._state.set_state(i, NPCState.State.ON_DUTY)
				manager.wander_centers[i] = my_pos
				manager.patrol_timer[i] = 0
			elif my_pos.distance_to(home_pos) < radius:
				if energy <= Config.ENERGY_EXHAUSTED:
					manager._state.set_state(i, NPCState.State.SLEEPING)
				else:
					manager._state.set_state(i, NPCState.State.OFF_DUTY)
		# Farmer arrived at work or home
		elif job == NPCState.Job.FARMER:
			var work_pos: Vector2 = manager.work_positions[i]
			if my_pos.distance_to(work_pos) < radius:
				manager._state.set_state(i, NPCState.State.FARMING)
			elif my_pos.distance_to(home_pos) < radius:
				if energy <= Config.ENERGY_EXHAUSTED:
					manager._state.set_state(i, NPCState.State.SLEEPING)
				else:
					manager._state.set_state(i, NPCState.State.OFF_DUTY)
					decide_what_to_do(i)
		# Raider arrived at camp
		elif job == NPCState.Job.RAIDER:
			if my_pos.distance_to(home_pos) < radius:
				manager.wander_centers[i] = my_pos
				_raider_deliver_food(i)
				if energy <= Config.ENERGY_EXHAUSTED:
					manager._state.set_state(i, NPCState.State.SLEEPING)
				else:
					manager._state.set_state(i, NPCState.State.RESTING)


func _raider_check_steal_food(i: int) -> void:
	var my_pos: Vector2 = manager.positions[i]

	for farm_pos in manager.farm_positions:
		if my_pos.distance_to(farm_pos) < manager._arrival_farm:
			manager.carrying_food[i] = 1
			return


func _raider_deliver_food(i: int) -> void:
	if manager.carrying_food[i] == 1:
		manager.carrying_food[i] = 0
		# Restore some energy from successful raid
		manager.energies[i] = minf(Config.ENERGY_MAX, manager.energies[i] + Config.ENERGY_FARM_RESTORE)
		# Notify main.gd to credit the camp
		var town_idx: int = manager.town_indices[i]
		if town_idx >= 0:
			manager.raider_delivered_food.emit(town_idx)
