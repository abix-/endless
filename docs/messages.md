# Message Queues & Shared State

## Overview

Communication between Bevy systems and GPU compute uses two patterns:
- **Bevy Messages** (`#[derive(Message)]`) for high-frequency system-to-system communication
- **Static Mutex queues** for boundaries where Bevy's scheduler can't reach (mainly GPU NPC/building sync and external staging)

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
| Factions | CPU | CPU → GPU | Set at spawn, never changes (0=Villager, 1+=Raider towns) |
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
| Home | CPU | Internal | Rest location (spawner building) |
| WorkPosition | CPU | Internal | Farm location for farmers |
| **Render-Only** (uploaded to NPC visual/equip storage buffers, never in compute shader) ||||
| Sprite indices | NpcGpuState | CPU → NpcVisualUpload → NpcVisualBuffers | Atlas col/row per NPC; packed into visual storage buffer [f32;8] by build_visual_upload |
| Colors | ECS → NpcVisualUpload | CPU → NpcVisualBuffers | RGBA tint from Faction/Job; packed into visual storage buffer [f32;8] by build_visual_upload |
| Equipment sprites | ECS → NpcVisualUpload | CPU → NpcVisualBuffers | Per-layer col/row (armor/helmet/weapon/item/status/healing); -1.0 sentinel = unequipped/inactive. Derived by `build_visual_upload` from ECS components each frame. |

## Bevy Messages

### Core Messages

| Message | Fields | Pattern |
|---------|--------|---------|
| SpawnNpcMsg | slot_idx, x, y, job, faction, town_idx, home_x/y, work_x/y, starting_post, attack_type | MessageWriter → MessageReader |
| DamageMsg | entity_idx (usize), amount (f32), attacker (i32, -1=tower/unknown), attacker_faction (i32) | process_proj_hits / attack_system → damage_system |
| BuildingDeathMsg | kind (BuildingKind), index (usize), bld_slot (usize), attacker (i32), attacker_faction (i32) | damage_system → building_death_system |
| GpuUpdateMsg | GpuUpdate enum (see below) | MessageWriter → populate_gpu_state |
| CombatLogMsg | kind, faction, day, hour, minute, message, location | 18+ writers → drain_combat_log |
| SaveGameMsg | none | save_load_input_system → save_game_system |
| LoadGameMsg | none | save_load_input_system → load_game_system |
| SelectFactionMsg | faction (i32) | click_to_select_system/game_hud → left_panel_system |
| ReassignMsg | npc_index, new_job | Defined but unused (placeholder for future role reassignment) |

### Dirty Signal Messages

