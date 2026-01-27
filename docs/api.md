# GDScript API Reference

## Overview

All GDScript interaction with the ECS goes through `EcsNpcManager`, a Godot `Node2D` that owns the GPU compute context and bridges to Bevy. Methods are exposed via `#[func]` in `rust/src/lib.rs`.

## Spawn API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `spawn_npc` | `x, y, job: i32` | void | Generic NPC. Job: 0=Villager, 1=Guard, 2=Farmer, 3=Raider |
| `spawn_guard` | `x, y, town_idx, home_x, home_y` | void | Guard with patrol route, faction 0, starts at post 0 |
| `spawn_guard_at_post` | `x, y, town_idx, home_x, home_y, starting_post` | void | Guard starting at specific patrol post (arrival pre-set) |
| `spawn_farmer` | `x, y, town_idx, home_x, home_y, work_x, work_y` | void | Farmer with work position, faction 0 |
| `spawn_raider` | `x, y, camp_x, camp_y` | void | Raider with camp as home, faction 1 |

All spawn methods allocate a slot (recycled or new), write GPU buffers directly, and queue a Bevy message. See [spawn.md](spawn.md).

## Projectile API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `fire_projectile` | `from_x, from_y, to_x, to_y, damage, faction, shooter_idx` | `i32` | Fire projectile. Returns slot index or -1 if at capacity. |
| `get_projectile_count` | none | `i32` | Current proj_count (high-water mark) |
| `get_projectile_debug` | none | `Dictionary` | proj_count, active, visible, pipeline_valid, sample positions |
| `get_projectile_trace` | none | `String` | First N projectiles with lifetime/active/pos/hit data |

## Target API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `set_target` | `npc_idx: i32, x, y` | void | Set movement target. Clears arrival/backoff flags. Queues to Bevy. |

## Health API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `apply_damage` | `npc_idx: i32, amount: f32` | void | Queue damage to DAMAGE_QUEUE for Bevy processing |

## Query API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `get_npc_count` | none | `i32` | Current NPC count from GPU_READ_STATE |
| `get_npc_position` | `idx: i32` | `Vector2` | Position from cached GPU data |
| `get_build_info` | none | `String` | Build timestamp and commit hash |
| `get_debug_stats` | none | `Dictionary` | npc_count, arrived_count, avg/max_backoff, cells_used, max_per_cell |
| `get_combat_debug` | none | `Dictionary` | attackers, targets_found, attacks, chases, sample positions/distances |
| `get_health_debug` | none | `Dictionary` | damage_processed, deaths, despawned, entity_count, health_samples |
| `get_guard_debug` | none | `Dictionary` | arrived_flags, prev_arrivals_true, arrival_queue_len |

## World Data API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `init_world` | `town_count: i32` | void | Initialize world data structures |
| `add_town` | `name: String, cx, cy, camp_x, camp_y` | void | Register town with center and camp positions |
| `add_farm` | `x, y, town_idx: i32` | void | Register farm, init occupancy |
| `add_bed` | `x, y, town_idx: i32` | void | Register bed, init occupancy |
| `add_guard_post` | `x, y, town_idx, patrol_order: i32` | void | Register patrol post with order |
| `get_town_center` | `town_idx: i32` | `Vector2` | Town center position |
| `get_camp_position` | `town_idx: i32` | `Vector2` | Raider camp position |
| `get_patrol_post` | `town_idx, order: i32` | `Vector2` | Patrol post position by order |
| `get_nearest_free_bed` | `x, y, town_idx: i32` | `Vector2` | Nearest unoccupied bed (or Vector2(-1,-1)) |
| `get_nearest_free_farm` | `x, y, town_idx: i32` | `Vector2` | Nearest unoccupied farm (or Vector2(-1,-1)) |
| `reserve_bed` | `x, y, npc_idx: i32` | void | Mark bed as occupied |
| `release_bed` | `x, y` | void | Mark bed as free |
| `reserve_farm` | `x, y, npc_idx: i32` | void | Mark farm as occupied |
| `release_farm` | `x, y` | void | Mark farm as free |
| `get_world_stats` | none | `Dictionary` | town/farm/bed/guard_post counts, free_beds, free_farms |

## Reset API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `reset` | none | void | Full reset: clear all queues, GPU state, Bevy entities, free slots, world data, projectiles |
