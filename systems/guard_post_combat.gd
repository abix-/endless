# guard_post_combat.gd
# Handles guard post targeting and attacking
extends RefCounted
class_name GuardPostCombat

var manager: Node  # npc_manager reference
var main: Node     # main.gd reference

# Per-post combat state (parallel arrays)
var attack_timers: PackedFloat32Array
var post_positions: PackedVector2Array
var post_town_indices: PackedInt32Array
var post_slot_keys: Array[String] = []
var post_count: int = 0

const MAX_POSTS := 500


func _init(npc_manager: Node, main_node: Node) -> void:
	manager = npc_manager
	main = main_node
	_init_arrays()


func _init_arrays() -> void:
	attack_timers.resize(MAX_POSTS)
	post_positions.resize(MAX_POSTS)
	post_town_indices.resize(MAX_POSTS)
	post_slot_keys.resize(MAX_POSTS)
	for i in MAX_POSTS:
		post_slot_keys[i] = ""


func register_post(pos: Vector2, town_idx: int, slot_key: String) -> int:
	if post_count >= MAX_POSTS:
		return -1
	var idx := post_count
	post_positions[idx] = pos
	post_town_indices[idx] = town_idx
	post_slot_keys[idx] = slot_key
	attack_timers[idx] = 0.0
	post_count += 1
	return idx


func unregister_post(town_idx: int, slot_key: String) -> void:
	# Find and remove the post (swap with last)
	for i in post_count:
		if post_town_indices[i] == town_idx and post_slot_keys[i] == slot_key:
			post_count -= 1
			if i < post_count:
				# Swap with last
				post_positions[i] = post_positions[post_count]
				post_town_indices[i] = post_town_indices[post_count]
				post_slot_keys[i] = post_slot_keys[post_count]
				attack_timers[i] = attack_timers[post_count]
			return


func process(delta: float) -> void:
	for i in post_count:
		# Decrease attack timer
		if attack_timers[i] > 0:
			attack_timers[i] -= delta
			continue

		var town_idx: int = post_town_indices[i]
		var slot_key: String = post_slot_keys[i]

		# Get upgrades for this post
		if town_idx >= main.guard_post_upgrades.size():
			continue
		var upgrades: Dictionary = main.guard_post_upgrades[town_idx].get(slot_key, {})

		# Skip if attack not enabled
		if not upgrades.get("attack_enabled", false):
			continue

		# Find enemy in range
		var enemy := _find_enemy(i, upgrades)
		if enemy >= 0:
			_fire_at(i, enemy, upgrades)
			attack_timers[i] = Config.GUARD_POST_ATTACK_COOLDOWN


func _find_enemy(post_idx: int, upgrades: Dictionary) -> int:
	var pos: Vector2 = post_positions[post_idx]
	var range_level: int = upgrades.get("range_level", 0)
	var attack_range: float = Config.GUARD_POST_BASE_RANGE * Config.get_guard_post_stat_scale(range_level)
	var range_sq: float = attack_range * attack_range

	var nearby: Array = manager._grid.get_nearby(pos)
	var nearest := -1
	var nearest_dist_sq := INF

	for npc_idx in nearby:
		if manager.healths[npc_idx] <= 0:
			continue
		if manager.factions[npc_idx] == NPCState.Faction.VILLAGER:
			continue  # Only target raiders

		var dist_sq: float = pos.distance_squared_to(manager.positions[npc_idx])
		if dist_sq < range_sq and dist_sq < nearest_dist_sq:
			nearest_dist_sq = dist_sq
			nearest = npc_idx

	return nearest


func _fire_at(post_idx: int, target: int, upgrades: Dictionary) -> void:
	var damage_level: int = upgrades.get("damage_level", 0)
	var damage: float = Config.GUARD_POST_BASE_DAMAGE * Config.get_guard_post_stat_scale(damage_level)
	var from_pos: Vector2 = post_positions[post_idx]
	var target_pos: Vector2 = manager.positions[target]

	# Use negative shooter index for guard posts (-1000 - post_idx)
	var shooter_id: int = -1000 - post_idx
	manager._projectiles.fire(from_pos, target_pos, damage, NPCState.Faction.VILLAGER, shooter_id)
