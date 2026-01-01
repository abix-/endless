extends Node

signal time_tick(hour, minute)
signal hour_changed(hour)
signal day_changed(day)

# Time settings
var minutes_per_tick := 1
var ticks_per_real_second := 10.0  # game speed

# Current time state
var current_day := 1
var current_hour := 6   # start at 6 AM
var current_minute := 0

var _tick_accumulator := 0.0
var paused := false

func _ready() -> void:
	print("World clock started: Day %d, %02d:%02d" % [current_day, current_hour, current_minute])

func _process(delta: float) -> void:
	if paused:
		return
	
	_tick_accumulator += delta * ticks_per_real_second
	
	while _tick_accumulator >= 1.0:
		_tick_accumulator -= 1.0
		_advance_time()

func _advance_time() -> void:
	current_minute += minutes_per_tick
	
	if current_minute >= 60:
		current_minute = 0
		current_hour += 1
		
		if current_hour >= 24:
			current_hour = 0
			current_day += 1
			day_changed.emit(current_day)
		
		hour_changed.emit(current_hour)
	
	time_tick.emit(current_hour, current_minute)

# Helper to get time as float (6.5 = 6:30 AM)
func get_time_float() -> float:
	return current_hour + (current_minute / 60.0)

# Check if current time is within a range
func is_time_between(start_hour: float, end_hour: float) -> bool:
	var current := get_time_float()
	if start_hour <= end_hour:
		return current >= start_hour and current < end_hour
	else:  # wraps midnight
		return current >= start_hour or current < end_hour

func is_daytime() -> bool:
	return is_time_between(6.0, 20.0)

func _input(event: InputEvent) -> void:
	if event is InputEventKey and event.pressed:
		match event.keycode:
			KEY_EQUAL:  # + key
				WorldClock.ticks_per_real_second *= 2.0
				print("Speed: %.1fx" % WorldClock.ticks_per_real_second)
			KEY_MINUS:
				WorldClock.ticks_per_real_second /= 2.0
				print("Speed: %.1fx" % WorldClock.ticks_per_real_second)
			KEY_SPACE:
				WorldClock.paused = not WorldClock.paused
				print("Paused: %s" % WorldClock.paused)
