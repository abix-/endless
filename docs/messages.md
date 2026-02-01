# Message Queues & Shared State

## Overview

Communication between Godot (GDScript), Bevy ECS, and GPU compute uses a hybrid architecture:
- **Lock-free channels** for cross-thread message passing (GodotToBevy, BevyToGodot)
- **Static Mutex** for GPU boundary state and UI queries
- **Bevy Messages** for high-frequency internal communication

Channels defined in `rust/src/channels.rs`. Statics defined in `rust/src/messages.rs`.

## Data Ownership

| Data | Owner | Direction | Notes |
|------|-------|-----------|-------|
| **GPU-Owned (Numeric/Physics)** ||||
| Positions | GPU | GPU → Bevy | Compute shader moves NPCs each frame |
| Targets | GPU | Bevy → GPU | Bevy decides destination, GPU interpolates movement |
| Factions | GPU | Write-once | Set at spawn (0=Villager, 1=Raider) |
| Combat targets | GPU | GPU → Bevy | GPU finds nearest enemy within 300px |
| Colors | GPU | Bevy → GPU | Set at spawn, updated by steal/flee systems |
| Speeds | GPU | Write-once | Movement speed per NPC |
| **Bevy-Owned (Logical State)** ||||
| NpcIndex | Bevy | Internal | Links Bevy entity to GPU slot index |
| Job | Bevy | Internal | Guard, Farmer, Raider, Fighter - determines behavior |
| Energy | Bevy | Internal | Drives tired/rest decisions (drain/recover rates) |
| Health | **Both** | Bevy → GPU | Bevy authoritative, synced to GPU for targeting |
| State markers | Bevy | Internal | Dead, InCombat, Patrolling, OnDuty, Resting, Raiding, Returning, Recovering, etc. |
| Config components | Bevy | Internal | FleeThreshold, LeashRange, WoundedThreshold, Stealer |
| AttackTimer | Bevy | Internal | Cooldown between attacks |
| AttackStats | Bevy | Internal | melee(range=150, speed=500) or ranged(range=300, speed=200) |
| PatrolRoute | Bevy | Internal | Guard post sequence for patrols |
| Home | Bevy | Internal | Rest location (bed or camp) |
| WorkPosition | Bevy | Internal | Farm location for farmers |

## GodotToBevy Channel (lib.rs → Bevy)

Lock-free crossbeam channel replaces SPAWN_QUEUE, TARGET_QUEUE, DAMAGE_QUEUE, RESET_BEVY.

| Message | Fields | Producer | Consumer |
|---------|--------|----------|----------|
| SpawnNpc | slot_idx, x, y, job, faction, town_idx, home_x/y, work_x/y, starting_post, attack_type | spawn_npc() | godot_to_bevy_read → SpawnNpcMsg |
| SetTarget | slot, x, y | set_target() | godot_to_bevy_read → SetTargetMsg |
| ApplyDamage | slot, amount | apply_damage() | godot_to_bevy_read → DamageMsg |
| SelectNpc | slot | set_selected_npc() | godot_to_bevy_read → SelectedNpc resource |
| Reset | - | reset() | godot_to_bevy_read → ResetFlag resource |
| SetPaused | bool | set_paused() | godot_to_bevy_read → GameTime.paused |
| SetTimeScale | f32 | set_time_scale() | godot_to_bevy_read → GameTime.time_scale |
| PlayerClick | x, y | (future) | godot_to_bevy_read → (unimplemented) |

## BevyToGodot Channel (Bevy → lib.rs)

Lock-free crossbeam channel replaces PROJECTILE_FIRE_QUEUE.

