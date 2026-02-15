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
    ├─ Spawn ECS entity with base + job-specific components
    │
    ├─ Update NpcEntityMap, PopulationStats, FactionStats
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

GPU dispatch count comes from `SlotAllocator.count()` (the high-water mark `next`). Dead NPC slots within this range are hidden via sentinel position (-9999) and culled by the renderer. No separate dispatch count resource needed.

## Spawn Parameters

`SpawnNpcMsg` fields:

| Field | Type | Notes |
|-------|------|-------|
| slot_idx | usize | Pre-allocated via SlotAllocator |
| x, y | f32 | Spawn position |
| job | i32 | 0=Farmer, 1=Archer, 2=Raider, 3=Fighter, 4=Miner |
| faction | i32 | 0=Player, 1+=AI settlements |
| town_idx | i32 | Town association (-1 = none) |
| home_x, home_y | f32 | Home/camp position |
| work_x, work_y | f32 | Farm position (-1 = none, farmers only) |
| starting_post | i32 | Patrol start index (-1 = none, archers only) |
| attack_type | i32 | 0=melee, 1=ranged (fighters only) |

## spawn_npc_system

Base components (all NPCs): `NpcIndex`, `Position`, `Job`, `TownId`, `Speed(resolved)`, `Health(resolved max_health)`, `CachedStats` (from `resolve_combat_stats()`), `Faction`, `Home`, `Personality`, `LastAteHour`, `Activity::default()`, `CombatState::default()`

Stats are resolved from `CombatConfig` resource via `resolve_combat_stats(job, attack_type, town_idx, level, personality, &config, &upgrades)`. The resolver applies job base stats × upgrade multipliers × trait multipliers × level multipliers. See `systems/stats.rs`. New NPCs spawn at level 0 (`level_from_xp(0) == 0`).

Job-specific templates:

| Job | Additional Components |
|-----|----------------------|
| Archer | `Energy`, `BaseAttackType::Melee`, `AttackTimer(0)`, `Archer`, `PatrolRoute`, `Activity::OnDuty { ticks_waiting: 0 }`, `EquippedWeapon`, `EquippedHelmet` |
| Farmer | `Energy`, `Farmer`, `WorkPosition`, `Activity::GoingToWork` |
| Miner | `Energy`, `Miner` |
| Raider | `Energy`, `BaseAttackType::Melee`, `AttackTimer(0)`, `Stealer`, `LeashRange(400)`, `EquippedWeapon` |
| Fighter | `BaseAttackType` (Melee or Ranged via attack_type), `AttackTimer(0)` |

GPU writes (all jobs): `SetPosition`, `SetTarget` (spawn position, or work position for farmers with valid work_x), `SetSpeed(100)`, `SetFaction`, `SetHealth(100)`, `SetSpriteFrame` (job-based sprite from constants.rs). Colors and equipment sprites are derived from ECS components by `sync_visual_sprites` (not sent as messages).

Sprite assignments: Farmer=(1,6), Archer=(0,11), Raider=(0,6), Fighter=(7,0), Miner=(1,6) (brown tint differentiates)

### Personality Generation

Deterministic based on slot index (reproducible). Each NPC gets 0-2 traits from [Brave, Tough, Swift, Focused] with 30% chance each, magnitude 0.5-1.5.

### Name Generation

Deterministic: adjective + job noun. Adjective cycles through a 10-word list, noun cycles through a 5-word job-specific list. Slot index determines both.

### reset_bevy_system

Checks `ResetFlag`. If set, clears `NpcEntityMap`, `PopulationStats`, and resets `SlotAllocator`.

**Town index convention**: NPCs and buildings both use direct WorldData town indices. Villager towns are at even indices (0, 2, 4...), raider camps at odd indices (1, 3, 5...). `build_patrol_route()` is `pub(crate)` and filters guard posts by `town_idx` directly (no `÷2` conversion).

## Building Spawners

All NPC population is building-driven: each **FarmerHome** supports 1 farmer, each **ArcherHome** supports 1 archer, each **MinerHome** supports 1 miner, and each **Tent** supports 1 raider. At game startup, `game_startup_system` builds `SpawnerState` from `WorldData.farmer_homes` + `WorldData.archer_homes` + `WorldData.miner_homes` + `WorldData.tents` and spawns 1 NPC per entry via `SlotAllocator` + `SpawnNpcMsg`. Menu sliders control how many FarmerHomes/ArcherHomes/MinerHomes/Tents world gen places.

When an NPC dies, `spawner_respawn_system` (hourly, Step::Behavior) detects the death via `NpcEntityMap` lookup, starts a 12-hour respawn timer, and spawns a replacement when it expires. Building spawners at runtime via the build menu pushes new `SpawnerEntry` with `respawn_timer: 0.0` — the system spawns the NPC on the next hourly tick. Camp grids only allow Tent placement; villager grids allow Farm/GuardPost/FarmerHome/ArcherHome/MinerHome.

Destroying a spawner building tombstones the `SpawnerEntry` (position.x = -99999). The linked NPC survives but won't respawn if killed.

All spawners set home to building position (FarmerHome/ArcherHome/MinerHome/Tent). All spawner types set faction from `world_data.towns[town_idx].faction` (player towns = 0, AI settlements = unique 1+).

## Known Issues

- **No spawn validation**: Doesn't verify town_idx is valid or that guard posts exist. Bad input silently creates an archer with no patrol route.
- **One-frame GPU delay**: GPU writes go through message collection → populate_buffer_writes → extract → upload. NPC won't render until the frame after Bevy processes the spawn.

## Rating: 8/10

Single spawn path with job-as-template pattern. Slot recycling via `SlotAllocator` — single source of truth for NPC counting (`count()` for GPU dispatch, `alive()` for UI). Personality and name generation are deterministic and reproducible. GPU writes properly batched through message system.
