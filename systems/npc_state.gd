# npc_state.gd
# State validation and transitions
extends RefCounted
class_name NPCState

enum State {
	IDLE,        # Not doing anything
	RESTING,     # At home/camp, recovering
	FIGHTING,    # In combat
	FLEEING,     # Running from combat
	WALKING,     # Generic movement (farmer to/from places)
	FARMING,     # Farmer at farm
	OFF_DUTY,    # At home/camp, awake
	ON_DUTY,     # Guard at post
	PATROLLING,  # Guard moving between posts
	RAIDING,     # Raider going to/at farm
	RETURNING,   # Raider going home
	WANDERING,   # Off-duty wandering around town
}

enum Faction { VILLAGER, RAIDER }
enum Job { FARMER, GUARD, RAIDER }

enum Trait {
	NONE,       # No special trait
	BRAVE,      # Won't flee (guards/raiders)
	COWARD,     # Flees at higher HP threshold
	EFFICIENT,  # +25% work output (farm yield, attack speed)
	HARDY,      # +25% max HP
	LAZY,       # -20% work output
	STRONG,     # +25% damage
	SWIFT,      # +25% move speed
	SHARPSHOT,  # +25% attack range
	BERSERKER,  # +50% damage below 50% HP
}

const TRAIT_NAMES := {
	Trait.NONE: "",
	Trait.BRAVE: "Brave",
	Trait.COWARD: "Coward",
	Trait.EFFICIENT: "Efficient",
	Trait.HARDY: "Hardy",
	Trait.LAZY: "Lazy",
	Trait.STRONG: "Strong",
	Trait.SWIFT: "Swift",
	Trait.SHARPSHOT: "Sharpshot",
	Trait.BERSERKER: "Berserker",
}

# First names for NPCs - 55 names
const FIRST_NAMES := [
	"Ada", "Aldric", "Bran", "Cara", "Dax", "Elara", "Finn", "Gwen",
	"Hal", "Iris", "Jace", "Kira", "Liam", "Mira", "Nox", "Orin",
	"Pax", "Quinn", "Ryn", "Sera", "Thane", "Una", "Vale", "Wren",
	"Xara", "Yara", "Zane", "Ash", "Bex", "Cole", "Dara", "Eli",
	"Fay", "Gren", "Hope", "Ivo", "Jade", "Knox", "Luna", "Max",
	"Neve", "Oak", "Pip", "Rue", "Sol", "Tara", "Uri", "Vera",
	"Abix", "Charlie", "Tomato", "Potato", "John", "Steve", "Geoff",
]

# Last names for NPCs - 100 names (48 × 100 = 4,800 unique combinations)
const LAST_NAMES := [
	# Place-based (20)
	"Brook", "Dale", "Field", "Ford", "Glen", "Grove", "Hall", "Heath",
	"Hill", "Marsh", "Mead", "Moor", "Ridge", "Shaw", "Stone", "Thorn",
	"Vale", "Wick", "Wood", "Wold",
	# Occupation-based (20)
	"Archer", "Baker", "Carver", "Cooper", "Farmer", "Fisher", "Fletcher", "Forger",
	"Harper", "Hunter", "Mason", "Miller", "Porter", "Reeve", "Shepherd", "Smith",
	"Tanner", "Thatcher", "Turner", "Weaver",
	# Nature-based (20)
	"Ash", "Birch", "Briar", "Elm", "Fern", "Frost", "Hawk", "Ivy",
	"Moss", "Oak", "Pine", "Raven", "Reed", "Rose", "Sage", "Storm",
	"Swift", "Thorn", "Wolf", "Wren",
	# Descriptive (20)
	"Black", "Bright", "Brown", "Dark", "Fair", "Gold", "Gray", "Green",
	"High", "Iron", "Long", "Old", "Red", "Sharp", "Silver", "Strong",
	"True", "White", "Wild", "Young",
	# Compound (20)
	"Ashford", "Blackwood", "Coldwell", "Eastbrook", "Fairfield", "Goldsmith", "Greenwood", "Highmore",
	"Ironside", "Longshore", "Northgate", "Oakhart", "Redmane", "Silverbrook", "Stoneheart", "Swiftwind",
	"Thornwood", "Westfall", "Whitmore", "Winterborn",
]

# Valid states per job type
const VALID_STATES := {
	Job.FARMER: [State.IDLE, State.RESTING, State.FLEEING, State.WALKING, State.FARMING, State.OFF_DUTY, State.WANDERING],
	Job.GUARD: [State.IDLE, State.RESTING, State.FIGHTING, State.FLEEING, State.WALKING, State.OFF_DUTY, State.ON_DUTY, State.PATROLLING, State.WANDERING],
	Job.RAIDER: [State.IDLE, State.RESTING, State.FIGHTING, State.FLEEING, State.RAIDING, State.RETURNING, State.OFF_DUTY],
}

const STATE_NAMES := {
	State.IDLE: "Idle",
	State.RESTING: "Resting",
	State.FIGHTING: "Fighting",
	State.FLEEING: "Fleeing",
	State.WALKING: "Walking",
	State.FARMING: "Farming",
	State.OFF_DUTY: "Off Duty",
	State.ON_DUTY: "On Duty",
	State.PATROLLING: "Patrolling",
	State.RAIDING: "Raiding",
	State.RETURNING: "Returning",
	State.WANDERING: "Wandering",
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
