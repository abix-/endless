# Chunk 5: Guard Logic Migration Plan

## Goal
Migrate guard behavior from GDScript to Bevy ECS. Guards should patrol between posts and rest when low on energy, entirely controlled by Rust systems.

## Current GDScript Logic (npc_needs.gd)

```gdscript
func _decide_guard(i: int) -> void:
    var energy: float = manager.energies[i]
    var state: int = manager.states[i]

    # Priority 1: Rest if energy < 50
    if energy < Config.ENERGY_HUNGRY:
        if state != NPCState.State.RESTING:
            _go_home(i)
        return

    # Priority 2: Patrol
    if state == ON_DUTY and patrol_timer >= GUARD_PATROL_WAIT:
        _guard_go_to_next_post(i)
    elif state not in [ON_DUTY, PATROLLING]:
        _guard_go_to_next_post(i)
```

**Guard States:**
- `PATROLLING` - Moving to next patrol post
- `ON_DUTY` - Standing at post, waiting
- `WALKING` - Going home to rest
- `RESTING` - At bed, recovering energy

**Key Values:**
- `ENERGY_HUNGRY = 50` - Go home threshold
- `GUARD_PATROL_WAIT = 180` - Ticks at post before moving (3 hours)
- Patrol order: clockwise through 4 corner posts

## Target Bevy Architecture

### Components

```rust
// State markers (only one active at a time)
#[derive(Component)]
pub struct Patrolling;

#[derive(Component)]
pub struct OnDuty {
    pub post_index: u32,      // Which post we're at
    pub ticks_waiting: u32,   // How long we've been here
}

#[derive(Component)]
pub struct Resting;

#[derive(Component)]
pub struct Walking;

// Core NPC data
#[derive(Component)]
pub struct Guard {
    pub town_idx: u32,
    pub current_post: u32,    // 0-3 clockwise
}

#[derive(Component)]
pub struct Energy(pub f32);   // 0-100

#[derive(Component)]
pub struct HomePosition(pub Vector2);
```

### Systems

```rust
// Runs every tick (or every N frames for performance)
fn guard_decision_system(
    mut commands: Commands,
    world: Res<WorldData>,
    query: Query<(Entity, &Guard, &Energy, &NpcIndex), Without<Patrolling>>,
) {
    for (entity, guard, energy, npc_idx) in query.iter() {
        if energy.0 < 50.0 {
            // Go rest
            commands.entity(entity)
                .remove::<OnDuty>()
                .insert(Walking);
            // Set target to home...
        } else {
            // Go patrol
            commands.entity(entity)
                .remove::<OnDuty>()
                .insert(Patrolling);
            // Set target to next post...
        }
    }
}

fn guard_patrol_system(
    mut commands: Commands,
    world: Res<WorldData>,
    mut query: Query<(Entity, &mut Guard, &NpcIndex), With<Patrolling>>,
    // Check arrival from GPU...
) {
    // When arrived at post: remove Patrolling, add OnDuty
}

fn guard_on_duty_system(
    mut commands: Commands,
    mut query: Query<(Entity, &Guard, &mut OnDuty)>,
) {
    for (entity, guard, mut on_duty) in query.iter_mut() {
        on_duty.ticks_waiting += 1;
        if on_duty.ticks_waiting >= 180 {
            // Time to move to next post
            commands.entity(entity)
                .remove::<OnDuty>()
                .insert(Patrolling);
        }
    }
}

fn energy_system(
    mut query: Query<(&mut Energy, Option<&Resting>)>,
) {
    for (mut energy, resting) in query.iter_mut() {
        if resting.is_some() {
            energy.0 = (energy.0 + 0.2).min(100.0);  // Recover while resting
        } else {
            energy.0 = (energy.0 - 0.1).max(0.0);   // Drain while active
        }
    }
}
```

### GDScript API Additions

```rust
impl EcsNpcManager {
    // Spawn guard with energy and home position
    #[func]
    fn spawn_guard(&mut self, x: f32, y: f32, town_idx: i32, home_x: f32, home_y: f32);

    // Get guard state for debugging/UI
    #[func]
    fn get_guard_state(&self, npc_index: i32) -> i32;  // 0=patrol, 1=duty, 2=rest, 3=walk

    // Get energy for debugging/UI
    #[func]
    fn get_npc_energy(&self, npc_index: i32) -> f32;
}
```

## Implementation Steps

### Step 1: Add Guard Components
- Add state marker components (Patrolling, OnDuty, Resting, Walking)
- Add Guard component with town_idx and current_post
- Add Energy component
- Add HomePosition component

### Step 2: Modify spawn_npc for Guards
- Create `spawn_guard()` that sets up Guard + Energy + HomePosition
- Initialize with Patrolling state
- Set initial target to first patrol post

### Step 3: Energy System
- Drain energy over time (active NPCs)
- Recover energy over time (resting NPCs)
- Simple per-frame update

### Step 4: Guard Decision System
- Check energy < 50 → go rest
- Check rested (energy > 80) → go patrol
- Runs periodically, not every frame

### Step 5: Patrol System
- On arrival at post → switch to OnDuty
- Track ticks at post
- After 180 ticks → switch to Patrolling, target next post

### Step 6: Arrival Detection
- Read GPU arrival buffer
- Match arrivals to entities
- Trigger state transitions

### Step 7: Test 7: Guard Patrol
- Spawn 4 guards in town
- Add 4 patrol posts
- Watch guards cycle through posts
- Drain energy → watch guards go rest → watch them return

## Files Changed

| File | Changes |
|------|---------|
| rust/src/lib.rs | Add components, systems, spawn_guard API |
| scripts/ecs_test.gd | Add Test 7: Guard Patrol |
| scenes/ecs_test.tscn | Add Test7 button |

## Key Challenges

1. **Arrival Detection**: GPU owns positions, need to read arrival buffer and match to Bevy entities
2. **Target Setting**: When state changes, must update GPU target buffer
3. **Tick Rate**: Systems shouldn't run every frame - need periodic scheduling

## Success Criteria

1. Guards spawn and immediately start patrolling
2. Guards cycle through 4 posts clockwise
3. After ~3 minutes, guards rest (energy depleted)
4. After resting, guards resume patrol
5. Visual: see guards moving between posts in Test 7

## Not In Scope (Chunk 5)

- Combat (Chunk 7)
- Fleeing behavior
- Night shift logic
- Town policies affecting guard behavior
- UI integration
