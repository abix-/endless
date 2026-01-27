# Spawn System

## Overview

NPCs are created through a dual-write pattern: GDScript spawn methods write GPU buffers directly (for immediate rendering) and push messages to Bevy queues (for entity creation). Slot allocation reuses dead NPC indices via FREE_SLOTS before allocating new ones.

## Data Flow

```
GDScript: spawn_guard(x, y, town, home_x, home_y)
│
├─ allocate_slot()
│   ├─ Try FREE_SLOTS.pop() (recycled from dead NPC)
│   └─ Else npc_count++ from GPU_READ_STATE
│
├─ Write GPU buffers directly:
│   position, target, color, speed, arrival,
│   backoff, faction, health
│
└─ Push to GUARD_QUEUE
         │
         ▼ (next frame, Bevy Step::Drain)
   drain_guard_queue → SpawnGuardMsg
         │
         ▼ (Step::Spawn)
   spawn_guard_system
   └─ Create entity with: NpcIndex, Job::Guard, Speed,
      Health, Faction, AttackStats, AttackTimer,
      PatrolRoute, OnDuty, Home, HasTarget
```

## Slot Allocation

```rust
fn allocate_slot() -> Option<usize> {
    // 1. Reuse dead NPC slot
    if let Some(recycled) = FREE_SLOTS.pop() {
        return Some(recycled);
    }
    // 2. Allocate new slot (if under MAX_NPC_COUNT)
    if npc_count < MAX_NPC_COUNT {
        let idx = npc_count;
        npc_count += 1;
        return Some(idx);
    }
    None // At capacity
}
```

`npc_count` is a high-water mark — it only grows. Dead slots are recycled through `FREE_SLOTS` but don't decrement the count.

## GDScript Spawn API

| Method | GPU Buffers Written | Bevy Queue | Faction | Health |
|--------|-------------------|------------|---------|--------|
| `spawn_npc(x, y, job)` | position, target, color, speed, arrival, backoff | SPAWN_QUEUE | - | - |
| `spawn_guard(x, y, town, home_x, home_y)` | + faction(0), health(100) | GUARD_QUEUE | Villager | 100 |
| `spawn_guard_at_post(x, y, town, home_x, home_y, post)` | + faction(0), health(100), arrival(1) | GUARD_QUEUE | Villager | 100 |
| `spawn_farmer(x, y, town, home_x, home_y, work_x, work_y)` | + faction(0), health(100) | FARMER_QUEUE | Villager | 100 |
| `spawn_raider(x, y, camp_x, camp_y)` | + faction(1), health(100) | RAIDER_QUEUE | Raider | 100 |

All methods also write color (job-based RGBA) and speed (100.0).

## Bevy Spawn Systems

### spawn_npc_system
Creates: `NpcIndex`, `Job`, `Speed(100)`, `Health(100)`, `Energy(100)`

### spawn_guard_system
Creates: `NpcIndex`, `Job::Guard`, `Speed(100)`, `Health(100)`, `Faction::Villager`, `AttackStats`, `AttackTimer(0)`, `Energy(100)`, `PatrolRoute` (built from WorldData guard posts), `OnDuty { ticks: 0 }`, `Home`, `HasTarget`

### spawn_farmer_system
Creates: `NpcIndex`, `Job::Farmer`, `Speed(100)`, `Health(100)`, `Faction::Villager`, `Energy(100)`, `WorkPosition`, `Home`, `GoingToWork`, `HasTarget`

### spawn_raider_system
Creates: `NpcIndex`, `Job::Raider`, `Speed(100)`, `Health(100)`, `Faction::Raider`, `AttackStats`, `AttackTimer(0)`, `Energy(100)`, `Home` (camp position)

### reset_bevy_system
Checks `RESET_BEVY` flag. If set, despawns all entities with `NpcIndex`, clears `NpcEntityMap`, resets `NpcCount`.

## Dual-Write Pattern

Spawn methods write to **both** GPU buffers and Bevy queues because:

1. **GPU buffers need data immediately** — the compute shader dispatches this frame and needs valid position/health/faction data for the new NPC. Writing directly via `buffer_update()` ensures it's there.

2. **Bevy needs entities for game logic** — state machines, patrol routes, energy, combat all operate on Bevy entities with components. These are created when the queue is drained next frame.

3. **One-frame gap is safe** — the NPC renders correctly on frame 1 (GPU has data). Bevy creates the entity on frame 2 (drain + spawn systems). During that gap, the NPC moves via GPU physics but has no Bevy logic. This is invisible at 140fps.

## Known Issues / Limitations

- **BUG: Dual npc_count counters**: `allocate_slot()` increments `GPU_READ_STATE.npc_count`, but Bevy spawn systems use `GpuData.npc_count` (line 47 of spawn.rs). These are separate counters that can drift. More critically, `allocate_slot()` may return a recycled FREE_SLOTS index, but the Bevy spawn system always uses `gpu_data.npc_count` as the index — it has no way to receive the recycled slot index. The spawn message types (SpawnNpcMsg, SpawnGuardMsg, etc.) don't carry the allocated slot index. **This means Bevy creates the entity with the wrong NpcIndex when a recycled slot is used.** The GPU has the NPC at the recycled slot, but Bevy thinks it's at a new slot. Combat targeting, damage, and death will reference the wrong entity.
- **npc_count never decreases**: High-water mark. 1000 spawns + 999 deaths = npc_count still 1000. Grid and buffers sized to high-water mark, not active count.
- **No spawn validation**: spawn_guard doesn't verify the town_idx is valid or that guard posts exist. Bad input silently creates a guard with no patrol route.
- **Duplicate GPU writes**: spawn_guard writes health to GPU directly AND Bevy's damage_system writes health via GpuUpdate::SetHealth. The direct write wins initially; GpuUpdate wins on subsequent changes. Not a conflict but two paths to the same buffer.

## Rating: 6/10

The dual-write pattern is pragmatic but the dual npc_count counters and missing slot index in spawn messages is a real bug. Slot recycling works on the GPU side but Bevy assigns wrong indices for recycled slots. Needs fix: add allocated index to all spawn message types.
