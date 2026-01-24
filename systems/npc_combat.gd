# npc_combat.gd
# Handles enemy detection, attacking, damage, death, and raider alerts
extends RefCounted
class_name NPCCombat

var manager: Node

# Projectile queue for parallel mode (fired on main thread after parallel phase)
var _projectile_queue: Array = []  # [{from, to, damage, faction, attacker}, ...]
var _projectile_mutex: Mutex

func _init(npc_manager: Node) -> void:
	manager = npc_manager
	_projectile_mutex = Mutex.new()

func process(delta: float) -> void:
	for i in manager.count:
		if manager.healths[i] <= 0:
			continue
		if manager.awake[i] == 0:
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
		# Stagger: only process 1/8 of NPCs per frame
		if i % Config.SCAN_STAGGER != frame % Config.SCAN_STAGGER:
			continue

		if manager.healths[i] <= 0:
			continue
		if manager.awake[i] == 0:
			continue

		var state: int = manager.states[i]

		# Skip states that don't need enemy scanning
		# FIGHTING/FLEEING already have targets
		# RESTING NPCs are at home, not looking for fights
		if state in [NPCState.State.FIGHTING, NPCState.State.FLEEING, NPCState.State.RESTING]:
			continue

		# Optimization: Only scan if there's a threat nearby (cell has enemies)
		if not _cell_has_threat(i):
			continue

		manager.scan_timers[i] -= delta * Config.SCAN_STAGGER  # Compensate for stagger
		if manager.scan_timers[i] <= 0:
			manager.scan_timers[i] = Config.SCAN_INTERVAL
			var enemy: int = _find_enemy_for(i)

			if enemy >= 0:
				manager.current_targets[i] = enemy
				var job: int = manager.jobs[i]
				# Farmers always flee (they can't fight)
				if job == NPCState.Job.FARMER or _should_flee(i):
					manager._state.set_state(i, NPCState.State.FLEEING)
				else:
					manager._state.set_state(i, NPCState.State.FIGHTING)
					if job == NPCState.Job.RAIDER:
						_alert_nearby_raiders(i, enemy)
				# Force immediate navigation update
				manager._nav.force_logic_update(i)


func _cell_has_threat(i: int) -> bool:
	var my_pos: Vector2 = manager.positions[i]
	var my_faction: int = manager.factions[i]
	var nearby: Array = manager._grid.get_nearby(my_pos)

	for other_idx in nearby:
		if manager.healths[other_idx] > 0 and manager.factions[other_idx] != my_faction:
			return true

	return false

func _process_fighting(i: int) -> void:
	var target_idx: int = manager.current_targets[i]

	if target_idx < 0 or manager.healths[target_idx] <= 0:
		manager.current_targets[i] = -1
		manager._state.set_state(i, NPCState.State.IDLE)
		manager._decide_what_to_do(i)
		return

	# Guards/raiders flee when health drops below threshold
	if _should_flee(i):
		manager._state.set_state(i, NPCState.State.FLEEING)
		return

	var my_pos: Vector2 = manager.positions[i]
	var enemy_pos: Vector2 = manager.positions[target_idx]

	# If target is fleeing, check for closer non-fleeing enemies
	var target_state: int = manager.states[target_idx]
	if target_state == NPCState.State.FLEEING:
		var closer_enemy: int = _find_closer_non_fleeing_enemy(i, target_idx)
		if closer_enemy >= 0:
			manager.current_targets[i] = closer_enemy
			target_idx = closer_enemy
			enemy_pos = manager.positions[target_idx]

	var job: int = manager.jobs[i]
	var town_idx: int = manager.town_indices[i]

	# Check leash policy for guards
	var guard_has_leash := false
	if job == NPCState.Job.GUARD and town_idx >= 0 and town_idx < manager.town_policies.size():
		guard_has_leash = manager.town_policies[town_idx].guard_leash

	# Apply leash (guards only if policy enabled, farmers/raiders always)
	if job != NPCState.Job.GUARD or guard_has_leash:
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
	var attack_range: float = manager.attack_ranges[i]
	if dist_to_enemy <= attack_range:
		if manager.attack_timers[i] <= 0:
			_attack(i, target_idx)

