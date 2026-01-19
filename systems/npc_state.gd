# npc_state.gd
# State validation and transitions
extends RefCounted
class_name NPCState

enum State { IDLE, WALKING, SLEEPING, WORKING, RESTING, WANDERING, FIGHTING, FLEEING }
enum Faction { VILLAGER, RAIDER }
enum Job { FARMER, GUARD, RAIDER }

# Valid states per job type
const VALID_STATES := {
	Job.FARMER: [State.IDLE, State.WALKING, State.SLEEPING, State.WORKING, State.RESTING, State.FLEEING],
	Job.GUARD: [State.IDLE, State.WALKING, State.SLEEPING, State.WORKING, State.RESTING, State.FIGHTING, State.WANDERING],
	Job.RAIDER: [State.IDLE, State.WALKING, State.SLEEPING, State.WANDERING, State.RESTING, State.FIGHTING],
}

const STATE_NAMES := ["Idle", "Walk", "Zzz", "Work", "Rest", "Wander", "Fight", "Flee"]
const JOB_NAMES := ["Farmer", "Guard", "Raider"]

var manager: Node

func _init(npc_manager: Node) -> void:
	manager = npc_manager

func is_valid_for_job(job: int, state: int) -> bool:
	if job not in VALID_STATES:
		return true
	return state in VALID_STATES[job]

func set_state(i: int, new_state: int) -> bool:
	var job: int = manager.jobs[i]
	
	if not is_valid_for_job(job, new_state):
		push_warning("Invalid state %s for %s at index %d" % [
			STATE_NAMES[new_state], JOB_NAMES[job], i
		])
		return false
	
	manager.states[i] = new_state
	return true

func get_state_name(state: int) -> String:
	if state >= 0 and state < STATE_NAMES.size():
		return STATE_NAMES[state]
	return "Unknown"

func get_job_name(job: int) -> String:
	if job >= 0 and job < JOB_NAMES.size():
		return JOB_NAMES[job]
	return "Unknown"
