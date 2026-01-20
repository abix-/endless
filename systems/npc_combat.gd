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
		# Stagger: only process 1/8 of NPCs per frame
		if i % Config.SCAN_STAGGER != frame % Config.SCAN_STAGGER:
			continue

		if manager.healths[i] <= 0:
			continue

		var state: int = manager.states[i]

		# Skip states that don't need enemy scanning
		# FIGHTING/FLEEING already have targets
		# RESTING NPCs are at home, not looking for fights
		if state in [NPCState.State.FIGHTING, NPCState.State.FLEEING, NPCState.State.RESTING]:
			continue

		# Optimization: Only scan if there's a threat nearby (cell has enemies)
		var my_cell: int = manager._grid._cell_index(manager.positions[i])
		if not _cell_has_threat(i, my_cell):
			continue

		manager.scan_timers[i] -= delta * Config.SCAN_STAGGER  # Compensate for stagger
		if manager.scan_timers[i] <= 0:
			manager.scan_timers[i] = Config.SCAN_INTERVAL
			var enemy: int = _find_enemy_for(i)

			if enemy >= 0:
				manager.current_targets[i] = enemy
				if _should_flee(i):
					manager._state.set_state(i, NPCState.State.FLEEING)
				else:
					manager._state.set_state(i, NPCState.State.FIGHTING)
					if manager.jobs[i] == NPCState.Job.RAIDER:
						_alert_nearby_raiders(i, enemy)
				# Force immediate navigation update
				manager._nav.force_logic_update(i)


func _cell_has_threat(i: int, cell_idx: int) -> bool:
	# Check this cell and adjacent cells for enemies
	var my_faction: int = manager.factions[i]
	var cx: int = cell_idx % Config.GRID_SIZE
	@warning_ignore("integer_division")
	var cy: int = cell_idx / Config.GRID_SIZE

	for dy in range(-1, 2):
		var ny: int = cy + dy
		if ny < 0 or ny >= Config.GRID_SIZE:
			continue
		for dx in range(-1, 2):
			var nx: int = cx + dx
			if nx < 0 or nx >= Config.GRID_SIZE:
				continue

			var check_cell: int = ny * Config.GRID_SIZE + nx
			var start: int = manager._grid.grid_cell_starts[check_cell]
			var cell_count: int = manager._grid.grid_cell_counts[check_cell]

			# Quick check: any enemy faction in this cell?
			for j in cell_count:
				var other_idx: int = manager._grid.grid_cells[start + j]
				if manager.factions[other_idx] != my_faction:
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

	# Guards don't leash - they fight wherever they are
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
	if health_pct < Config.RECOVERY_THRESHOLD:
		# Stay and heal until 75%
		manager._state.set_state(i, NPCState.State.OFF_DUTY)
		manager.recovering[i] = 1
	else:
		manager._state.set_state(i, NPCState.State.IDLE)
		manager._decide_what_to_do(i)

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
	var trait: int = manager.traits[attacker]
	if trait == NPCState.Trait.EFFICIENT:
		cooldown *= 0.75  # 25% faster attacks
	elif trait == NPCState.Trait.LAZY:
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
	var victim_target: int = manager.current_targets[victim]
	if victim_target < 0:
		manager.current_targets[victim] = attacker
		if _should_flee(victim):
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
	var nearby: Array = manager._grid_get_nearby(my_pos)

	# Calculate detection range (guards get upgraded alert radius)
	var detect_range: float = Config.ALERT_RADIUS
	if my_faction == NPCState.Faction.VILLAGER:
		var town_idx: int = manager.town_indices[i]
		if town_idx >= 0 and town_idx < manager.town_upgrades.size():
			var alert_level: int = manager.town_upgrades[town_idx].alert_radius
			detect_range *= 1.0 + alert_level * Config.UPGRADE_ALERT_RADIUS_BONUS
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
	# Farmers always flee
	if manager.will_flee[i] == 1:
		return true

	var trait: int = manager.traits[i]
	# Brave NPCs never flee
	if trait == NPCState.Trait.BRAVE:
		return false

	var health_pct: float = manager.healths[i] / manager.max_healths[i]
	var job: int = manager.jobs[i]

	# Coward flees at +20% higher threshold
	var coward_bonus: float = 0.2 if trait == NPCState.Trait.COWARD else 0.0

	# Guards flee below 33% (or 53% if coward)
	if job == NPCState.Job.GUARD and health_pct < Config.GUARD_FLEE_THRESHOLD + coward_bonus:
		return true
	# Raiders flee below 50% (or 70% if coward)
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
	var trait: int = manager.traits[i]

	if trait == NPCState.Trait.STRONG:
		damage *= 1.25
	elif trait == NPCState.Trait.BERSERKER:
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