func _process_fleeing(i: int) -> void:
	var target_idx: int = manager.current_targets[i]

	# Enemy dead - stop fleeing, but stay and heal if low HP
	if target_idx < 0 or manager.healths[target_idx] <= 0:
		manager.current_targets[i] = -1
		_stop_fleeing(i)
		return

	# Check if reached flee destination
	var my_pos: Vector2 = manager.positions[i]
	var flee_target: Vector2 = _get_flee_target(i)
	var dist_to_target: float = my_pos.distance_to(flee_target)

	if dist_to_target < manager._arrival_home:
		manager.current_targets[i] = -1
		_stop_fleeing(i)


func _stop_fleeing(i: int) -> void:
	var health_pct: float = manager.healths[i] / manager.get_scaled_max_health(i)
	var recovery_threshold := _get_recovery_threshold(i)
	if health_pct < recovery_threshold:
		# Stay and heal until recovery threshold
		manager._state.set_state(i, NPCState.State.OFF_DUTY)
		manager.recovering[i] = 1
	else:
		manager._state.set_state(i, NPCState.State.IDLE)
		manager._decide_what_to_do(i)


func _get_recovery_threshold(i: int) -> float:
	var town_idx: int = manager.town_indices[i]
	var job: int = manager.jobs[i]

	# Raiders use default threshold
	if job == NPCState.Job.RAIDER:
		return Config.RECOVERY_THRESHOLD

	# Check policies
	if town_idx >= 0 and town_idx < manager.town_policies.size():
		var policies: Dictionary = manager.town_policies[town_idx]
		# Prioritize healing = stay until 100%
		if policies.prioritize_healing:
			return 1.0
		return policies.recovery_hp

	return Config.RECOVERY_THRESHOLD

func _attack(attacker: int, victim: int) -> void:
	var cooldown: float = Config.ATTACK_COOLDOWN
	var job: int = manager.jobs[attacker]

	# Apply attack speed upgrade for guards
	if job == NPCState.Job.GUARD:
		var town_idx: int = manager.town_indices[attacker]
		if town_idx >= 0 and town_idx < manager.town_upgrades.size():
			var atk_speed_level: int = manager.town_upgrades[town_idx].guard_attack_speed
			if atk_speed_level > 0:
				cooldown *= 1.0 - (atk_speed_level * Config.UPGRADE_GUARD_ATTACK_SPEED)

	# Apply trait modifiers
	var npc_trait: int = manager.traits[attacker]
	if npc_trait == NPCState.Trait.EFFICIENT:
		cooldown *= 0.75  # 25% faster attacks
	elif npc_trait == NPCState.Trait.LAZY:
		cooldown *= 1.2   # 20% slower attacks

	manager.attack_timers[attacker] = cooldown
	var is_ranged: bool = job == NPCState.Job.GUARD or job == NPCState.Job.RAIDER

	if is_ranged and manager._projectiles:
		# Fire projectile - damage and flash happen on hit
		var from_pos: Vector2 = manager.positions[attacker]
		var target_pos: Vector2 = manager.positions[victim]
		var damage: float = _get_damage(attacker)
		var faction: int = manager.factions[attacker]
		manager._projectiles.fire(from_pos, target_pos, damage, faction, attacker)
	else:
		# Melee instant damage (scaled) - flash the victim
		var damage: float = _get_damage(attacker)
		manager.healths[victim] -= damage
		manager.mark_health_dirty(victim)
		manager._renderer.trigger_flash(victim)

		if manager.healths[victim] <= 0:
			_die(victim, attacker)
		else:
			_aggro_victim(attacker, victim)


func _aggro_victim(attacker: int, victim: int) -> void:
	# Wake victim on damage
	manager.wake_npc(victim)

	var victim_target: int = manager.current_targets[victim]
	if victim_target < 0:
		manager.current_targets[victim] = attacker
		var job: int = manager.jobs[victim]
		# Farmers always flee (they can't fight)
		if job == NPCState.Job.FARMER or _should_flee(victim):
			manager._state.set_state(victim, NPCState.State.FLEEING)
		else:
			manager._state.set_state(victim, NPCState.State.FIGHTING)
		# Force immediate navigation update on state change
		manager._nav.force_logic_update(victim)