Individual message types replace the old `DirtyFlags` resource. Each signal is independent — systems that care about mining don't block systems that care about squads. `DirtyWriters<'w>` SystemParam bundles all writers for convenience.

| Message | Trigger | Consumer |
|---------|---------|----------|
| BuildingGridDirtyMsg | Building placed/destroyed | rebuild_building_grid_system, populate_tile_flags |
| TerrainDirtyMsg | Terrain biome changed (expansion/load/reset) | sync_terrain_tilemap |
| PatrolsDirtyMsg | Waypoint built/destroyed/reordered | rebuild_patrol_routes_system |
| PatrolPerimeterDirtyMsg | Building changed (waypoints, homes) | sync_patrol_perimeter_system |
| HealingZonesDirtyMsg | Level-up (heal stats changed) | update_healing_zone_cache |
| SquadsDirtyMsg | NPC death/spawn, UI assign/dismiss | squad_cleanup_system |
| MiningDirtyMsg | Miner home built/destroyed, mining policy change | mining_policy_system |
| AiSquadsDirtyMsg | Military spawn/death, building changes | ai_squad_commander_system |
| PatrolSwapMsg | UI patrol reorder (a, b indices) | rebuild_patrol_routes_system |

`DirtyWriters` provides `mark_building_changed(kind)` helper that emits the right combo of signals for build/destroy events, and `emit_all()` for startup/reset to trigger first-frame rebuilds.

**Drain pattern for Reader/Writer conflicts:** When a system needs both `MessageReader<T>` (requires `Res<Messages<T>>`) and `MessageWriter<T>` (requires `ResMut<Messages<T>>`) for the same message type — e.g., because it reads dirty signals AND writes them via `DirtyWriters` in `WorldState` — Bevy's scheduler panics (B0002). The fix: split the read into a tiny drain system that runs `.before()` the main system, writing to an intermediate `Resource` flag. Two drain systems exist:
- `ai_dirty_drain_system` → `AiSnapshotDirty` (drains BuildingGridDirtyMsg + MiningDirtyMsg + PatrolPerimeterDirtyMsg for `ai_decision_system`)
- `perimeter_dirty_drain_system` → `PerimeterSyncDirty` (drains PatrolPerimeterDirtyMsg for `sync_patrol_perimeter_system`)

Both can drain the same message type because each `MessageReader` has an independent cursor.

### CombatLogMsg

Replaces direct `ResMut<CombatLog>` writes from 18+ systems. Writers emit `CombatLogMsg` via `MessageWriter` (non-exclusive — all writers can run in parallel). `drain_combat_log` system (Step::Drain) collects messages into the `CombatLog` resource for UI display.


## Lifecycle Helpers

Startup/load paths are centralized to prevent drift:
- `world::materialize_generated_world`: shared generated-world bootstrap (writes `SpawnNpcMsg` and spawns building ECS entities).
- `save::restore_world_from_save`: shared save-restore pipeline used by both menu load and in-game quickload.

## GPU Update Messages

`GpuUpdateMsg`: systems emit via `MessageWriter<GpuUpdateMsg>`. `populate_gpu_state` (PostUpdate) reads messages directly and routes updates: NPC variants go to `NpcGpuState` flat arrays with per-buffer dirty flags (7 bools: `dirty_positions`, `dirty_targets`, `dirty_speeds`, `dirty_factions`, `dirty_healths`, `dirty_arrivals`, `dirty_flags`); `Bld*` variants go to `BuildingGpuState` (positions, factions, healths, sprite_indices, flash_values, flags + dirty tracking). GPU-authoritative buffers (positions/arrivals) also track per-index dirty lists for sparse writes.

`ProjGpuUpdateMsg`: systems emit via `MessageWriter<ProjGpuUpdateMsg>` (attack/tower/hit processing). `populate_proj_buffer_writes` (PostUpdate) reads these messages directly and applies updates to `ProjBufferWrites` (spawn/deactivate dirty index sets). No static projectile queue remains.

`build_visual_upload` (chained after `populate_gpu_state`) packs ECS visual data into `NpcVisualUpload`. Both `NpcGpuState` and `NpcVisualUpload` are read by `extract_npc_data` during Extract via `Extract<Res<T>>` (zero-clone) and written directly to GPU buffers.

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
| BldSetPosition | idx, x, y | place_building_instance (building GPU state) |
| BldSetFaction | idx, faction | place_building_instance |
| BldSetHealth | idx, health | place_building_instance, damage_system, healing_system |
| BldSetSpriteFrame | idx, col, row, atlas | place_building_instance |
| BldSetFlags | idx, flags | place_building_instance |
| BldSetDamageFlash | idx, intensity | damage_system (1.0 on hit, decays at 5.0/s in populate_gpu_state) |
| BldHide | idx | death_cleanup_system (building branch) |

`Bld*` variants are routed to `BuildingGpuState` by `populate_gpu_state`. NPC variants are routed to `NpcGpuState`.

**Removed (replaced by `build_visual_upload`):** SetColor, SetHealing, SetSleeping, SetEquipSprite — visual state is now derived from ECS components each frame by `build_visual_upload` (see [gpu-compute.md](gpu-compute.md)).

## Static Queues

| Static | Type | Writer | Reader |
|--------|------|--------|--------|
| GAME_CONFIG_STAGING | `Mutex<Option<GameConfig>>` | external config | drain_game_config |

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

All three allocators share a `SlotPool` inner type (LIFO free list, high-water mark tracking) with type-safe Bevy Resource wrappers:

`SlotAllocator` (NPC slots, max=100K) wraps `SlotPool`. Allocated in `spawn_npc_system`, recycled in `death_cleanup_system`.

`BuildingSlots` (building slots, max=5K) wraps `SlotPool`. Allocated in `place_building_instance`, recycled in `death_cleanup_system` (building branch). Separate from NPC slots to prevent slot collisions.

`ProjSlotAllocator` (projectile slots, max=50K) manages projectile slot indices. Allocated in `attack_system`, recycled in `process_proj_hits`.

## Bevy Resources for State

| Resource | Purpose | Writer | Reader |
|----------|---------|--------|--------|
| NpcMetaCache | Name, level, xp, trait, town, job per NPC | spawn_npc_system | UI queries |
| NpcEnergyCache | Energy level per NPC | energy_system | UI queries |
| NpcLogCache | Activity log per NPC | behavior systems | UI queries |
| NpcTargetThrashDebug | Target write diagnostics (reason-tagged + sink-level 1s window: `SinkTargetChanges/s`, `SinkPingPong/s`, `SinkTargetWrites/s`, `ReasonFlips/min`) | behavior/combat writers + sink recorder in `populate_gpu_state` | profiler tab, selected-NPC inspector |
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
