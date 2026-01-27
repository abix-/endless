# Message Queues & Shared State

## Overview

Static Mutex-protected queues bridge Godot's single-threaded GDScript calls, Bevy's ECS systems, and the GPU compute pipeline. All defined in `rust/src/messages.rs`.

## GDScript to Bevy Queues

| Queue | Message Type | Fields | Producer | Consumer |
|-------|-------------|--------|----------|----------|
| SPAWN_QUEUE | SpawnNpcMsg | slot_idx, x, y, job, faction, town_idx, home_x/y, work_x/y, starting_post | spawn_npc() | drain_spawn_queue → spawn_npc_system |
| TARGET_QUEUE | SetTargetMsg | npc_index, x, y | set_target() | drain_target_queue → apply_targets_system |
| ARRIVAL_QUEUE | ArrivalMsg | npc_index | process() arrival detection | drain_arrival_queue → handle_arrival_system |
| DAMAGE_QUEUE | DamageMsg | npc_index, amount | attack_system, projectile hits | drain_damage_queue → damage_system |

## GPU Update Queue

`GPU_UPDATE_QUEUE: Mutex<Vec<GpuUpdate>>` — unified queue for Bevy systems to write GPU buffer updates. Drained at the start of each `process()` call.

| Variant | Fields | Producer | GPU Buffer Updated |
|---------|--------|----------|-------------------|
| SetTarget | idx, x, y | attack_system, behavior systems | target_buffer, arrival_buffer, backoff_buffer |
| SetHealth | idx, health | damage_system | health_buffer |
| SetFaction | idx, faction | spawn_npc_system | faction_buffer |
| SetPosition | idx, x, y | spawn_npc_system | position_buffer |
| SetSpeed | idx, speed | spawn_npc_system | speed_buffer |
| SetColor | idx, r, g, b, a | spawn_npc_system, steal_arrival_system, flee_system | color_buffer |
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
| FRAME_DELTA | f32 | process() | cooldown_system, energy_system |

## Debug State

| State | Type | Writer | Reader |
|-------|------|--------|--------|
| HEALTH_DEBUG | HealthDebugInfo | damage_system, death_system, death_cleanup_system | get_health_debug() API |
| COMBAT_DEBUG | CombatDebug | attack_system | get_combat_debug() API |

COMBAT_DEBUG (defined in `systems/combat.rs`) tracks 18 fields: `attackers_queried`, `targets_found`, `attacks_made`, `chases_started`, `in_combat_added`, `sample_target_idx`, `positions_len`, `combat_targets_len`, `bounds_failures`, `sample_dist`, `in_range_count`, `timer_ready_count`, `sample_timer`, `cooldown_entities`, `frame_delta`, `sample_combat_target_0/5`, `sample_pos_0/5`.

## Food Storage & Events

`FOOD_STORAGE: Mutex<FoodStorage>` — Bevy-owned per-town and per-camp food counts. Keeping food in Rust avoids cross-boundary calls during raider eat decisions.

| Field | Type | Writer | Reader |
|-------|------|--------|--------|
| town_food | `Vec<i32>` | add_town_food() API | get_town_food() API |
| camp_food | `Vec<i32>` | steal_arrival_system | get_camp_food() API |

| Queue | Type | Writer | Reader |
|-------|------|--------|--------|
| FOOD_DELIVERED_QUEUE | `Vec<FoodDelivered>` | steal_arrival_system | get_food_events() API |
| FOOD_CONSUMED_QUEUE | `Vec<FoodConsumed>` | (future eat system) | get_food_events() API |

## Known Issues / Limitations

- **All queues are unbounded**: No backpressure. If spawn calls outpace Bevy drain (shouldn't happen at 60fps), queues grow without limit.
- **GPU_READ_STATE is one frame stale**: Bevy reads positions from previous frame's dispatch. Acceptable at 140fps.
- **Mutex contention**: All queues use `std::sync::Mutex`. At current scale (single Godot thread + Bevy on same thread via godot-bevy), there's no contention. Multi-threaded Bevy would need consideration.

## Rating: 8/10

Clean unified queue architecture. GPU_UPDATE_QUEUE consolidates what was originally 10+ separate queues. Spawn path now routes all GPU writes through the queue — no more direct `buffer_update()` calls. The static Mutex pattern is simple and correct for single-threaded Godot.
