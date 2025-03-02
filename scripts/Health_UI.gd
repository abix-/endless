extends Control

@onready var health_bar = $Panel/ProgressBar
@onready var health_label = $Panel/Label

var player = null

func initialize(player_node):
	player = player_node
	update_health_display()

func update_health_display():
	if player:
		var current = player.current_health
		var maximum = player.max_health
		
		# Update progress bar
		health_bar.max_value = maximum
		health_bar.value = current
		
		# Update label
		health_label.text = "Health: %d/%d" % [current, maximum]

func _process(_delta):
	# Update every frame to catch any health changes
	if player:
		update_health_display()
