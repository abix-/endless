# Spawn System

## Overview

NPCs are created through `SpawnNpcMsg` messages processed by `spawn_npc_system`. Slot allocation uses Bevy's `SlotAllocator` resource, which reuses dead NPC indices before allocating new ones. Job determines the component template at spawn time. All GPU writes go through `GpuUpdateMsg` messages — see [messages.md](messages.md).

## Data Flow

```
SpawnNpcMsg (via MessageWriter)
    │
    ▼ (Step::Spawn)
spawn_npc_system
    │
    ├─ Emit GPU updates: SetPosition, SetTarget, SetColor,
    │   SetSpeed, SetFaction, SetHealth, SetSpriteFrame
    │
    ├─ Update GPU_DISPATCH_COUNT (max slot + 1)
    │
    ├─ Spawn ECS entity with base + job-specific components
    │
    ├─ Update NpcEntityMap, NpcCount, PopulationStats, FactionStats
    │
    ├─ Initialize NpcMetaCache (name, level, trait)
    │
    └─ Add to NpcsByTownCache
```

## Slot Allocation

`SlotAllocator` (Bevy Resource, defined in `resources.rs`):

```rust
pub struct SlotAllocator {
    pub next: usize,      // High-water mark
    pub free: Vec<usize>, // Recycled slots from dead NPCs
}
```

`alloc()` pops from the free list first, falls back to incrementing `next`. `free()` pushes onto the free list. Both spawn (`spawn_npc_system`) and death (`death_cleanup_system`) use this resource. LIFO reuse — most recently freed slot is allocated first.

`GPU_DISPATCH_COUNT` (static Mutex) tracks how many NPCs have initialized GPU buffers. Updated by `spawn_npc_system` after emitting GPU writes, ensuring compute never dispatches NPCs with uninitialized buffers.

## Spawn Parameters

`SpawnNpcMsg` fields:

| Field | Type | Notes |
|-------|------|-------|
| slot_idx | usize | Pre-allocated via SlotAllocator |
| x, y | f32 | Spawn position |
| job | i32 | 0=Farmer, 1=Guard, 2=Raider, 3=Fighter |
| faction | i32 | 0=Villager, 1+=Raider camps |
| town_idx | i32 | Town association (-1 = none) |
| home_x, home_y | f32 | Home/camp position |
| work_x, work_y | f32 | Farm position (-1 = none, farmers only) |
| starting_post | i32 | Patrol start index (-1 = none, guards only) |
| attack_type | i32 | 0=melee, 1=ranged (fighters only) |

## spawn_npc_system

Base components (all NPCs): `NpcIndex`, `Position`, `Job`, `TownId`, `Speed(100)`, `Health(100)`, `MaxHealth(100)`, `Faction`, `Home`, `Personality`, `LastAteHour`

Job-specific templates:

| Job | Additional Components |
|-----|----------------------|
| Guard | `Energy`, `AttackStats::melee()`, `AttackTimer(0)`, `Guard`, `PatrolRoute`, `OnDuty { ticks_waiting: 0 }`, `EquippedWeapon`, `EquippedHelmet` |
| Farmer | `Energy`, `Farmer`, `WorkPosition`, `GoingToWork` (HasTarget auto-inserted via `#[require]`) |
| Raider | `Energy`, `AttackStats::melee()`, `AttackTimer(0)`, `Stealer`, `FleeThreshold(0.50)`, `LeashRange(400)`, `WoundedThreshold(0.25)`, `EquippedWeapon` |
| Fighter | `AttackStats` (melee or ranged via attack_type), `AttackTimer(0)` |

GPU writes (all jobs): `SetPosition`, `SetTarget` (spawn position, or work position for farmers with valid work_x), `SetColor` (job-based; raiders get per-faction color from 10-color palette), `SetSpeed(100)`, `SetFaction`, `SetHealth(100)`, `SetSpriteFrame` (job-based sprite from constants.rs), `SetEquipSprite` × 4 (clear all equipment layers to -1.0), then job-specific equipment: Guards get weapon (0,8) + helmet (7,9), Raiders get weapon (0,8)

Sprite assignments: Farmer=(1,6), Guard=(0,11), Raider=(0,6), Fighter=(7,0)

### Personality Generation

Deterministic based on slot index (reproducible). Each NPC gets 0-2 traits from [Brave, Tough, Swift, Focused] with 30% chance each, magnitude 0.5-1.5.

### Name Generation

Deterministic: adjective + job noun. Adjective cycles through a 10-word list, noun cycles through a 5-word job-specific list. Slot index determines both.

### reset_bevy_system

Checks `ResetFlag`. If set, clears `NpcCount`, `NpcEntityMap`, `PopulationStats`, and resets `SlotAllocator`.

## Known Issues

- **npc_count never decreases**: High-water mark. 1000 spawns + 999 deaths = npc_count still 1000. Buffers sized to peak, not active count.
- **No spawn validation**: Doesn't verify town_idx is valid or that guard posts exist. Bad input silently creates a guard with no patrol route.
- **One-frame GPU delay**: GPU writes go through message collection → populate_buffer_writes → extract → upload. NPC won't render until the frame after Bevy processes the spawn.

## Rating: 7/10

Single spawn path with job-as-template pattern. Slot recycling works. Personality and name generation are deterministic and reproducible. GPU writes properly batched through message system. Weaknesses: high-water mark dispatch count, no defensive validation on spawn parameters.
