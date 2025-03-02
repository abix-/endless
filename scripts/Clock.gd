extends Node

signal day_changed(day)
signal hour_changed(hour)

var minutes = 50
var hours = 6
var days = 0

# Set how long a day should last in real-world minutes
@export var day_length_minutes: float = 5.0  # Default: 5 real minutes = 1 game day

# This will be calculated based on day_length_minutes
var time_scale: float = 0.0

func _ready():
	# Calculate the time scale based on the desired day length
	_update_time_scale()

# Call this if you change day_length_minutes at runtime
func set_day_length(real_minutes: float) -> void:
	day_length_minutes = real_minutes
	_update_time_scale()

func _update_time_scale() -> void:
	# Formula: (24 hours * 60 minutes) / (day_length_minutes * 60 seconds)
	# This converts "X real minutes per day" to "Y game minutes per real second"
	time_scale = (24.0 * 60.0) / (day_length_minutes * 60.0)
	print("Day length set to %s real minutes. Time scale: %s" % [day_length_minutes, time_scale])

func _process(delta):
	# Update time
	var previous_hour = hours
	var previous_day = days
	
	minutes += delta * time_scale
	
	# Handle hour change
	if minutes >= 60:
		minutes = 0
		hours += 1
		emit_signal("hour_changed", hours)
	
	# Handle day change
	if hours >= 24:
		hours = 0
		days += 1
		emit_signal("day_changed", days)

func get_time_of_day():
	return hours + (minutes / 60.0)

func is_morning():
	return hours >= 6 and hours < 12

func is_afternoon():
	return hours >= 12 and hours < 18

func is_evening():
	return hours >= 18 and hours < 22

func is_night():
	return hours >= 22 or hours < 6

# Get current time as a formatted string (for debugging)
func get_time_string() -> String:
	var period = "AM"
	var display_hour = hours
	
	if hours >= 12:
		period = "PM"
		if hours > 12:
			display_hour = hours - 12
	elif hours == 0:
		display_hour = 12
	
	return "%d:%02d %s (Day %d)" % [display_hour, int(minutes), period, days]