func _die(i: int, killer: int = -1) -> void:
	var victim_faction: int = manager.factions[i]
	manager.record_kill(victim_faction)

	# Emit death signal for logging
	var killer_job: int = manager.jobs[killer] if killer >= 0 else -1
	var killer_level: int = manager.levels[killer] if killer >= 0 else 0
	manager.npc_died.emit(i, manager.jobs[i], manager.levels[i], manager.town_indices[i], killer_job, killer_level)

	# Grant XP to killer based on victim's level
	if killer >= 0 and manager.healths[killer] > 0:
		var xp_gained: int = manager.levels[i]
		manager.grant_xp(killer, xp_gained)

	# Release any reserved bed and farm
	manager.release_bed(i)
	manager.release_farm(i)

	manager.healths[i] = 0
	manager._state.set_state(i, NPCState.State.IDLE)
	manager.current_targets[i] = -1
	manager.positions[i] = Vector2(-9999, -9999)
	manager._renderer.hide_npc(i)
	manager.free_slot(i)

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
		manager._nav.force_logic_update(other_idx)

func _find_enemy_for(i: int) -> int:
	var my_pos: Vector2 = manager.positions[i]
	var my_faction: int = manager.factions[i]
	var job: int = manager.jobs[i]
	var nearby: Array = manager._grid_get_nearby(my_pos)

	# Calculate detection range (guards get upgraded alert radius)
	var detect_range: float = Config.ALERT_RADIUS
	if my_faction == NPCState.Faction.VILLAGER:
		var town_idx: int = manager.town_indices[i]
		if town_idx >= 0 and town_idx < manager.town_upgrades.size():
			var alert_level: int = manager.town_upgrades[town_idx].alert_radius
			detect_range *= 1.0 + alert_level * Config.UPGRADE_ALERT_RADIUS_BONUS

		# Non-aggressive guards only react to close enemies
		if job == NPCState.Job.GUARD and town_idx >= 0 and town_idx < manager.town_policies.size():
			if not manager.town_policies[town_idx].guard_aggressive:
				detect_range = manager.attack_ranges[i] * 1.5  # Only react within 1.5x attack range
	var detect_range_sq: float = detect_range * detect_range

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

		# Only detect within range
		if dist_sq > detect_range_sq:
			continue

		if dist_sq < nearest_dist_sq:
			nearest_dist_sq = dist_sq
			nearest = other_idx

	return nearest

func _is_hostile(faction_a: int, faction_b: int) -> bool:
	return (faction_a == NPCState.Faction.VILLAGER and faction_b == NPCState.Faction.RAIDER) or \
		   (faction_a == NPCState.Faction.RAIDER and faction_b == NPCState.Faction.VILLAGER)


func _should_flee(i: int) -> bool:
	var job: int = manager.jobs[i]
	var town_idx: int = manager.town_indices[i]
	var npc_trait: int = manager.traits[i]
	var health_pct: float = manager.healths[i] / manager.max_healths[i]

	# Get policies (if available)
	var policies: Dictionary = {}
	if town_idx >= 0 and town_idx < manager.town_policies.size():
		policies = manager.town_policies[town_idx]

	# Brave NPCs never flee
	if npc_trait == NPCState.Trait.BRAVE:
		return false

	# Coward flees at +20% higher threshold
	var coward_bonus: float = 0.2 if npc_trait == NPCState.Trait.COWARD else 0.0

	# Farmers
	if job == NPCState.Job.FARMER:
		# Check farmer_fight_back policy - if true, farmers don't auto-flee
		if not policies.is_empty() and policies.farmer_fight_back:
			var flee_threshold: float = policies.farmer_flee_hp + coward_bonus
			return health_pct < flee_threshold
		# Default: farmers always flee (will_flee[i] == 1)
		if manager.will_flee[i] == 1:
			return true

	# Guards - use policy flee threshold if available
	if job == NPCState.Job.GUARD:
		var flee_threshold: float = Config.GUARD_FLEE_THRESHOLD
		if not policies.is_empty():
			flee_threshold = policies.guard_flee_hp
		if health_pct < flee_threshold + coward_bonus:
			return true

	# Raiders flee below 50% (or 70% if coward) - no policy control
	if job == NPCState.Job.RAIDER and health_pct < Config.RAIDER_WOUNDED_THRESHOLD + coward_bonus:
		return true

	return false


func _get_flee_target(i: int) -> Vector2:
	var job: int = manager.jobs[i]
	if job == NPCState.Job.RAIDER:
		# Raiders flee to camp
		return manager.home_positions[i]
	else:
		# Farmers/guards flee to town center (fountain)
		var town_idx: int = manager.town_indices[i]
		if town_idx >= 0 and town_idx < manager.town_centers.size():
			return manager.town_centers[town_idx]
		return manager.home_positions[i]  # Fallback


