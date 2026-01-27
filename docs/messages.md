# Message Queues & Shared State

## Overview

Static Mutex-protected queues bridge Godot's single-threaded GDScript calls, Bevy's ECS systems, and the GPU compute pipeline. All defined in `rust/src/messages.rs`.

## GDScript to Bevy Queues

| Queue | Message Type | Fields | Producer | Consumer |
|-------|-------------|--------|----------|----------|
| SPAWN_QUEUE | SpawnNpcMsg | x, y, job | spawn_npc() | drain_spawn_queue → spawn_npc_system |
| GUARD_QUEUE | SpawnGuardMsg | x, y, town_idx, home_x, home_y, starting_post | spawn_guard() | drain_guard_queue → spawn_guard_system |
| FARMER_QUEUE | SpawnFarmerMsg | x, y, town_idx, home_x, home_y, work_x, work_y | spawn_farmer() | drain_farmer_queue → spawn_farmer_system |
| RAIDER_QUEUE | SpawnRaiderMsg | x, y, camp_x, camp_y | spawn_raider() | drain_raider_queue → spawn_raider_system |
| TARGET_QUEUE | SetTargetMsg | npc_index, x, y | set_target() | drain_target_queue → apply_targets_system |
| ARRIVAL_QUEUE | ArrivalMsg | npc_index | process() arrival detection | drain_arrival_queue → handle_arrival_system |
| DAMAGE_QUEUE | DamageMsg | npc_index, amount | attack_system, projectile hits | drain_damage_queue → damage_system |

## GPU Update Queue

`GPU_UPDATE_QUEUE: Mutex<Vec<GpuUpdate>>` — unified queue for Bevy systems to write GPU buffer updates. Drained at the start of each `process()` call.

| Variant | Fields | Producer | GPU Buffer Updated |
|---------|--------|----------|-------------------|
| SetTarget | idx, x, y | attack_system, behavior systems | target_buffer, arrival_buffer, backoff_buffer |
| SetHealth | idx, health | damage_system | health_buffer |
| SetFaction | idx, faction | (unused currently, available) | faction_buffer |
| SetPosition | idx, x, y | (unused currently, available) | position_buffer |
| SetSpeed | idx, speed | (unused currently, available) | speed_buffer |
| SetColor | idx, r, g, b, a | (unused currently, available) | color_buffer |
| ApplyDamage | idx, amount | (unused — damage goes through Bevy) | health_buffer |
| HideNpc | idx | death_cleanup_system | position_buffer → (-9999, -9999) |

## GPU Read State

`GPU_READ_STATE: Mutex<GpuReadState>` — snapshot of GPU output, updated after each dispatch in `process()`. Read by Bevy systems for game logic.

| Field | Type | Source | Consumers |
|-------|------|--------|-----------|
| npc_count | usize | High-water mark | allocate_slot(), process() |
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

## Known Issues / Limitations

- **All queues are unbounded**: No backpressure. If spawn calls outpace Bevy drain (shouldn't happen at 60fps), queues grow without limit.
- **GPU_READ_STATE is one frame stale**: Bevy reads positions from previous frame's dispatch. Acceptable at 140fps.
- **Several GpuUpdate variants unused**: SetFaction, SetPosition, SetSpeed, SetColor, ApplyDamage are defined but no system currently produces them (spawns write directly, damage goes through Bevy). They exist for future use.
- **Mutex contention**: All queues use `std::sync::Mutex`. At current scale (single Godot thread + Bevy on same thread via godot-bevy), there's no contention. Multi-threaded Bevy would need consideration.

## Rating: 8/10

Clean unified queue architecture. GPU_UPDATE_QUEUE consolidates what was originally 10+ separate queues. The static Mutex pattern is simple and correct for single-threaded Godot. The unused variants are minor dead code — they provide a clear extension surface.
