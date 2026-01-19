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
const RESPAWN_MINUTES := 720  # 12 hours

# Combat distances
const LEASH_DISTANCE := 400.0
const FLEE_DISTANCE := 150.0
const ALERT_RADIUS := 200.0

# Raider AI
const RAIDER_CONFIDENCE_THRESHOLD := 3
const RAIDER_WOUNDED_THRESHOLD := 0.5
const RAIDER_HUNGRY_THRESHOLD := 50.0
const RAIDER_GROUP_RADIUS := 200.0
const RAIDER_RETREAT_DIST := 400.0
const RAIDER_LEASH_MULTIPLIER := 1.5

# Spatial grid
const GRID_SIZE := 64
const GRID_CELL_SIZE := 100.0
const GRID_CELL_CAPACITY := 64

# LOD thresholds (squared distances)
const LOD_NEAR_SQ := 160000.0    # 400px
const LOD_MID_SQ := 640000.0     # 800px
const LOD_FAR_SQ := 1440000.0    # 1200px

# World bounds
const WORLD_WIDTH := 6000
const WORLD_HEIGHT := 4500
const WORLD_MARGIN := 400  # Keep towns away from edges

# Town settings
const FARMERS_PER_TOWN := 10
const GUARDS_PER_TOWN := 30
const FARMS_PER_TOWN := 2
const GUARD_POSTS_PER_TOWN := 6
const RAIDERS_PER_CAMP := 30
const CAMP_DISTANCE := 900  # Distance from town to raider camp

# Rendering
const RENDER_MARGIN := 100.0
const NPC_SPRITE_SIZE := 16.0
const NPC_CLICK_RADIUS := 16.0

# NPC stats
const FARMER_HP := 50.0
const FARMER_DAMAGE := 5.0
const FARMER_RANGE := 30.0  # Melee
const GUARD_HP := 150.0
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
const ENERGY_EXHAUSTED := 20.0
const ENERGY_FARM_RESTORE := 30.0
