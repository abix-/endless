# Message Queues & Shared State

## Overview

Communication between Bevy systems and GPU compute uses two patterns:
- **Bevy Messages** (`#[derive(Message)]`) for high-frequency system-to-system communication
- **Static Mutex queues** for boundaries where Bevy's scheduler can't reach (mainly GPU NPC/building sync and external staging)

All message types and statics are defined in `rust/src/messages.rs`. Bevy resources are in `rust/src/resources.rs`.

## Data Ownership & Authority

See [authority.md](authority.md) for the complete data ownership table, hard rules, and staleness budget. That document is the single source of truth for which systems own which data.

## Bevy Messages

### Core Messages

| Message | Fields | Pattern |
|---------|--------|---------|
| SpawnNpcMsg | slot_idx, x, y, job, faction, town_idx, home_x/y, work_x/y, starting_post, attack_type | MessageWriter → MessageReader |
| DamageMsg | target (EntityUid), amount (f32), attacker (i32, -1=tower/unknown), attacker_faction (i32) | process_proj_hits / attack_system → damage_system |
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
| PatrolSwapMsg | UI patrol reorder (slot_a, slot_b) | rebuild_patrol_routes_system |

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

`GpuUpdateMsg`: systems emit via `MessageWriter<GpuUpdateMsg>`. `populate_gpu_state` (PostUpdate) reads messages directly and applies updates to `EntityGpuState` flat arrays with per-buffer dirty flags (7 bools: `dirty_positions`, `dirty_targets`, `dirty_speeds`, `dirty_factions`, `dirty_healths`, `dirty_arrivals`, `dirty_flags`). All entity types (NPCs and buildings) share the same state — no `Bld*` routing. GPU-authoritative buffers (positions/arrivals) also track per-index dirty lists for sparse writes.

`ProjGpuUpdateMsg`: systems emit via `MessageWriter<ProjGpuUpdateMsg>` (attack/tower/hit processing). `populate_proj_buffer_writes` (PostUpdate) reads these messages directly and applies updates to `ProjBufferWrites` (spawn/deactivate dirty index sets). No static projectile queue remains.

`build_visual_upload` (chained after `populate_gpu_state`) updates dirty slots in persistent `NpcVisualUpload` buffers. Both `EntityGpuState` and `NpcVisualUpload` are read by `extract_npc_data` during Extract via `Extract<Res<T>>` (zero-clone) and written directly to GPU buffers.

| Variant | Fields | Producer Systems |
|---------|--------|------------------|
| SetTarget | idx, x, y | resolve_movement_system (sole emitter) |
| SetHealth | idx, health | spawn_npc_system, damage_system |
| SetFaction | idx, faction | spawn_npc_system |
| SetPosition | idx, x, y | spawn_npc_system |
| SetSpeed | idx, speed | spawn_npc_system |
| ApplyDamage | idx, amount | damage_system |
| HideNpc | idx | death_system |
| SetSpriteFrame | idx, col, row, atlas | spawn_npc_system (atlas: 0.0=character, 1.0=world) |
| SetDamageFlash | idx, intensity | damage_system (1.0 on hit, decays at 5.0/s in populate_gpu_state) |
| SetFlags | idx, flags | spawn_npc_system, building slot allocation (bit 0: combat scan enabled, bit 1: building) |
| Hide | idx | death_system (NPC and building branches) |
| MarkVisualDirty | idx | decision_system (activity visual key change), arrival_system (farm delivery), healing_system (healing flag toggle), death_system (loot drop activity) |

All variants are routed to `EntityGpuState` by `populate_gpu_state`. NPCs and buildings share the same unified slot namespace — building placement uses SetPosition/SetFaction/SetHealth/SetSpriteFrame/SetFlags with the building's unified slot.

Visual state updates are event-driven via `visual_dirty_indices` in `EntityGpuState`. `SetSpriteFrame`, `SetDamageFlash`, `Hide`, and `MarkVisualDirty` all push to this dirty list. Flash decay also marks slots dirty. `build_visual_upload` only updates dirty slots each frame (full rebuild on startup/load via `visual_full_rebuild` flag).

## Static Queues

| Static | Type | Writer | Reader |
|--------|------|--------|--------|
| GAME_CONFIG_STAGING | `Mutex<Option<GameConfig>>` | external config | drain_game_config |

GPU readback data is written directly to Bevy resources by `ReadbackComplete` observers.

## GPU Read State

`GpuReadState` (Bevy Resource, main-world only — no Clone, no extraction) holds GPU output for gameplay systems. Populated asynchronously by `ReadbackComplete` observers when Bevy's Readback system completes the GPU→CPU transfer. Not extracted to render world — nothing in render world reads it. `entity_count` set by `GpuSlotPool.count()` (not from readback — buffer is MAX-sized).

| Field | Type | Source | Consumers |
|-------|------|--------|-----------|
| entity_count | usize | GpuSlotPool.count() | gpu_position_readback |
| positions | Vec\<f32\> | ReadbackComplete (npc_positions buffer) | attack_system, healing_system, click_to_select_system |
| combat_targets | Vec\<i32\> | ReadbackComplete (combat_targets buffer) | attack_system, building_tower_system (candidate selection — re-validated via ECS) |
| health | Vec\<f32\> | ReadbackComplete (npc_health buffer) | advisory only — ECS Health is authoritative (see [authority.md](authority.md)) |
| factions | Vec\<i32\> | ReadbackComplete (npc_factions buffer, throttled) | advisory/debug only — throttled, never use as hard gate (see [authority.md](authority.md)) |
| threat_counts | Vec\<u32\> | ReadbackComplete (threat_counts buffer, throttled) | behavior_system (flee threshold calculations), AI threat checks |

## Slot Management

Two allocators share a `SlotPool` inner type (LIFO free list, high-water mark tracking) with type-safe Bevy Resource wrappers:

`GpuSlotPool` (NPC + building slots, max=MAX_ENTITIES=200K) wraps `SlotPool`. NPCs and buildings share one namespace — each entity's slot IS its GPU buffer index. Allocated in `spawn_npc_system` (NPCs) and `place_building_instance` (buildings), recycled in `death_system` (both branches).

`ProjSlotAllocator` (projectile slots, max=50K) manages projectile slot indices. Allocated in `attack_system`, recycled in `process_proj_hits`.

## Bevy Resources for State

| Resource | Purpose | Writer | Reader |
|----------|---------|--------|--------|
| NpcMetaCache | Name, level, xp, trait, town, job per NPC | spawn_npc_system | UI queries |
| NpcLogCache | Activity log per NPC | behavior systems | UI queries |
| NpcTargetThrashDebug | Target write diagnostics (reason-tagged + sink-level 1s window: `SinkTargetChanges/s`, `SinkPingPong/s`, `SinkTargetWrites/s`, `ReasonFlips/min`) | resolve_movement_system (sole recorder, via MovementIntents) | profiler tab, selected-NPC inspector |
| NpcsByTownCache | NPC indices grouped by town | spawn/death systems | UI queries |
| PopulationStats | Alive/working/dead counts per job+town | spawn/death/state systems | UI queries |
| KillStats | archer_kills, villager_kills | death_system | UI queries |
| SelectedNpc | Currently selected NPC index | (external input) | UI queries |
| FoodStorage | Per-town food counts | economy systems | economy systems |
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

- **Health dual ownership**: See [authority.md](authority.md) — CPU-authoritative, GPU mirror is advisory, bounded 1-frame divergence.
- **All large resources zero-clone**: GpuReadState no longer extracted, ProjPositionState + ProjBufferWrites use `Extract<Res<T>>` (zero-clone).
