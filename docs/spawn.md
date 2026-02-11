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
    ├─ Emit GPU updates: SetPosition, SetTarget,
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

Base components (all NPCs): `NpcIndex`, `Position`, `Job`, `TownId`, `Speed(resolved)`, `Health(resolved max_health)`, `CachedStats` (from `resolve_combat_stats()`), `Faction`, `Home`, `Personality`, `LastAteHour`, `Activity::default()`, `CombatState::default()`

Stats are resolved from `CombatConfig` resource via `resolve_combat_stats(job, attack_type, town_idx, level, personality, &config, &upgrades)`. The resolver applies job base stats × upgrade multipliers × trait multipliers × level multipliers. See `systems/stats.rs`. New NPCs spawn at level 0 (`level_from_xp(0) == 0`).

Job-specific templates:

| Job | Additional Components |
|-----|----------------------|
| Guard | `Energy`, `BaseAttackType::Melee`, `AttackTimer(0)`, `Guard`, `PatrolRoute`, `Activity::OnDuty { ticks_waiting: 0 }`, `EquippedWeapon`, `EquippedHelmet` |
| Farmer | `Energy`, `Farmer`, `WorkPosition`, `Activity::GoingToWork` |
| Raider | `Energy`, `BaseAttackType::Melee`, `AttackTimer(0)`, `Stealer`, `LeashRange(400)`, `EquippedWeapon` |
| Fighter | `BaseAttackType` (Melee or Ranged via attack_type), `AttackTimer(0)` |

GPU writes (all jobs): `SetPosition`, `SetTarget` (spawn position, or work position for farmers with valid work_x), `SetSpeed(100)`, `SetFaction`, `SetHealth(100)`, `SetSpriteFrame` (job-based sprite from constants.rs). Colors and equipment sprites are derived from ECS components by `sync_visual_sprites` (not sent as messages).

Sprite assignments: Farmer=(1,6), Guard=(0,11), Raider=(0,6), Fighter=(7,0)

### Personality Generation

Deterministic based on slot index (reproducible). Each NPC gets 0-2 traits from [Brave, Tough, Swift, Focused] with 30% chance each, magnitude 0.5-1.5.

### Name Generation

Deterministic: adjective + job noun. Adjective cycles through a 10-word list, noun cycles through a 5-word job-specific list. Slot index determines both.

### reset_bevy_system

Checks `ResetFlag`. If set, clears `NpcCount`, `NpcEntityMap`, `PopulationStats`, and resets `SlotAllocator`.

## reassign_npc_system (Step::Behavior)

Processes role reassignment requests (Farmer ↔ Guard) from `ReassignQueue` resource. The UI roster panel pushes `(slot, new_job)` tuples; this system drains the queue each frame.

**Farmer → Guard**: removes `Farmer`, `WorkPosition`, `AssignedFarm` (releases farm occupancy), inserts `Guard`, `BaseAttackType::Melee`, `AttackTimer(0)`, `EquippedWeapon`, `EquippedHelmet`, builds `PatrolRoute` via `build_patrol_route()`, sets `Activity::OnDuty`. Re-resolves `CachedStats` via `resolve_combat_stats()` using actual NPC level from `NpcMetaCache`. GPU: `SetSpriteFrame(SPRITE_GUARD)`.

**Town index convention**: NPCs store `TownId` as the WorldData index (0, 2, 4... for villagers; 1, 3, 5... for raiders). Buildings (farms, beds, guard posts) store `town_idx` as the pair index (0, 1, 2...). `build_patrol_route()` converts by dividing by 2: `npc_town_idx / 2` → pair index for guard post lookup.

**Guard → Farmer**: removes `Guard`, `BaseAttackType`, `AttackTimer`, `EquippedWeapon`, `EquippedHelmet`, `PatrolRoute`, inserts `Farmer`, finds nearest farm via `find_nearest_location()`, inserts `WorkPosition` + `Activity::GoingToWork`. Re-resolves `CachedStats` for new job using actual NPC level. GPU: `SetSpriteFrame(SPRITE_FARMER)`.

Both paths update `PopulationStats` (dec old job, inc new job), `NpcMetaCache.job`, and log to `CombatLog`.

Equipment visuals update automatically — `sync_visual_sprites` reads `EquippedWeapon`/`EquippedHelmet` ECS components each frame.

`ReassignQueue` is a plain `Resource` (not a Bevy Message) because the roster panel runs in `EguiPrimaryContextPass`, a separate schedule from `Update` where `MessageWriter` is unavailable.

## Known Issues

- **npc_count never decreases**: High-water mark. 1000 spawns + 999 deaths = npc_count still 1000. Buffers sized to peak, not active count.
- **No spawn validation**: Doesn't verify town_idx is valid or that guard posts exist. Bad input silently creates a guard with no patrol route.
- **One-frame GPU delay**: GPU writes go through message collection → populate_buffer_writes → extract → upload. NPC won't render until the frame after Bevy processes the spawn.

## Rating: 7/10

Single spawn path with job-as-template pattern. Slot recycling works. Personality and name generation are deterministic and reproducible. GPU writes properly batched through message system. Weaknesses: high-water mark dispatch count, no defensive validation on spawn parameters.
