# Message Queues & Shared State

## Overview

Communication between Bevy systems and GPU compute uses two patterns:
- **Bevy Messages** (`#[derive(Message)]`) for high-frequency system-to-system communication
- **Static Mutex queues** for boundaries where Bevy's scheduler can't reach (GPU buffer sync, arrival detection)

All message types and statics are defined in `rust/src/messages.rs`. Bevy resources are in `rust/src/resources.rs`.

## Data Ownership & Authority Model

Each piece of NPC data has exactly one authoritative owner. Readers on the other side tolerate 1-frame staleness.

**Staleness budget**: 1 frame = 16ms @ 60fps. NPC max speed 100px/s × 0.016s = 1.6px drift. All thresholds are designed for this: arrival=8px, targeting=300px, separation=20px.

**Anti-pattern**: no system may read from GPU readback AND write back to the same GPU field in the same frame. That creates a feedback loop where 1-frame delay compounds into oscillation.

| Data | Authority | Direction | Notes |
|------|-----------|-----------|-------|
| **GPU-Authoritative** (written by compute shader, read back 1 frame later) ||||
| Positions | GPU | GPU → CPU | Compute shader moves NPCs; Bevy async Readback → GpuReadState |
| Spatial grid | GPU | Internal | Built each frame (clear → insert → query). Not read back. |
| Combat targets | GPU | GPU → CPU | Nearest enemy index via grid neighbor search; readback to GpuReadState |
| Arrivals | CPU | Internal | `HasTarget` + `gpu_position_readback` distance check → `AtDestination` |
| **CPU-Authoritative** (written by ECS systems, uploaded to GPU next frame) ||||
| Health | CPU | CPU → GPU | damage_system/healing_system write; uploaded for GPU targeting threshold |
| Targets/Goals | CPU | CPU → GPU | decision_system/attack_system set destination; GPU interpolates movement |
| Factions | CPU | CPU → GPU | Set at spawn, never changes (0=Villager, 1+=Raider camps) |
| Speeds | CPU | CPU → GPU | Set at spawn, modified by starvation_system |
| **CPU-Only** (never sent to GPU) ||||
| NpcIndex | CPU | Internal | Links Bevy entity to GPU slot index |
| Job | CPU | Internal | Archer, Farmer, Raider, Fighter, Miner — determines behavior |
| Energy | CPU | Internal | Drives tired/rest decisions (drain/recover rates) |
| State markers | CPU | Internal | Dead, InCombat, Patrolling, OnDuty, Resting, Raiding, etc. |
| Config components | CPU | Internal | FleeThreshold, LeashRange, WoundedThreshold, Stealer |
| AttackTimer | CPU | Internal | Cooldown between attacks |
| AttackStats | CPU | Internal | melee(range=150, speed=500) or ranged(range=300, speed=200) |
| PatrolRoute | CPU | Internal | Guard post sequence for patrols |
| Home | CPU | Internal | Rest location (bed or camp) |
| WorkPosition | CPU | Internal | Farm location for farmers |
| **Render-Only** (uploaded to NPC visual/equip storage buffers, never in compute shader) ||||
| Sprite indices | NpcGpuState | CPU → NpcVisualUpload → NpcVisualBuffers | Atlas col/row per NPC; packed into visual storage buffer [f32;8] by build_visual_upload |
| Colors | ECS → NpcVisualUpload | CPU → NpcVisualBuffers | RGBA tint from Faction/Job; packed into visual storage buffer [f32;8] by build_visual_upload |
| Equipment sprites | ECS → NpcVisualUpload | CPU → NpcVisualBuffers | Per-layer col/row (armor/helmet/weapon/item/status/healing); -1.0 sentinel = unequipped/inactive. Derived by `build_visual_upload` from ECS components each frame. |

## Bevy Messages

Three message types used for intra-ECS communication:

| Message | Fields | Pattern |
|---------|--------|---------|
| SpawnNpcMsg | slot_idx, x, y, job, faction, town_idx, home_x/y, work_x/y, starting_post, attack_type | MessageWriter → MessageReader |
| DamageMsg | npc_index, amount, attacker (i32, -1=no attacker) | MessageWriter → MessageReader |
| BuildingDamageMsg | kind (BuildingKind), index (usize), amount (f32), attacker_faction (i32) | attack_system → building_damage_system |
| GpuUpdateMsg | GpuUpdate enum (see below) | MessageWriter → collect_gpu_updates |
| ReassignMsg | npc_index, new_job | Defined but unused (placeholder for future role reassignment) |

