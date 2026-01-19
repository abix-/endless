# npc_state.gd
# State validation and transitions
extends RefCounted
class_name NPCState

enum State {
	IDLE,        # Not doing anything
	SLEEPING,    # At home/camp, asleep
	FIGHTING,    # In combat
	FLEEING,     # Running from combat
	WALKING,     # Generic movement (farmer to/from places)
	FARMING,     # Farmer at farm
	OFF_DUTY,    # At home/camp, awake
	ON_DUTY,     # Guard at post
	PATROLLING,  # Guard moving between posts
	RAIDING,     # Raider going to/at farm
	RETURNING,   # Raider going home
}

enum Faction { VILLAGER, RAIDER }
enum Job { FARMER, GUARD, RAIDER }

# Valid states per job type
const VALID_STATES := {
	Job.FARMER: [State.IDLE, State.SLEEPING, State.FLEEING, State.WALKING, State.FARMING, State.OFF_DUTY],
	Job.GUARD: [State.IDLE, State.SLEEPING, State.FIGHTING, State.WALKING, State.OFF_DUTY, State.ON_DUTY, State.PATROLLING],
	Job.RAIDER: [State.IDLE, State.SLEEPING, State.FIGHTING, State.RAIDING, State.RETURNING, State.OFF_DUTY],
}

const STATE_NAMES := {
	State.IDLE: "Idle",
	State.SLEEPING: "Sleeping",
	State.FIGHTING: "Fighting",
	State.FLEEING: "Fleeing",
	State.WALKING: "Walking",
	State.FARMING: "Farming",
	State.OFF_DUTY: "Off Duty",
	State.ON_DUTY: "On Duty",
	State.PATROLLING: "Patrolling",
	State.RAIDING: "Raiding",
	State.RETURNING: "Returning",
}

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
			get_state_name(new_state), JOB_NAMES[job], i
		])
		return false

	manager.states[i] = new_state
	return true

func get_state_name(state: int) -> String:
	if state in STATE_NAMES:
		return STATE_NAMES[state]
	return "Unknown"

func get_job_name(job: int) -> String:
	if job >= 0 and job < JOB_NAMES.size():
		return JOB_NAMES[job]
	return "Unknown"