func _get_damage(i: int) -> float:
	var damage: float = manager.get_scaled_damage(i)
	var npc_trait: int = manager.traits[i]

	if npc_trait == NPCState.Trait.STRONG:
		damage *= 1.25
	elif npc_trait == NPCState.Trait.BERSERKER:
		var hp_pct: float = manager.healths[i] / manager.get_scaled_max_health(i)
		if hp_pct < 0.5:
			damage *= 1.5

	return damage


func _find_closer_non_fleeing_enemy(i: int, current_target: int) -> int:
	var my_pos: Vector2 = manager.positions[i]
	var my_faction: int = manager.factions[i]
	var current_dist_sq: float = my_pos.distance_squared_to(manager.positions[current_target])
	var nearby: Array = manager._grid_get_nearby(my_pos)

	var best: int = -1
	var best_dist_sq: float = current_dist_sq

	for other_idx in nearby:
		if other_idx == i or other_idx == current_target:
			continue
		if manager.healths[other_idx] <= 0:
			continue
		if manager.states[other_idx] == NPCState.State.FLEEING:
			continue
		if not _is_hostile(my_faction, manager.factions[other_idx]):
			continue

		var dist_sq: float = my_pos.distance_squared_to(manager.positions[other_idx])
		if dist_sq < best_dist_sq:
			best_dist_sq = dist_sq
			best = other_idx

	return best


# ============================================================
# PARALLEL PROCESSING
# ============================================================

var _parallel_frame: int = 0
var _parallel_delta: float = 0.0

func process_scanning_parallel() -> void:
	_parallel_frame = Engine.get_process_frames()
	var task_id = WorkerThreadPool.add_group_task(_scan_single_npc, manager.count)
	WorkerThreadPool.wait_for_group_task_completion(task_id)


func _scan_single_npc(i: int) -> void:
	# Stagger: only process 1/16 of NPCs per frame
	if i % Config.SCAN_STAGGER != _parallel_frame % Config.SCAN_STAGGER:
		return

	if manager.healths[i] <= 0:
		return
	if manager.awake[i] == 0:
		return

	var state: int = manager.states[i]

	# Skip states that don't need enemy scanning
	if state in [NPCState.State.FIGHTING, NPCState.State.FLEEING, NPCState.State.RESTING]:
		return

	# Check if cell has threat (read-only, safe)
	if not _cell_has_threat(i):
		return

	# Find enemy (read-only grid access, safe)
	var enemy: int = _find_enemy_for(i)

	# Write to own current_target (safe - each NPC writes only to its own index)
	if enemy >= 0:
		manager.current_targets[i] = enemy
		# Note: State changes deferred to avoid race conditions
		# The main thread will detect targets and set states


func process_parallel(delta: float) -> void:
	_parallel_delta = delta
	_parallel_frame = Engine.get_process_frames()
	var task_id = WorkerThreadPool.add_group_task(_process_single_npc_combat, manager.count)
	WorkerThreadPool.wait_for_group_task_completion(task_id)


func _process_single_npc_combat(i: int) -> void:
	if manager.healths[i] <= 0:
		return
	if manager.awake[i] == 0:
		return

	# Decrement attack timer (safe - each NPC writes only to its own timer)
	if manager.attack_timers[i] > 0:
		manager.attack_timers[i] -= _parallel_delta

	var state: int = manager.states[i]

	if state == NPCState.State.FIGHTING:
		_process_fighting_parallel(i)


func _process_fighting_parallel(i: int) -> void:
	var target_idx: int = manager.current_targets[i]

	# Target dead or invalid - mark for state change (handled by main thread)
	if target_idx < 0 or manager.healths[target_idx] <= 0:
		manager.current_targets[i] = -1
		return

	# Flee check - switch to FLEEING if health too low
	if _should_flee(i):
		manager.states[i] = NPCState.State.FLEEING
		manager.last_logic_frame[i] = 0
		return

	var my_pos: Vector2 = manager.positions[i]
	var enemy_pos: Vector2 = manager.positions[target_idx]

	# Leash check - disengage if too far from home
	var job: int = manager.jobs[i]
	var town_idx: int = manager.town_indices[i]
	var has_leash := true
	if job == NPCState.Job.GUARD and town_idx >= 0 and town_idx < manager.town_policies.size():
		has_leash = manager.town_policies[town_idx].guard_leash
	if has_leash:
		var home_pos: Vector2 = manager.wander_centers[i]
		var leash: float = Config.LEASH_DISTANCE
		if job == NPCState.Job.RAIDER:
			leash *= Config.RAIDER_LEASH_MULTIPLIER
		if my_pos.distance_to(home_pos) > leash:
			manager.current_targets[i] = -1
			return

	var dist_to_enemy: float = my_pos.distance_to(enemy_pos)
	var attack_range: float = manager.attack_ranges[i]

	if dist_to_enemy <= attack_range and manager.attack_timers[i] <= 0:
		_attack_parallel(i, target_idx)


