# Spawn System

## Overview

NPCs are created through a single unified `spawn_npc()` API. Slot allocation uses Bevy's `SlotAllocator` resource, which reuses dead NPC indices before allocating new ones. Job determines the component template at spawn time. All GPU writes go through `GPU_UPDATE_QUEUE` — no direct `buffer_update()` calls in the spawn path.

## Data Flow

```
GDScript: spawn_npc(x, y, job, faction, opts: Dictionary)
│
├─ allocate_slot() via Bevy SlotAllocator resource
│   ├─ Try slots.free.pop() (recycled from dead NPC)
│   └─ Else slots.next++ (high-water mark)
│
├─ Build SpawnNpcMsg with slot_idx
│
└─ Send via GodotToBevy channel
         │
         ▼ (next frame, Bevy Step::Drain)
   drain_spawn_queue → SpawnNpcMsg
         │
         ▼ (Step::Spawn)
   spawn_npc_system
   ├─ Push GPU_UPDATE_QUEUE: SetPosition, SetTarget,
   │   SetColor, SetSpeed, SetFaction, SetHealth
   ├─ Update GPU_DISPATCH_COUNT (max slot + 1)
   │
   └─ match job:
      Guard  → Energy, AttackStats, AttackTimer, Guard,
               PatrolRoute, OnDuty
      Farmer → Energy, Farmer, WorkPosition, GoingToWork
      Raider → Energy, AttackStats, AttackTimer, Stealer,
               Raiding, FleeThreshold, LeashRange,
               WoundedThreshold
      Fighter→ AttackStats, AttackTimer
```

## Slot Allocation

Slot allocation uses Bevy's `SlotAllocator` resource (defined in `resources.rs`):

```rust
pub struct SlotAllocator {
    pub next: usize,      // High-water mark
    pub free: Vec<usize>, // Recycled slots from dead NPCs
}

impl SlotAllocator {
    pub fn alloc(&mut self) -> Option<usize> {
        self.free.pop().or_else(|| {
            if self.next < MAX_NPC_COUNT {
                let idx = self.next;
                self.next += 1;
                Some(idx)
            } else {
                None
            }
        })
    }
    pub fn free(&mut self, slot: usize) { self.free.push(slot); }
}
```

Both spawn (`allocate_slot()` in lib.rs) and death (`death_cleanup_system`) use this single resource, ensuring freed slots are properly recycled. The `next` counter is a high-water mark — it only grows (or resets to 0). `GPU_DISPATCH_COUNT` (separate static) tracks how many NPCs have initialized GPU buffers — see [messages.md](messages.md).

## GDScript Spawn API

Single method replaces 5 job-specific methods:

```gdscript
# Returns slot index or -1 if at capacity
spawn_npc(x, y, job, faction, opts: Dictionary) -> int
```

**Required params:**

| Param | Values | Notes |
|-------|--------|-------|
| x, y | float | Spawn position |
| job | 0=Farmer, 1=Guard, 2=Raider, 3=Fighter | Determines component template |
| faction | 0=Villager, 1=Raider | GPU targeting |

**Optional params (Dictionary):**

| Key | Type | Default | Notes |
|-----|------|---------|-------|
| home_x, home_y | float | -1.0 | Home/camp position |
| work_x, work_y | float | -1.0 | Farm position (farmers only) |
| town_idx | int | -1 | Town association |
| starting_post | int | -1 | Patrol start (guards only) |
| attack_type | int | 0 | 0=melee, 1=ranged (fighters only) |

```gdscript
# Guard at patrol post 2:
ecs.spawn_npc(pos.x, pos.y, 1, 0, {"home_x": home.x, "home_y": home.y, "town_idx": 0, "starting_post": 2})
# Farmer:
ecs.spawn_npc(pos.x, pos.y, 0, 0, {"home_x": home.x, "home_y": home.y, "work_x": farm.x, "work_y": farm.y, "town_idx": 0})
# Ranged fighter:
ecs.spawn_npc(pos.x, pos.y, 3, 1, {"attack_type": 1})
# Simple NPC (all defaults):
ecs.spawn_npc(pos.x, pos.y, 3, 0, {})
```

## spawn_npc_system (generic)

Base components (all NPCs): `NpcIndex`, `Job`, `Speed(100)`, `Health(100)`, `Faction`, `Home`

Job-specific templates:

| Job | Additional Components |
|-----|----------------------|
| Guard | `Energy`, `AttackStats`, `AttackTimer(0)`, `Guard { town_idx }`, `PatrolRoute`, `OnDuty { ticks: 0 }` |
| Farmer | `Energy`, `Farmer { town_idx }`, `WorkPosition`, `GoingToWork` |
| Raider | `Energy`, `AttackStats`, `AttackTimer(0)`, `Stealer`, `FleeThreshold(0.50)`, `LeashRange(400)`, `WoundedThreshold(0.25)` |
| Fighter | `AttackStats` (melee or ranged via attack_type), `AttackTimer(0)` |

GPU writes (via GPU_UPDATE_QUEUE, all jobs): `SetPosition`, `SetTarget` (= spawn position, or work position for farmers), `SetColor` (job-based), `SetSpeed(100)`, `SetFaction`, `SetHealth(100)`

### reset_bevy_system
Checks `RESET_BEVY` flag. If set, despawns all entities with `NpcIndex`, clears `NpcEntityMap`, resets `NpcCount`.

## Known Issues / Limitations

- **npc_count never decreases**: High-water mark. 1000 spawns + 999 deaths = npc_count still 1000. Grid and buffers sized to high-water mark, not active count.
- **No spawn validation**: spawn_npc doesn't verify the town_idx is valid or that guard posts exist. Bad input silently creates a guard with no patrol route.
- **One-frame GPU delay**: GPU writes go through GPU_UPDATE_QUEUE, drained in `process()`. NPC won't render until the frame after Bevy processes the spawn. At 140fps this is invisible.

## Rating: 8/10

Single spawn path with job-as-template pattern. Slot index carried in message — fixes the previous dual-counter bug. All GPU writes routed through unified queue. GpuData resource eliminated.
