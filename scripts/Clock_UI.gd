extends Control

@onready var time_label = $TimeLabel
# Reference the autoloaded Clock singleton
@onready var game_clock = Clock  # Using the name you registered in Project Settings

func _ready():
	# Connect to game clock signals
	game_clock.hour_changed.connect(_on_hour_changed)
	game_clock.day_changed.connect(_on_day_changed)
	
	# Update the time immediately
	_update_time_display()
	
	# Set up a timer to update the minutes display
	var timer = Timer.new()
	timer.wait_time = 0.5  # Update twice per second for smoother display
	timer.autostart = true
	timer.timeout.connect(_on_timer_timeout)
	add_child(timer)

func _on_hour_changed(hour):
	_update_time_display()
	
func _on_day_changed(day):
	_update_time_display()

func _on_timer_timeout():
	_update_time_display()

func _update_time_display():
	var hour = game_clock.hours
	var minute = int(game_clock.minutes)
	var am_pm = "AM"
	
	# Convert to 12-hour format
	if hour >= 12:
		am_pm = "PM"
		if hour > 12:
			hour = hour - 12
	elif hour == 0:
		hour = 12
	
	# Format the time string with leading zeros for minutes
	var time_str = "%d:%02d%s" % [hour, minute, am_pm]
	time_label.text = time_str
