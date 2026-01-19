# npc_combat.gd
# Handles enemy detection, attacking, damage, death, and raider alerts
extends RefCounted
class_name NPCCombat

var manager: Node

func _init(npc_manager: Node) -> void:
	manager = npc_manager

func process(delta: float) -> void:
	for i in manager.count:
		if manager.healths[i] <= 0:
			continue
		
		if manager.attack_timers[i] > 0:
			manager.attack_timers[i] -= delta
		
		var state: int = manager.states[i]
		match state:
			NPCState.State.FIGHTING:
				_process_fighting(i)
			NPCState.State.FLEEING:
				_process_fleeing(i)

func process_scanning(delta: float) -> void:
	var frame: int = Engine.get_process_frames()
	
	for i in manager.count:
		# Stagger: only process 1/4 of NPCs per frame
		if i % Config.SCAN_STAGGER != frame % Config.SCAN_STAGGER:
			continue
		
		if manager.healths[i] <= 0:
			continue
		
		var state: int = manager.states[i]
		if state in [NPCState.State.FIGHTING, NPCState.State.FLEEING, NPCState.State.SLEEPING]:
			continue
		
		manager.scan_timers[i] -= delta * Config.SCAN_STAGGER  # Compensate for stagger
		if manager.scan_timers[i] <= 0:
			manager.scan_timers[i] = Config.SCAN_INTERVAL
			var enemy: int = _find_enemy_for(i)
			
			if enemy >= 0:
				manager.current_targets[i] = enemy
				var will_flee: int = manager.will_flee[i]
				if will_flee == 1:
					manager._state.set_state(i, NPCState.State.FLEEING)
				else:
					manager._state.set_state(i, NPCState.State.FIGHTING)
					if manager.jobs[i] == NPCState.Job.RAIDER:
						_alert_nearby_raiders(i, enemy)

func _process_fighting(i: int) -> void:
	var target_idx: int = manager.current_targets[i]
	
	if target_idx < 0 or manager.healths[target_idx] <= 0:
		manager.current_targets[i] = -1
		manager._state.set_state(i, NPCState.State.IDLE)
		manager._decide_what_to_do(i)
		return
	
	var my_pos: Vector2 = manager.positions[i]
	var enemy_pos: Vector2 = manager.positions[target_idx]
	
	# Guards don't leash - they fight wherever they are
	var job: int = manager.jobs[i]
	if job != NPCState.Job.GUARD:
		var home_pos: Vector2 = manager.wander_centers[i]
		var leash: float = Config.LEASH_DISTANCE
		if job == NPCState.Job.RAIDER:
			leash = Config.LEASH_DISTANCE * Config.RAIDER_LEASH_MULTIPLIER
		
		var dist_to_home: float = my_pos.distance_to(home_pos)
		if dist_to_home > leash:
			manager.current_targets[i] = -1
			manager._state.set_state(i, NPCState.State.IDLE)
			manager._decide_what_to_do(i)
			return
	
	var dist_to_enemy: float = my_pos.distance_to(enemy_pos)
	if dist_to_enemy <= Config.ATTACK_RANGE:
		if manager.attack_timers[i] <= 0:
			_attack(i, target_idx)

func _process_fleeing(i: int) -> void:
	var target_idx: int = manager.current_targets[i]
	
	if target_idx < 0 or manager.healths[target_idx] <= 0:
		manager.current_targets[i] = -1
		manager._state.set_state(i, NPCState.State.IDLE)
		manager._decide_what_to_do(i)
		return
	
	var my_pos: Vector2 = manager.positions[i]
	var enemy_pos: Vector2 = manager.positions[target_idx]
	var dist: float = my_pos.distance_to(enemy_pos)
	
	if dist > Config.FLEE_DISTANCE:
		manager.current_targets[i] = -1
		manager._state.set_state(i, NPCState.State.IDLE)
		manager._decide_what_to_do(i)

func _attack(attacker: int, victim: int) -> void:
	manager.attack_timers[attacker] = Config.ATTACK_COOLDOWN
	manager.healths[victim] -= manager.attack_damages[attacker]
	manager.mark_health_dirty(victim)
	
	if manager.healths[victim] <= 0:
		_die(victim)
	else:
		var victim_target: int = manager.current_targets[victim]
		if victim_target < 0:
			manager.current_targets[victim] = attacker
			var will_flee: int = manager.will_flee[victim]
			if will_flee == 1:
				manager._state.set_state(victim, NPCState.State.FLEEING)
			else:
				manager._state.set_state(victim, NPCState.State.FIGHTING)

func _die(i: int) -> void:
	var victim_faction: int = manager.factions[i]
	manager.record_kill(victim_faction)
	manager.record_death(i)  # Record death time for respawn

	manager.healths[i] = 0
	manager._state.set_state(i, NPCState.State.IDLE)
	manager.current_targets[i] = -1
	manager.positions[i] = Vector2(-9999, -9999)
	manager._renderer.hide_npc(i)

func _alert_nearby_raiders(alerter_idx: int, target_idx: int) -> void:
	var pos: Vector2 = manager.positions[alerter_idx]
	var nearby: Array = manager._grid_get_nearby(pos)
	var alert_range_sq: float = Config.ALERT_RADIUS * Config.ALERT_RADIUS
	
	for other_idx in nearby:
		if other_idx == alerter_idx:
			continue
		if manager.healths[other_idx] <= 0:
			continue
		if manager.jobs[other_idx] != NPCState.Job.RAIDER:
			continue
		
		var state: int = manager.states[other_idx]
		if state in [NPCState.State.FIGHTING, NPCState.State.FLEEING]:
			continue
		
		var other_pos: Vector2 = manager.positions[other_idx]
		if pos.distance_squared_to(other_pos) > alert_range_sq:
			continue
		
		manager.current_targets[other_idx] = target_idx
		manager._state.set_state(other_idx, NPCState.State.FIGHTING)

func _find_enemy_for(i: int) -> int:
	var my_pos: Vector2 = manager.positions[i]
	var my_faction: int = manager.factions[i]
	var nearby: Array = manager._grid_get_nearby(my_pos)
	
	var nearest: int = -1
	var nearest_dist_sq: float = INF
	var checked: int = 0
	
	for other_idx in nearby:
		if other_idx == i:
			continue
		if manager.healths[other_idx] <= 0:
			continue
		
		var other_faction: int = manager.factions[other_idx]
		if not _is_hostile(my_faction, other_faction):
			continue
		
		checked += 1
		if checked >= Config.MAX_SCAN:
			break
		
		var other_pos: Vector2 = manager.positions[other_idx]
		var dist_sq: float = my_pos.distance_squared_to(other_pos)
		
		if dist_sq < nearest_dist_sq:
			nearest_dist_sq = dist_sq
			nearest = other_idx
	
	return nearest

func _is_hostile(faction_a: int, faction_b: int) -> bool:
	return (faction_a == NPCState.Faction.VILLAGER and faction_b == NPCState.Faction.RAIDER) or \
		   (faction_a == NPCState.Faction.RAIDER and faction_b == NPCState.Faction.VILLAGER)
