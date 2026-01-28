# Message Queues & Shared State

## Overview

Static Mutex-protected queues bridge Godot's single-threaded GDScript calls, Bevy's ECS systems, and the GPU compute pipeline. All defined in `rust/src/messages.rs`.

## GDScript to Bevy Queues

| Queue | Message Type | Fields | Producer | Consumer |
|-------|-------------|--------|----------|----------|
| SPAWN_QUEUE | SpawnNpcMsg | slot_idx, x, y, job, faction, town_idx, home_x/y, work_x/y, starting_post, attack_type | spawn_npc() | drain_spawn_queue → spawn_npc_system |
| TARGET_QUEUE | SetTargetMsg | npc_index, x, y | set_target() | drain_target_queue → apply_targets_system |
| ARRIVAL_QUEUE | ArrivalMsg | npc_index | process() arrival detection | drain_arrival_queue → handle_arrival_system |
| DAMAGE_QUEUE | DamageMsg | npc_index, amount | projectile hits (GPU→CPU) | drain_damage_queue → damage_system |
| PROJECTILE_FIRE_QUEUE | FireProjectileMsg | from_x/y, to_x/y, damage, faction, shooter, speed, lifetime | attack_system | process() → upload_projectile() |

## GPU Update Queue

`GPU_UPDATE_QUEUE: Mutex<Vec<GpuUpdate>>` — unified queue for Bevy systems to write GPU buffer updates. Drained at the start of each `process()` call.

| Variant | Fields | Producer | GPU Buffer Updated |
|---------|--------|----------|-------------------|
| SetTarget | idx, x, y | attack_system, behavior systems | target_buffer, arrival_buffer, backoff_buffer |
| SetHealth | idx, health | damage_system | health_buffer |
| SetFaction | idx, faction | spawn_npc_system | faction_buffer |
| SetPosition | idx, x, y | spawn_npc_system | position_buffer |
| SetSpeed | idx, speed | spawn_npc_system | speed_buffer |
| SetColor | idx, r, g, b, a | spawn_npc_system, raider_arrival_system, flee_system | color_buffer |
| ApplyDamage | idx, amount | (unused — damage goes through Bevy) | health_buffer |
| HideNpc | idx | death_cleanup_system | position_buffer → (-9999, -9999) |

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
| FREE_SLOTS | Vec\<usize\> | death_cleanup_system (NPC dies) | allocate_slot() (NPC spawns) |
| FREE_PROJ_SLOTS | Vec\<usize\> | process() (projectile hits/expires) | fire_projectile() |

Both are LIFO (stack) — most recently freed slot is reused first. No generational counters.

## Slot Allocation vs GPU Dispatch

Two separate counters decouple slot allocation from GPU dispatch, preventing uninitialized buffer data from being dispatched.

| Counter | Type | Writer | Reader |
|---------|------|--------|--------|
| NPC_SLOT_COUNTER | `Mutex<usize>` | allocate_slot() | allocate_slot() |
| GPU_DISPATCH_COUNT | `Mutex<usize>` | spawn_npc_system | process() for dispatch |

`NPC_SLOT_COUNTER` is the high-water mark — incremented immediately when GDScript calls `spawn_npc()`. `GPU_DISPATCH_COUNT` is only updated after `spawn_npc_system` pushes GPU buffer data to `GPU_UPDATE_QUEUE`. This ensures `process()` never dispatches NPCs with uninitialized GPU buffers. See [frame-loop.md](frame-loop.md) for timing details.

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

## Architecture: What Stays Static vs What Migrates

All communication currently uses static Mutex. This is correct for cross-boundary state but not idiomatic for Bevy-internal state. See [roadmap.md](roadmap.md) Phase 10.

| Category | Pattern | Statics | Count |
|----------|---------|---------|-------|
| GDScript↔Bevy boundary | Static Mutex (stays) | SPAWN/TARGET/DAMAGE/ARRIVAL_QUEUE, RESET_BEVY, NPC_SLOT_COUNTER, FREE_SLOTS, FREE_PROJ_SLOTS | 8 |
| Bevy↔GPU boundary | Static Mutex (stays) | GPU_UPDATE_QUEUE, GPU_READ_STATE, GPU_DISPATCH_COUNT | 3 |
| UI query state | Static Mutex (stays) | NPC_META, NPC_STATES, NPC_ENERGY, KILL_STATS, SELECTED_NPC, NPCS_BY_TOWN | 6 |
| Bevy-internal state | Migrate → `Res<T>` / Events | WORLD_DATA, BED/FARM_OCCUPANCY, HEALTH/COMBAT_DEBUG, FOOD_STORAGE, food event queues | 8 |

**Migration pattern:** Bevy systems emit `GpuUpdateEvent` instead of locking `GPU_UPDATE_QUEUE` directly. A single collector system drains events and locks the static queue once. Bevy-internal state uses `Res<T>` / `ResMut<T>` with staging statics at the GDScript boundary. This enables multi-threaded Bevy scheduling — systems that don't share Resources can run in parallel.

## Known Issues / Limitations

- **All queues are unbounded**: No backpressure. If spawn calls outpace Bevy drain (shouldn't happen at 60fps), queues grow without limit.
- **GPU_READ_STATE is one frame stale**: Bevy reads positions from previous frame's dispatch. Acceptable at 140fps.
- **Bevy-internal statics**: 8 statics that should be Bevy Resources still use static Mutex. Functional but hides data dependencies from Bevy's scheduler. See Phase 10 migration plan.

## Rating: 8/10

Clean unified queue architecture. GPU_UPDATE_QUEUE consolidates what was originally 10+ separate queues. Spawn path now routes all GPU writes through the queue — no more direct `buffer_update()` calls. The static Mutex pattern is correct for cross-boundary state (12 statics). Bevy-internal state (8 statics) planned for migration to Resources.
