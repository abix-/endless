# GDScript API Reference

## Overview

All GDScript interaction with the ECS goes through `EcsNpcManager`, a Godot `Node2D` that owns the GPU compute context and bridges to Bevy. Methods are exposed via `#[func]` in `rust/src/lib.rs`.

## Spawn API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `spawn_npc` | `x, y, job, faction, opts: Dictionary` | `i32` | Unified spawn. Returns slot index or -1. Job: 0=Farmer, 1=Guard, 2=Raider, 3=Fighter. |

Job determines component template at spawn time. Optional params in Dictionary: `home_x`, `home_y`, `work_x`, `work_y`, `town_idx`, `starting_post`, `attack_type`. All default to -1 or 0. See [spawn.md](spawn.md).

```gdscript
# Guard at patrol post 2:
ecs.spawn_npc(pos.x, pos.y, 1, 0, {"home_x": home.x, "home_y": home.y, "town_idx": town_idx, "starting_post": 2})
# Farmer:
ecs.spawn_npc(pos.x, pos.y, 0, 0, {"home_x": home.x, "home_y": home.y, "work_x": farm.x, "work_y": farm.y, "town_idx": town_idx})
# Raider:
ecs.spawn_npc(pos.x, pos.y, 2, 1, {"home_x": camp.x, "home_y": camp.y})
# Ranged fighter:
ecs.spawn_npc(pos.x, pos.y, 3, 1, {"attack_type": 1})
```

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
| `get_nearest_free_bed` | `town_idx: i32, x, y: f32` | `i32` | Nearest free bed index or -1 |
| `get_nearest_free_farm` | `town_idx: i32, x, y: f32` | `i32` | Nearest free farm index or -1 |
| `reserve_bed` | `bed_idx: i32, npc_idx: i32` | `bool` | Claim bed if free |
| `release_bed` | `bed_idx: i32` | void | Set occupant to -1 |
| `reserve_farm` | `farm_idx: i32` | `bool` | Claim farm if count < 1 |
| `release_farm` | `farm_idx: i32` | void | Decrement occupancy |
| `get_world_stats` | none | `Dictionary` | town/farm/bed/guard_post counts, free_beds, free_farms |

## Food Storage API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `init_food_storage` | `town_count, camp_count: i32` | void | Initialize per-town and per-camp food counters |
| `add_town_food` | `town_idx, amount: i32` | void | Add food to a town (farmer produced) |
| `get_town_food` | `town_idx: i32` | `i32` | Get food count for a town |
| `get_camp_food` | `camp_idx: i32` | `i32` | Get food count for a camp |
| `get_food_events` | none | `Dictionary` | Deliveries and consumed counts since last call (clears queues) |

## Reset API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `reset` | none | void | Full reset: clear all queues, GPU state, Bevy entities, free slots, world data, projectiles |
