# Message Queues & Shared State

## Overview

Communication between Bevy systems and GPU compute uses two patterns:
- **Bevy Messages** (`#[derive(Message)]`) for high-frequency system-to-system communication
- **Static Mutex queues** for boundaries where Bevy's scheduler can't reach (GPU buffer sync, arrival detection)

All message types and statics are defined in `rust/src/messages.rs`. Bevy resources are in `rust/src/resources.rs`.

## Data Ownership

| Data | Owner | Direction | Notes |
|------|-------|-----------|-------|
| **GPU-Owned (Numeric/Physics)** ||||
| Positions | GPU | Bevy → GPU | Compute shader moves NPCs each frame |
| Targets | GPU | Bevy → GPU | Bevy decides destination, GPU interpolates movement |
| Factions | GPU | Write-once | Set at spawn (0=Villager, 1=Raider) |
| Combat targets | GPU | GPU → Bevy | Nearest enemy index or -1 (placeholder, not yet ported) |
| Colors | GPU | Bevy → GPU | Set at spawn, updated by steal/flee systems |
| Speeds | GPU | Write-once | Movement speed per NPC |
| **Bevy-Owned (Logical State)** ||||
| NpcIndex | Bevy | Internal | Links Bevy entity to GPU slot index |
| Job | Bevy | Internal | Guard, Farmer, Raider, Fighter — determines behavior |
| Energy | Bevy | Internal | Drives tired/rest decisions (drain/recover rates) |
| Health | **Both** | Bevy → GPU | Bevy authoritative, synced to GPU for targeting |
| State markers | Bevy | Internal | Dead, InCombat, Patrolling, OnDuty, Resting, Raiding, etc. |
| Config components | Bevy | Internal | FleeThreshold, LeashRange, WoundedThreshold, Stealer |
| AttackTimer | Bevy | Internal | Cooldown between attacks |
| AttackStats | Bevy | Internal | melee(range=150, speed=500) or ranged(range=300, speed=200) |
| PatrolRoute | Bevy | Internal | Guard post sequence for patrols |
| Home | Bevy | Internal | Rest location (bed or camp) |
| WorkPosition | Bevy | Internal | Farm location for farmers |

## Bevy Messages

Four message types used for intra-ECS communication:

| Message | Fields | Pattern |
|---------|--------|---------|
| SpawnNpcMsg | slot_idx, x, y, job, faction, town_idx, home_x/y, work_x/y, starting_post, attack_type | MessageWriter → MessageReader |
| SetTargetMsg | npc_index, x, y | MessageWriter → MessageReader |
| ArrivalMsg | npc_index | MessageWriter → MessageReader |
| DamageMsg | npc_index, amount | MessageWriter → MessageReader |
| GpuUpdateMsg | GpuUpdate enum (see below) | MessageWriter → collect_gpu_updates |

## GPU Update Messages

Systems emit `GpuUpdateMsg` via `MessageWriter<GpuUpdateMsg>`. The collector system `collect_gpu_updates` runs after Step::Behavior and drains all messages into `GPU_UPDATE_QUEUE` with a single Mutex lock. Then `populate_buffer_writes` (PostUpdate) drains the queue into `NpcBufferWrites` flat arrays for extraction to the render world.

| Variant | Fields | Producer Systems |
|---------|--------|------------------|
| SetTarget | idx, x, y | attack_system, decision_system, apply_targets_system |
| SetHealth | idx, health | spawn_npc_system, damage_system |
| SetFaction | idx, faction | spawn_npc_system |
| SetPosition | idx, x, y | spawn_npc_system |
| SetSpeed | idx, speed | spawn_npc_system |
| SetColor | idx, r, g, b, a | spawn_npc_system, behavior systems |
| ApplyDamage | idx, amount | damage_system |
| HideNpc | idx | death_cleanup_system |
| SetSpriteFrame | idx, col, row | spawn_npc_system |
| SetHealing | idx, healing | healing_system (visual only, not applied to GPU buffer) |
| SetCarriedItem | idx, item_id | arrival_system (visual only, not applied to GPU buffer) |

## Static Queues

