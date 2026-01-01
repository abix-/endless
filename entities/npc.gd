extends CharacterBody2D

signal died

@export var npc_name := "Unnamed"
@export var job := "None"
@export var faction := "neutral"
@export var move_speed := 50.0
@export var works_at_night := false

@onready var label: Label = $Label

# Needs
var energy := 100.0
var energy_decay_per_hour := 6.0
var energy_restore_per_hour := 12.0
var energy_restore_idle := 2.0

# Combat
var health := 100.0
var max_health := 100.0
var attack_damage := 10.0
var attack_range := 30.0
var attack_cooldown := 1.0
var detection_range := 100.0
var _attack_timer := 0.0
var current_target: Node = null
var will_flee := false
var flee_threshold := 0.5
var leash_distance := 200.0
var _chase_time := 0.0
var max_chase_time := 10.0

# Enemy scanning optimization
var _enemy_scan_timer := 0.0
var _enemy_scan_interval := 0.5

# State
enum State { IDLE, WALKING, SLEEPING, WORKING, RESTING, WANDERING, FIGHTING, FLEEING }
var current_state: State = State.IDLE

# Locations
var home_location: Node
var work_location: Node
var target_position: Vector2
var walk_purpose: State

# Wander settings
var wander_center: Vector2
var wander_radius := 150.0

# Faction hostility
var hostile_factions := {
	"villager": ["raider"],
	"raider": ["villager"],
	"neutral": []
}

func _ready() -> void:
	WorldClock.time_tick.connect(_on_time_tick)
	wander_center = global_position
	_enemy_scan_timer = randf() * _enemy_scan_interval
	SpatialGrid.register_npc(self)
	_decide_what_to_do()
	_update_label()
	_set_color()

func _set_color() -> void:
	var sprite := $Sprite2D
	match job:
		"Farmer":
			sprite.modulate = Color.GREEN
		"Guard":
			sprite.modulate = Color.BLUE
		"Raider":
			sprite.modulate = Color.RED
		_:
			sprite.modulate = Color.WHITE

func _process(delta: float) -> void:
	if current_state == State.SLEEPING or current_state == State.RESTING or current_state == State.WORKING:
		return
	
	var old_pos = global_position
	
	if _attack_timer > 0:
		_attack_timer -= delta
	
	# Only scan for enemies if NOT already in combat
	if current_state != State.FIGHTING and current_state != State.FLEEING:
		_enemy_scan_timer += delta
		if _enemy_scan_timer >= _enemy_scan_interval:
			_enemy_scan_timer = 0.0
			if current_state != State.SLEEPING:
				var enemy := _find_nearest_enemy()
				if enemy:
					current_target = enemy
					if will_flee:
						_change_state(State.FLEEING)
					else:
						_change_state(State.FIGHTING)
					return
	
	if current_state == State.FLEEING:
		_process_flee(delta)
		SpatialGrid.update_npc(self, old_pos)
		return
	
	if current_state == State.FIGHTING:
		_process_combat(delta)
		SpatialGrid.update_npc(self, old_pos)
		return
	
	if current_state == State.WALKING or current_state == State.WANDERING:
		var direction := global_position.direction_to(target_position)
		var distance := global_position.distance_to(target_position)
		
		if distance < 5.0:
			_arrive_at_target()
		else:
			global_position += direction * move_speed * delta
		
		SpatialGrid.update_npc(self, old_pos)

func _process_combat(delta: float) -> void:
	if not is_instance_valid(current_target) or current_target.health <= 0:
		current_target = null
		_chase_time = 0.0
		_change_state(State.IDLE)
		_decide_what_to_do()
		return
	
	var distance_to_enemy = global_position.distance_to(current_target.global_position)
	var distance_to_home = global_position.distance_to(wander_center)
	
	_chase_time += delta
	if distance_to_home > leash_distance or _chase_time > max_chase_time:
		current_target = null
		_chase_time = 0.0
		_change_state(State.IDLE)
		_decide_what_to_do()
		return
	
	if distance_to_enemy <= attack_range:
		_chase_time = 0.0
		if _attack_timer <= 0:
			_attack(current_target)
	else:
		var direction = global_position.direction_to(current_target.global_position)
		global_position += direction * move_speed * delta

func _process_flee(delta: float) -> void:
	if not is_instance_valid(current_target):
		current_target = null
		_change_state(State.IDLE)
		_decide_what_to_do()
		return
	
	var distance = global_position.distance_to(current_target.global_position)
	
	if distance > 150.0:
		current_target = null
		_change_state(State.IDLE)
		_decide_what_to_do()
		return
	
	var direction = current_target.global_position.direction_to(global_position)
	global_position += direction * move_speed * 1.2 * delta