## GPU Update Messages

Systems emit `GpuUpdateMsg` via `MessageWriter<GpuUpdateMsg>`. The collector system `collect_gpu_updates` runs after Step::Behavior and drains all messages into `GPU_UPDATE_QUEUE` with a single Mutex lock. Then `populate_gpu_state` (PostUpdate) drains the queue into `NpcGpuState` flat arrays with per-field dirty tracking. `build_visual_upload` (chained after) packs ECS visual data into `NpcVisualUpload`. Both are read by `extract_npc_data` during Extract via `Extract<Res<T>>` (zero-clone) and written directly to GPU buffers.

| Variant | Fields | Producer Systems |
|---------|--------|------------------|
| SetTarget | idx, x, y | attack_system, decision_system |
| SetHealth | idx, health | spawn_npc_system, damage_system |
| SetFaction | idx, faction | spawn_npc_system |
| SetPosition | idx, x, y | spawn_npc_system |
| SetSpeed | idx, speed | spawn_npc_system |
| ApplyDamage | idx, amount | damage_system |
| HideNpc | idx | death_cleanup_system |
| SetSpriteFrame | idx, col, row, atlas | spawn_npc_system (atlas: 0.0=character, 1.0=world) |
| SetDamageFlash | idx, intensity | damage_system (1.0 on hit, decays at 5.0/s in populate_gpu_state) |
| SetFlags | idx, flags | spawn_npc_system, building slot allocation (bit 0: combat scan enabled) |

**Removed (replaced by `build_visual_upload`):** SetColor, SetHealing, SetSleeping, SetEquipSprite — visual state is now derived from ECS components each frame by `build_visual_upload` (see [gpu-compute.md](gpu-compute.md)).

## Static Queues

| Static | Type | Writer | Reader |
|--------|------|--------|--------|
| GPU_UPDATE_QUEUE | `Mutex<Vec<GpuUpdate>>` | collect_gpu_updates | populate_gpu_state |
| GAME_CONFIG_STAGING | `Mutex<Option<GameConfig>>` | external config | drain_game_config |
| PROJ_GPU_UPDATE_QUEUE | `Mutex<Vec<ProjGpuUpdate>>` | attack_system, waypoint_attack_system | populate_proj_buffer_writes |
| FREE_PROJ_SLOTS | `Mutex<Vec<usize>>` | (unused) | (unused) |

GPU readback statics (`GPU_READ_STATE`, `PROJ_HIT_STATE`, `PROJ_POSITION_STATE`) deleted — replaced by Bevy `ReadbackComplete` observers writing directly to Bevy resources.

## GPU Read State

`GpuReadState` (Bevy Resource, main-world only — no Clone, no extraction) holds GPU output for gameplay systems. Populated asynchronously by `ReadbackComplete` observers when Bevy's Readback system completes the GPU→CPU transfer. Not extracted to render world — nothing in render world reads it. `npc_count` set by `SlotAllocator.count()` (not from readback — buffer is MAX-sized).

| Field | Type | Source | Consumers |
|-------|------|--------|-----------|
| npc_count | usize | SlotAllocator.count() | gpu_position_readback |
| positions | Vec\<f32\> | ReadbackComplete (npc_positions buffer) | attack_system, healing_system, click_to_select_system |
| combat_targets | Vec\<i32\> | ReadbackComplete (combat_targets buffer) | attack_system (target selection) |
| health | Vec\<f32\> | CPU cache | (available for queries) |
| factions | Vec\<i32\> | CPU cache | (available for queries) |

## Slot Management

`SlotAllocator` (Bevy Resource) manages NPC slot indices with an internal free list. Slots are allocated in `spawn_npc_system` and recycled in `death_cleanup_system`. LIFO reuse — most recently freed slot is allocated first.

`ProjSlotAllocator` (Bevy Resource) manages projectile slot indices with an internal free list. Slots are allocated in `attack_system` and recycled in `process_proj_hits` (on collision or expiry).

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

- **Health dual ownership**: CPU-authoritative but synced to GPU for targeting. If upload fails or is delayed, GPU targets based on stale health. Bounded to 1 frame.
- **All large resources zero-clone**: GpuReadState no longer extracted, ProjPositionState + ProjBufferWrites use `Extract<Res<T>>` (zero-clone).