func _attack_parallel(attacker: int, victim: int) -> void:
	var cooldown: float = Config.ATTACK_COOLDOWN
	var job: int = manager.jobs[attacker]

	# Apply attack speed upgrade for guards
	if job == NPCState.Job.GUARD:
		var town_idx: int = manager.town_indices[attacker]
		if town_idx >= 0 and town_idx < manager.town_upgrades.size():
			var atk_speed_level: int = manager.town_upgrades[town_idx].guard_attack_speed
			if atk_speed_level > 0:
				cooldown *= 1.0 - (atk_speed_level * Config.UPGRADE_GUARD_ATTACK_SPEED)

	# Apply trait modifiers
	var npc_trait: int = manager.traits[attacker]
	if npc_trait == NPCState.Trait.EFFICIENT:
		cooldown *= 0.75
	elif npc_trait == NPCState.Trait.LAZY:
		cooldown *= 1.2

	manager.attack_timers[attacker] = cooldown

	var damage: float = _get_damage(attacker)
	var is_ranged: bool = job == NPCState.Job.GUARD or job == NPCState.Job.RAIDER

	if is_ranged and manager._projectiles:
		# Queue projectile for main thread (thread-safe)
		var from_pos: Vector2 = manager.positions[attacker]
		var target_pos: Vector2 = manager.positions[victim]
		var faction: int = manager.factions[attacker]
		_projectile_mutex.lock()
		_projectile_queue.append({
			"from": from_pos,
			"to": target_pos,
			"damage": damage,
			"faction": faction,
			"attacker": attacker
		})
		_projectile_mutex.unlock()
	else:
		# Melee: use pending_damage (thread-safe via mutex)
		manager.add_pending_damage(victim, damage)


# Fire queued projectiles (call from main thread after parallel phase)
func fire_queued_projectiles() -> void:
	if _projectile_queue.is_empty():
		return
	_projectile_mutex.lock()
	var queue := _projectile_queue.duplicate()
	_projectile_queue.clear()
	_projectile_mutex.unlock()

	for p in queue:
		manager._projectiles.fire(p.from, p.to, p.damage, p.faction, p.attacker)


# Internal versions for unified parallel processing (called from npc_manager)
func _scan_single_npc_internal(i: int, frame: int) -> void:
	# Stagger: only process 1/16 of NPCs per frame
	if i % Config.SCAN_STAGGER != frame % Config.SCAN_STAGGER:
		return

	var state: int = manager.states[i]
	if state in [NPCState.State.FIGHTING, NPCState.State.FLEEING, NPCState.State.RESTING]:
		return

	if not _cell_has_threat(i):
		return

	var enemy: int = _find_enemy_for(i)
	if enemy >= 0:
		manager.current_targets[i] = enemy


func _process_single_npc_combat_internal(i: int, delta: float, _frame: int) -> void:
	if manager.attack_timers[i] > 0:
		manager.attack_timers[i] -= delta

	var state: int = manager.states[i]
	if state == NPCState.State.FIGHTING:
		_process_fighting_parallel(i)
	elif state == NPCState.State.FLEEING:
		var target_idx: int = manager.current_targets[i]
		if target_idx < 0 or manager.healths[target_idx] <= 0:
			manager.current_targets[i] = -1
		else:
			# Check if reached flee destination
			var my_pos: Vector2 = manager.positions[i]
			var flee_target: Vector2
			var job: int = manager.jobs[i]
			if job == NPCState.Job.RAIDER:
				flee_target = manager.home_positions[i]
			else:
				var town_idx: int = manager.town_indices[i]
				if town_idx >= 0 and town_idx < manager.town_centers.size():
					flee_target = manager.town_centers[town_idx]
				else:
					flee_target = manager.home_positions[i]
			if my_pos.distance_to(flee_target) < manager._arrival_home:
				manager.current_targets[i] = -1
