# Chunk 4: World Data Migration Plan

## Goal
Move world data (towns, farms, beds, guard posts, camps) from GDScript to Bevy Resources. This enables future NPC logic to query the world without crossing FFI boundary.

## Current State (GDScript in main.gd)

```gdscript
var towns: Array = []  # {name, center, grid, slots, guard_posts, camp}
var town_food: PackedInt32Array
var camp_food: PackedInt32Array
var town_upgrades: Array  # {guard_health, farm_yield, ...}
var town_policies: Array  # {eat_food, guard_flee_hp, ...}
var guard_post_upgrades: Array[Dictionary]  # slot_key -> {attack_enabled, ...}
```

Per-town arrays in npc_manager.gd:
```gdscript
var farms_by_town: Array  # Array[Array[Vector2]]
var farm_occupant_counts: Array  # Array[PackedInt32Array]
var beds_by_town: Array  # Array[Array[Vector2]]
var bed_occupants: Array  # Array[PackedInt32Array] (-1 = free)
var guard_posts_by_town: Array  # Array[Array[Vector2]]
var town_centers: Array[Vector2]
```

## Target State (Bevy Resources)

### Core Structs

```rust
// === World Layout (immutable after init) ===

#[derive(Clone)]
pub struct Town {
    pub name: String,
    pub center: Vector2,
    pub camp_position: Vector2,
}

#[derive(Clone)]
pub struct Farm {
    pub position: Vector2,
    pub town_idx: u32,
}

#[derive(Clone)]
pub struct Bed {
    pub position: Vector2,
    pub town_idx: u32,
}

#[derive(Clone)]
pub struct GuardPost {
    pub position: Vector2,
    pub town_idx: u32,
    pub patrol_order: u32,  // 0-3 for clockwise perimeter
}

// === World State (mutable) ===

#[derive(Resource, Default)]
pub struct WorldData {
    pub towns: Vec<Town>,
    pub farms: Vec<Farm>,
    pub beds: Vec<Bed>,
    pub guard_posts: Vec<GuardPost>,
}

#[derive(Resource, Default)]
pub struct FarmOccupancy {
    pub occupant_count: Vec<i32>,  // NPCs working each farm
}

#[derive(Resource, Default)]
pub struct BedOccupancy {
    pub occupant_npc: Vec<i32>,  // NPC index or -1
}

#[derive(Resource, Default)]
pub struct FoodStorage {
    pub town_food: Vec<i32>,
    pub camp_food: Vec<i32>,
}
```

### GDScript API

```rust
impl EcsNpcManager {
    // Called once after world generation
    #[func]
    fn init_world(&mut self, town_count: i32);

    // Called for each town
    #[func]
    fn add_town(&mut self, name: GString, center_x: f32, center_y: f32,
                camp_x: f32, camp_y: f32);

    // Called for each building
    #[func]
    fn add_farm(&mut self, x: f32, y: f32, town_idx: i32);

    #[func]
    fn add_bed(&mut self, x: f32, y: f32, town_idx: i32);

    #[func]
    fn add_guard_post(&mut self, x: f32, y: f32, town_idx: i32, patrol_order: i32);

    // State queries (for future NPC logic)
    #[func]
    fn get_nearest_free_bed(&self, town_idx: i32, x: f32, y: f32) -> i32;

    #[func]
    fn get_nearest_free_farm(&self, town_idx: i32, x: f32, y: f32) -> i32;

    #[func]
    fn get_patrol_post(&self, town_idx: i32, patrol_index: i32) -> Vector2;

    #[func]
    fn get_town_center(&self, town_idx: i32) -> Vector2;

    #[func]
    fn get_camp_position(&self, town_idx: i32) -> Vector2;
}
```

## Implementation Steps

### Step 1: Define Rust Structs
- Add structs to lib.rs
- Add Resources to Bevy App
- No GDScript changes yet

### Step 2: Add World Init API
- `init_world(town_count)` - allocate vectors
- `add_town(...)` - append town
- `add_farm(...)`, `add_bed(...)`, `add_guard_post(...)` - append buildings
- Called from main.gd after `_generate_world()`

### Step 3: Add Query API
- `get_nearest_free_bed(town_idx, x, y)` - returns bed index or -1
- `get_nearest_free_farm(town_idx, x, y)` - returns farm index or -1
- `get_patrol_post(town_idx, patrol_idx)` - returns guard post position
- `get_town_center(town_idx)`, `get_camp_position(town_idx)`

### Step 4: Add Occupancy API
- `reserve_bed(bed_idx, npc_idx)` - returns success
- `release_bed(bed_idx)`
- `reserve_farm(farm_idx)` - increment count
- `release_farm(farm_idx)` - decrement count

### Step 5: Integration Test
- Create test scene that:
  1. Inits world with 1 town
  2. Adds 2 farms, 4 beds, 4 guard posts
  3. Spawns NPCs via existing `spawn_npc()`
  4. Queries buildings via new API
  5. Validates positions match

### Step 6: Wire Up main.gd
- After `_generate_world()`, call Rust init functions
- Keep GDScript arrays as backup until Chunk 5+ replaces them
- Log to confirm sync

## Files Changed

| File | Changes |
|------|---------|
| rust/src/lib.rs | Add world structs, Resources, GDScript API |
| main.gd | Add calls to init_world(), add_town(), add_farm(), etc. |
| scripts/ecs_test.gd | Optional: add world data test scenario |
| CHANGELOG.md | Document new features |

## Dependencies

- Chunk 3 (GPU physics) complete - need working spawn/render
- No dependency on NPC logic migration

## Not In Scope

- Moving upgrade/policy data (stays in GDScript for now)
- Food generation/consumption logic (stays in GDScript)
- Guard post combat system (stays in GDScript)
- Dynamic building add/remove (GDScript handles UI, just needs to also call Rust)

## Success Criteria

1. Bevy Resources contain world layout after init
2. GDScript can query buildings via Rust API
3. ecs_test.tscn still passes all tests
4. No performance regression (world init is one-time cost)

## Future Work (Chunk 5+)

Once world data is in Bevy:
- Guard patrol system can query posts directly
- Farmer farm assignment can use Rust
- Bed reservation can be fully Rust-side
- Eventually remove duplicate GDScript arrays