| Message | Fields | Producer | Consumer |
|---------|--------|----------|----------|
| FireProjectile | from_x/y, to_x/y, speed, damage, faction, shooter, lifetime | attack_system | process() → upload_projectile() |
| SpawnView | slot, job, x, y | (future) | (future Godot visual creation) |
| DespawnView | slot | (future) | (future Godot visual removal) |
| SyncTransform | slot, x, y | (unused - GPU renders directly) | - |
| SyncHealth | slot, hp, max_hp | (future) | (future Godot health bars) |
| SyncColor | slot, r, g, b, a | (future) | (future Godot color sync) |
| SyncSprite | slot, col, row | (future) | (future Godot sprite sync) |

## Remaining Static Queues

| Queue | Message Type | Fields | Producer | Consumer |
|-------|-------------|--------|----------|----------|
| ARRIVAL_QUEUE | ArrivalMsg | npc_index | process() arrival detection | drain_arrival_queue → handle_arrival_system |

## GPU Update Messages

Systems emit `GpuUpdateMsg` messages via `MessageWriter<GpuUpdateMsg>`. A collector system (`collect_gpu_updates`) runs at end of frame and drains all messages into `GPU_UPDATE_QUEUE` with a single Mutex lock. `process()` then drains the static queue to write GPU buffers.

**Why Messages, not Events?** godot-bevy distinguishes:
- **Messages** (`#[derive(Message)]`) — high-frequency batch operations (every frame)
- **Observers** (`#[derive(Event)]`) — infrequent reactive events (button presses, signals)

GPU updates happen every frame from 10+ systems → Message pattern is correct.

```rust
#[derive(Message, Clone)]
pub struct GpuUpdateMsg(pub GpuUpdate);
```

| Variant | Fields | Producer Systems | GPU Buffer Updated |
|---------|--------|------------------|-------------------|
| SetTarget | idx, x, y | attack_system, tired_system, resume_patrol_system, resume_work_system, patrol_system, raider_arrival_system, flee_system, leash_system, npc_decision_system | target_buffer, arrival_buffer, backoff_buffer |
| SetHealth | idx, health | spawn_npc_system, damage_system | health_buffer |
| SetFaction | idx, faction | spawn_npc_system | faction_buffer |
| SetPosition | idx, x, y | spawn_npc_system | position_buffer |
| SetSpeed | idx, speed | spawn_npc_system | speed_buffer |
| SetColor | idx, r, g, b, a | spawn_npc_system, raider_arrival_system, flee_system | color_buffer |
| ApplyDamage | idx, amount | (unused — damage goes through Bevy) | health_buffer |
| HideNpc | idx | death_cleanup_system | position, target, arrival, health (full slot cleanup) |
| SetSpriteFrame | idx, col, row | spawn_npc_system | sprite_frame_buffer |
| SetHealing | idx, healing: bool | healing_system | (visual flag) |
| SetCarriedItem | idx, item_id: u8 | arrival_system | carried_items buffer |

**Static Queue (Bevy↔GPU boundary):** `GPU_UPDATE_QUEUE: Mutex<Vec<GpuUpdate>>` — written by `collect_gpu_updates`, drained by `process()`.

## GPU Read State

`GPU_READ_STATE: Mutex<GpuReadState>` — snapshot of GPU output, updated after each dispatch in `process()`. Read by Bevy systems for game logic.

| Field | Type | Source | Consumers |
|-------|------|--------|-----------|
| npc_count | usize | Dispatch count | process(), query APIs |
| positions | Vec\<f32\> | position_buffer readback | attack_system (range check) |
| combat_targets | Vec\<i32\> | combat_target_buffer readback | attack_system (target selection) |
| health | Vec\<f32\> | CPU cache (not GPU readback) | (available for queries) |
| factions | Vec\<i32\> | CPU cache | (available for queries) |

## Slot Pools

| Pool | Type | Push | Pop |
|------|------|------|-----|
| SlotAllocator (Bevy) | Resource | death_cleanup_system (NPC dies) | allocate_slot() (NPC spawns) |
| FREE_PROJ_SLOTS | Vec\<usize\> | process() (projectile hits/expires) | fire_projectile() |

