# config.gd
# Game constants - all tunable values in one place
extends Node

# NPC movement and combat
const MOVE_SPEED := 50.0
const ATTACK_RANGE := 30.0
const SEPARATION_RADIUS := 18.0
const SEPARATION_STRENGTH := 120.0
const ATTACK_COOLDOWN := 1.0
const SCAN_INTERVAL := 0.2
const MAX_SCAN := 50
const SCAN_STAGGER := 8

# NPC capacity
const MAX_NPC_COUNT := 3000

# Combat distances
const LEASH_DISTANCE := 400.0
const FLEE_DISTANCE := 150.0
const ALERT_RADIUS := 200.0
const GUARD_FLEE_THRESHOLD := 0.33  # Guards flee below 33% health
const RECOVERY_THRESHOLD := 0.75    # NPCs heal until 75% before resuming

# Raider AI
const RAIDER_CONFIDENCE_THRESHOLD := 3
const RAIDER_WOUNDED_THRESHOLD := 0.5
const RAIDER_HUNGRY_THRESHOLD := 50.0
const RAIDER_GROUP_RADIUS := 200.0
const RAIDER_RETREAT_DIST := 400.0
const RAIDER_LEASH_MULTIPLIER := 1.5

# Guard patrol
const GUARD_PATROL_WAIT := 30  # Minutes to wait at each post

# Spatial grid
const GRID_SIZE := 64
const GRID_CELL_SIZE := 100.0
const GRID_CELL_CAPACITY := 64

# LOD thresholds (squared distances)
const LOD_NEAR_SQ := 160000.0    # 400px
const LOD_MID_SQ := 640000.0     # 800px
const LOD_FAR_SQ := 1440000.0    # 1200px

# World bounds
const WORLD_WIDTH := 8000
const WORLD_HEIGHT := 8000
const WORLD_MARGIN := 400  # Keep towns away from edges

# Town settings
const FARMERS_PER_TOWN := 10
const GUARDS_PER_TOWN := 30
const TOWN_GRID_SPACING := 100  # Pixels between building slot centers (96px building + gap)
const MAX_FARMERS_PER_TOWN := 10  # Population cap (can be upgraded)
const MAX_GUARDS_PER_TOWN := 30   # Population cap (can be upgraded)
const FARMS_PER_TOWN := 2
const GUARD_POSTS_PER_TOWN := 6
const RAIDERS_PER_CAMP := 15
const CAMP_DISTANCE := 1100  # Distance from town to raider camp (past guard posts)
const SPAWN_INTERVAL_HOURS := 4  # Hours between spawning new NPCs

# Rendering
const RENDER_MARGIN := 100.0
const NPC_SPRITE_SIZE := 16.0
const NPC_CLICK_RADIUS := 16.0

# NPC stats
const FARMER_HP := 50.0
const FARMER_DAMAGE := 5.0
const FARMER_RANGE := 30.0  # Melee
const GUARD_HP := 120.0
const GUARD_DAMAGE := 15.0
const GUARD_RANGE := 150.0  # Ranged
const RAIDER_HP := 120.0
const RAIDER_DAMAGE := 15.0
const RAIDER_RANGE := 150.0  # Ranged

# Projectiles
const MAX_PROJECTILES := 500
const PROJECTILE_SPEED := 200.0
const PROJECTILE_SIZE := 6.0
const PROJECTILE_LIFETIME := 3.0
const PROJECTILE_HIT_RADIUS := 10.0

# Energy
const ENERGY_MAX := 100.0
const ENERGY_SLEEP_GAIN := 12.0
const ENERGY_REST_GAIN := 5.0
const ENERGY_ACTIVITY_DRAIN := 6.0
const ENERGY_HUNGRY := 50.0  # Go home to rest when below this
const ENERGY_STARVING := 10.0  # Actually eat food when below this

# Food
const FOOD_PER_MEAL := 1

# HP Regen (per hour)
const HP_REGEN_AWAKE := 2.0
const HP_REGEN_SLEEP := 6.0  # 3x faster when sleeping

# Town Upgrades
const UPGRADE_MAX_LEVEL := 10
const UPGRADE_COSTS := [10, 25, 50, 100, 200, 400, 800, 1500, 3000, 5000]  # Food cost per level
# Guard upgrades
const UPGRADE_GUARD_HEALTH_BONUS := 0.1   # +10% HP per level
const UPGRADE_GUARD_ATTACK_BONUS := 0.1   # +10% damage per level
const UPGRADE_GUARD_RANGE_BONUS := 0.05   # +5% range per level
const UPGRADE_GUARD_SIZE_BONUS := 0.05    # +5% size per level
const UPGRADE_GUARD_ATTACK_SPEED := 0.08  # -8% cooldown per level (faster attacks)
const UPGRADE_GUARD_MOVE_SPEED := 0.05    # +5% move speed per level
# Economy upgrades
const UPGRADE_FARM_YIELD_BONUS := 0.15    # +15% food per level
const UPGRADE_FARMER_HP_BONUS := 0.2      # +20% farmer HP per level
# Defense upgrades
const UPGRADE_HEALING_RATE_BONUS := 0.2   # +20% HP regen per level
const UPGRADE_ALERT_RADIUS_BONUS := 0.1   # +10% alert radius per level
# Utility upgrades
const UPGRADE_FOOD_EFFICIENCY := 0.1      # -10% food cost per level (1.0, 0.9, 0.8...)
# Population upgrades
const UPGRADE_FARMER_CAP_BONUS := 2       # +2 max farmers per level
const UPGRADE_GUARD_CAP_BONUS := 10       # +10 max guards per level