func _attack(enemy: Node) -> void:
	_attack_timer = attack_cooldown
	enemy.take_damage(attack_damage, self)

func take_damage(amount: float, attacker: Node) -> void:
	health -= amount
	_update_label()
	
	if health <= 0:
		_die()
		return
	
	current_target = attacker
	
	if will_flee and health < max_health * flee_threshold:
		_change_state(State.FLEEING)
	elif current_state != State.FIGHTING and current_state != State.FLEEING:
		if will_flee:
			_change_state(State.FLEEING)
		else:
			_change_state(State.FIGHTING)

func _die() -> void:
	SpatialGrid.unregister_npc(self)
	died.emit()
	queue_free()

func _find_nearest_enemy() -> Node:
	var nearby := SpatialGrid.get_nearby_npcs(global_position)
	var nearest: Node = null
	var nearest_dist := detection_range
	
	for npc in nearby:
		if npc == self:
			continue
		if not is_instance_valid(npc):
			continue
		if npc.health <= 0:
			continue
		if _is_hostile_to(npc.faction):
			var dist = global_position.distance_to(npc.global_position)
			if dist < nearest_dist:
				nearest_dist = dist
				nearest = npc
	return nearest

func _is_hostile_to(other_faction: String) -> bool:
	if faction in hostile_factions:
		return other_faction in hostile_factions[faction]
	return false

func _on_time_tick(hour: int, minute: int) -> void:
	if minute == 0:
		match current_state:
			State.IDLE:
				energy = maxf(0.0, energy - energy_decay_per_hour)
			State.WALKING:
				energy = maxf(0.0, energy - energy_decay_per_hour)
			State.WORKING:
				energy = maxf(0.0, energy - energy_decay_per_hour)
			State.WANDERING:
				energy = maxf(0.0, energy - energy_decay_per_hour)
			State.FIGHTING:
				energy = maxf(0.0, energy - energy_decay_per_hour)
			State.FLEEING:
				energy = maxf(0.0, energy - energy_decay_per_hour)
			State.RESTING:
				energy = minf(100.0, energy + energy_restore_idle)
			State.SLEEPING:
				energy = minf(100.0, energy + energy_restore_per_hour)
	
	if minute % 15 == 0:
		if current_state != State.FIGHTING and current_state != State.FLEEING:
			_decide_what_to_do()
		_update_label()

func _is_work_time() -> bool:
	if works_at_night:
		return not WorldClock.is_daytime()
	else:
		return WorldClock.is_daytime()

func _decide_what_to_do() -> void:
	if faction == "raider":
		_decide_raider()
		return
	
	if energy <= 20.0:
		if current_state != State.SLEEPING:
			_go_to(home_location.global_position, State.SLEEPING)
		return
	
	if _is_work_time():
		if current_state != State.WORKING and current_state != State.WALKING:
			_go_to(work_location.global_position, State.WORKING)
		return
	
	if not _is_work_time():
		if current_state != State.RESTING and current_state != State.WALKING:
			_go_to(home_location.global_position, State.RESTING)

func _decide_raider() -> void:
	if energy <= 20.0:
		_change_state(State.RESTING)
		return
	
	if current_state != State.WANDERING:
		_wander()

func _wander() -> void:
	var angle := randf() * TAU
	var distance := randf_range(50.0, wander_radius)
	target_position = wander_center + Vector2(cos(angle), sin(angle)) * distance
	_change_state(State.WANDERING)

func _go_to(destination: Vector2, purpose: State) -> void:
	target_position = destination
	walk_purpose = purpose
	_change_state(State.WALKING)

func _arrive_at_target() -> void:
	if current_state == State.WANDERING:
		_change_state(State.IDLE)
	else:
		_change_state(walk_purpose)

func _change_state(new_state: State) -> void:
	current_state = new_state

func _update_label() -> void:
	var state_names := {
		State.IDLE: "Idle",
		State.WALKING: "Walk",
		State.SLEEPING: "Zzz",
		State.WORKING: "Work",
		State.RESTING: "Rest",
		State.WANDERING: "Wander",
		State.FIGHTING: "Fight",
		State.FLEEING: "Flee"
	}
	#label.text = "%s (%s) | H:%.0f E:%.0f | %s" % [npc_name, job, health, energy, state_names[current_state]]
