extends Camera3D

@export var player_path: NodePath
@export var follow_speed = 5.0
@export var height = 10.0
@export var distance = 6.0

var player: Node3D

func _ready():
	player = get_node(player_path)

func _process(delta):
	if player:
		var target_pos = player.global_position
		target_pos.y = height
		target_pos.z += distance
		
		global_position = global_position.lerp(target_pos, delta * follow_speed)
		look_at(player.global_position, Vector3.UP)
