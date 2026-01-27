# Spawn System

## Overview

NPCs are created through a single unified `spawn_npc()` API. Slot allocation reuses dead NPC indices via FREE_SLOTS before allocating new ones. Job determines the component template at spawn time. All GPU writes go through `GPU_UPDATE_QUEUE` — no direct `buffer_update()` calls in the spawn path.

## Data Flow

```
GDScript: spawn_npc(x, y, job, faction, home_x, home_y, work_x, work_y, town_idx, starting_post)
│
├─ allocate_slot()
│   ├─ Try FREE_SLOTS.pop() (recycled from dead NPC)
│   └─ Else NPC_SLOT_COUNTER++ (high-water mark)
│
├─ Build SpawnNpcMsg with slot_idx
│
└─ Push to SPAWN_QUEUE
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
```

## Slot Allocation

```rust
fn allocate_slot() -> Option<usize> {
    // 1. Reuse dead NPC slot
    if let Some(recycled) = FREE_SLOTS.pop() {
        return Some(recycled);
    }
    // 2. Allocate new slot from high-water mark
    if NPC_SLOT_COUNTER < MAX_NPC_COUNT {
        let idx = NPC_SLOT_COUNTER;
        NPC_SLOT_COUNTER += 1;
        return Some(idx);
    }
    None // At capacity
}
```

`NPC_SLOT_COUNTER` is a high-water mark — it only grows (or resets to 0). Dead slots are recycled through `FREE_SLOTS` but don't decrement the counter. Slot index is carried in `SpawnNpcMsg.slot_idx` so Bevy creates the entity at the correct GPU buffer index. `GPU_DISPATCH_COUNT` (separate from `NPC_SLOT_COUNTER`) tracks how many NPCs have initialized GPU buffers — see [messages.md](messages.md).

## GDScript Spawn API

Single method replaces 5 job-specific methods:

```gdscript
# Returns slot index or -1 if at capacity
spawn_npc(x, y, job, faction, home_x, home_y, work_x, work_y, town_idx, starting_post) -> int
```

| Param | Values | Notes |
|-------|--------|-------|
| job | 0=Farmer, 1=Guard, 2=Raider | Determines component template |
| faction | 0=Villager, 1=Raider | GPU targeting |
| home_x/y | position or -1,-1 | Home/camp position |
| work_x/y | position or -1,-1 | Farm position (farmers only) |
| town_idx | 0+ or -1 | Town association |
| starting_post | 0+ or -1 | Patrol start (guards only) |

## spawn_npc_system (generic)

Base components (all NPCs): `NpcIndex`, `Job`, `Speed(100)`, `Health(100)`, `Faction`, `Home`

Job-specific templates:

| Job | Additional Components |
|-----|----------------------|
| Guard | `Energy`, `AttackStats`, `AttackTimer(0)`, `Guard { town_idx }`, `PatrolRoute`, `OnDuty { ticks: 0 }` |
| Farmer | `Energy`, `Farmer { town_idx }`, `WorkPosition`, `GoingToWork` |
| Raider | `Energy`, `AttackStats`, `AttackTimer(0)`, `Stealer`, `FleeThreshold(0.50)`, `LeashRange(400)`, `WoundedThreshold(0.25)` |

GPU writes (via GPU_UPDATE_QUEUE, all jobs): `SetPosition`, `SetTarget` (= position), `SetColor` (job-based), `SetSpeed(100)`, `SetFaction`, `SetHealth(100)`

### reset_bevy_system
Checks `RESET_BEVY` flag. If set, despawns all entities with `NpcIndex`, clears `NpcEntityMap`, resets `NpcCount`.

## Known Issues / Limitations

- **npc_count never decreases**: High-water mark. 1000 spawns + 999 deaths = npc_count still 1000. Grid and buffers sized to high-water mark, not active count.
- **No spawn validation**: spawn_npc doesn't verify the town_idx is valid or that guard posts exist. Bad input silently creates a guard with no patrol route.
- **One-frame GPU delay**: GPU writes go through GPU_UPDATE_QUEUE, drained in `process()`. NPC won't render until the frame after Bevy processes the spawn. At 140fps this is invisible.

## Rating: 8/10

Single spawn path with job-as-template pattern. Slot index carried in message — fixes the previous dual-counter bug. All GPU writes routed through unified queue. GpuData resource eliminated.