| Static | Type | Writer | Reader |
|--------|------|--------|--------|
| GPU_UPDATE_QUEUE | `Mutex<Vec<GpuUpdate>>` | collect_gpu_updates | populate_buffer_writes |
| ARRIVAL_QUEUE | `Mutex<Vec<ArrivalMsg>>` | (future: GPU readback) | drain_arrival_queue |
| GPU_READ_STATE | `Mutex<GpuReadState>` | (not yet populated) | attack_system, apply_targets_system |
| GPU_DISPATCH_COUNT | `Mutex<usize>` | spawn_npc_system | (legacy, used for dispatch count) |
| GAME_CONFIG_STAGING | `Mutex<Option<GameConfig>>` | external config | drain_game_config |
| PROJ_GPU_UPDATE_QUEUE | `Mutex<Vec<ProjGpuUpdate>>` | (unused) | (unused) |
| FREE_PROJ_SLOTS | `Mutex<Vec<usize>>` | (unused) | (unused) |
| PERF_STATS | `Mutex<PerfStats>` | bevy_timer_end | (debug display) |

## GPU Read State

`GPU_READ_STATE` holds a snapshot of GPU output for Bevy systems to read. Currently **not populated** — no GPU→CPU readback is implemented. The struct exists with empty vecs.

| Field | Type | Intended Source | Consumers |
|-------|------|----------------|-----------|
| npc_count | usize | Dispatch count | apply_targets_system |
| positions | Vec\<f32\> | position_buffer readback | attack_system (range check) |
| combat_targets | Vec\<i32\> | combat_target_buffer readback | attack_system (target selection) |
| health | Vec\<f32\> | CPU cache | (available for queries) |
| factions | Vec\<i32\> | CPU cache | (available for queries) |

## Slot Management

`SlotAllocator` (Bevy Resource) manages NPC slot indices with an internal free list. Slots are allocated in `spawn_npc_system` and recycled in `death_cleanup_system`. LIFO reuse — most recently freed slot is allocated first.

`ProjSlotAllocator` (Bevy Resource) manages projectile slot indices. Currently unused — projectile compute is not yet ported.

## Bevy Resources for State

| Resource | Purpose | Writer | Reader |
|----------|---------|--------|--------|
| NpcMetaCache | Name, level, xp, trait, town, job per NPC | spawn_npc_system | UI queries |
| NpcEnergyCache | Energy level per NPC | energy_system | UI queries |
| NpcLogCache | Activity log per NPC | behavior systems | UI queries |
| NpcsByTownCache | NPC indices grouped by town | spawn/death systems | UI queries |
| PopulationStats | Alive/working/dead counts per job+town | spawn/death/state systems | UI queries |
| KillStats | guard_kills, villager_kills | death_cleanup_system | UI queries |
| SelectedNpc | Currently selected NPC index | (external input) | UI queries |
| FoodStorage | Per-town food counts | economy systems | economy systems |
| FoodEvents | Delivered/consumed food event logs | behavior systems | UI queries |
| GameConfig | World size, NPC counts, thresholds | drain_game_config (from staging) | spawn, economy |
| GameTime | Total hours, day/night, time scale, paused | game_time_system | behavior, economy |

NPC state is derived at query time via `derive_npc_state()` which checks ECS components (Dead, InCombat, Resting, etc.), not cached.

## State Constants

Used for UI display of NPC state as integers:

```
IDLE=0, WALKING=1, RESTING=2, WORKING=3, PATROLLING=4, ON_DUTY=5,
FIGHTING=6, RAIDING=7, RETURNING=8, RECOVERING=9, FLEEING=10,
GOING_TO_REST=11, GOING_TO_WORK=12
```

## Known Issues

- **GPU_READ_STATE not populated**: No GPU→CPU readback. Systems reading positions and combat_targets get empty vecs.
- **Projectile statics unused**: `FREE_PROJ_SLOTS` and `PROJ_GPU_UPDATE_QUEUE` exist but projectile compute is not ported.
- **Health dual ownership**: CPU-authoritative but synced to GPU. Could diverge if sync fails.
- **SetHealing/SetCarriedItem no-ops**: These GpuUpdate variants are matched but not applied to any GPU buffer.

## Rating: 7/10

MessageWriter pattern enables parallel system execution with a single mutex lock at frame end. Data ownership is clear (GPU owns physics, Bevy owns logic). Static queues are minimal — only used where Bevy's scheduler can't reach. Main weakness is the unpopulated GPU_READ_STATE and leftover projectile statics.