NPC slots use Bevy's `SlotAllocator` resource for unified spawn/death handling. Projectile slots still use static. Both are LIFO (stack) — most recently freed slot is reused first. No generational counters.

## Slot Allocation vs GPU Dispatch

Slot allocation and GPU dispatch are decoupled, preventing uninitialized buffer data from being dispatched.

| Counter | Type | Writer | Reader |
|---------|------|--------|--------|
| SlotAllocator.next | Bevy Resource | allocate_slot(), reset() | allocate_slot(), get_npc_count() |
| GPU_DISPATCH_COUNT | `Mutex<usize>` | spawn_npc_system | process() for dispatch |

`SlotAllocator.next` is the high-water mark — incremented when `spawn_npc()` allocates a slot via Bevy resource. `GPU_DISPATCH_COUNT` is only updated after `spawn_npc_system` pushes GPU buffer data to `GPU_UPDATE_QUEUE`. This ensures `process()` never dispatches NPCs with uninitialized GPU buffers. See [frame-loop.md](frame-loop.md) for timing details.

## Control Flags

| Flag | Type | Writer | Reader |
|------|------|--------|--------|
| RESET_BEVY | bool | reset() API | reset_bevy_system (despawns all entities) |

Note: `FRAME_DELTA` was removed — systems now use `Res<PhysicsDelta>` from godot-bevy (synced with Godot's physics frame).

## Debug State

| State | Type | Writer | Reader |
|-------|------|--------|--------|
| HEALTH_DEBUG | HealthDebugInfo | damage_system, death_system, death_cleanup_system | get_health_debug() API |
| COMBAT_DEBUG | CombatDebug | attack_system | get_combat_debug() API |

COMBAT_DEBUG (defined in `systems/combat.rs`) tracks 18 fields: `attackers_queried`, `targets_found`, `attacks_made`, `chases_started`, `in_combat_added`, `sample_target_idx`, `positions_len`, `combat_targets_len`, `bounds_failures`, `sample_dist`, `in_range_count`, `timer_ready_count`, `sample_timer`, `cooldown_entities`, `frame_delta`, `sample_combat_target_0/5`, `sample_pos_0/5`.

## Food Storage & Events

`FOOD_STORAGE: Mutex<FoodStorage>` — Bevy-owned per-town food counts. All settlements are "towns" (villager towns first, then raider towns by index).

| Field | Type | Writer | Reader |
|-------|------|--------|--------|
| food | `Vec<i32>` | add_town_food() API, raider_arrival_system | get_town_food() API |

| Queue | Type | Writer | Reader |
|-------|------|--------|--------|
| FOOD_DELIVERED_QUEUE | `Vec<FoodDelivered>` | raider_arrival_system | get_food_events() API |
| FOOD_CONSUMED_QUEUE | `Vec<FoodConsumed>` | (future eat system) | get_food_events() API |

## UI Query State

Static registries for UI panels to query NPC data. GDScript can't access Bevy World directly, so these caches bridge the boundary.

| Static | Type | Writer | Reader |
|--------|------|--------|--------|
| NPC_META | `Vec<NpcMeta>` | spawn_npc_system (init), set_npc_name() | get_npc_info(), get_npcs_by_town(), get_npc_name() |
| NPC_STATES | `Vec<i32>` | spawn_npc_system, behavior systems | get_npc_info(), get_npcs_by_town() |
| NPC_ENERGY | `Vec<f32>` | energy_system | get_npc_info() |
| KILL_STATS | `KillStats` | death_cleanup_system | get_population_stats() |
| SELECTED_NPC | `i32` | set_selected_npc() | get_selected_npc() |
| NPCS_BY_TOWN | `Vec<Vec<usize>>` | spawn_npc_system (add), death_cleanup_system (remove) | get_npcs_by_town(), get_population_stats(), get_town_population() |

**NpcMeta struct:** name (String), level (i32), xp (i32), trait_id (i32), town_id (i32), job (i32)

**KillStats struct:** guard_kills (i32), villager_kills (i32)

**State constants:** STATE_IDLE=0, STATE_WALKING=1, STATE_RESTING=2, STATE_WORKING=3, STATE_PATROLLING=4, STATE_ON_DUTY=5, STATE_FIGHTING=6, STATE_RAIDING=7, STATE_RETURNING=8, STATE_RECOVERING=9, STATE_FLEEING=10, STATE_GOING_TO_REST=11, STATE_GOING_TO_WORK=12

## Architecture: Channels vs Statics

| Category | Pattern | Items | Count |
|----------|---------|-------|-------|
| GDScript→Bevy | **Channel** (migrated Phase 11.7) | SpawnNpc, SetTarget, ApplyDamage, Reset, SetPaused, SetTimeScale | 6 msgs |
| Bevy→GDScript | **Channel** (migrated Phase 11.7) | FireProjectile (+ future Sync* msgs) | 1 msg |
| lib.rs boundary | Static Mutex (stays) | ARRIVAL_QUEUE, FREE_PROJ_SLOTS | 2 |
| Bevy↔GPU boundary | Static Mutex (stays) | GPU_UPDATE_QUEUE, GPU_READ_STATE, GPU_DISPATCH_COUNT | 3 |
| UI query state | Static Mutex (stays) | NPC_META, NPC_STATES, NPC_ENERGY, KILL_STATS, SELECTED_NPC, NPCS_BY_TOWN | 6 |
| Bevy-internal state | Bevy Resources | FOOD_STORAGE, SlotAllocator, GAME_CONFIG_STAGING (staging only) | 3 |

**Phase 11.7 completed:** Replaced 5 static queues with lock-free crossbeam channels:
- SPAWN_QUEUE → GodotToBevyMsg::SpawnNpc
- TARGET_QUEUE → GodotToBevyMsg::SetTarget
- DAMAGE_QUEUE → GodotToBevyMsg::ApplyDamage
- PROJECTILE_FIRE_QUEUE → BevyToGodotMsg::FireProjectile
- RESET_BEVY → GodotToBevyMsg::Reset

**Why channels?** godot-bevy docs recommend crossbeam for cross-thread communication. Channels are lock-free (no Mutex contention), fire-and-forget from lib.rs, and drained by Bevy systems that can run in parallel.

**Why statics remain at lib.rs boundary?** lib.rs runs outside Bevy's scheduler. Accessing Bevy resources via `get_bevy_app()` would serialize all calls on the main thread ("systems with GodotAccess are forced onto the main thread and run sequentially"). The remaining statics are low-frequency or batch operations (slot allocation, GPU buffer sync, UI queries).

**Phase 10.2 completed:** Bevy systems emit `GpuUpdateMsg` via `MessageWriter` instead of locking `GPU_UPDATE_QUEUE` directly. `collect_gpu_updates` system runs at end of frame and does a single Mutex lock to batch all updates.

## Known Issues / Limitations

- **Channels are unbounded**: No backpressure. If spawn calls outpace Bevy drain (shouldn't happen at 60fps), channels grow without limit.
- **GPU_READ_STATE is one frame stale**: Bevy reads positions from previous frame's dispatch. Acceptable at 140fps.
- **7 statics at lib.rs boundary**: ARRIVAL_QUEUE, projectile slots, GPU state. NPC slot allocation moved to Bevy SlotAllocator resource.

## Rating: 7/10

Hybrid channel + static architecture works but has gaps. Channels are unbounded with no backpressure — if spawn calls outpace drain, memory grows without limit. GPU_READ_STATE is one frame stale. Health has confusing dual ownership (CPU-authoritative but synced to GPU). 7 statics remain at lib.rs boundary. Lock-free channels are good; the boundary complexity is the cost of Godot/Bevy integration.
