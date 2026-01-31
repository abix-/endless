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

Projectiles are created internally by Bevy's `attack_system` via `PROJECTILE_FIRE_QUEUE`. No GDScript fire API — all combat projectiles originate from Bevy ECS.

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
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
| `get_npc_target` | `idx: i32` | `Vector2` | Movement target from cached GPU data |
| `get_npc_health` | `idx: i32` | `f32` | Health from cached GPU data |
| `get_build_info` | none | `String` | Build timestamp and commit hash |
| `get_debug_stats` | none | `Dictionary` | npc_count, arrived_count, avg/max_backoff, cells_used, max_per_cell |
| `get_combat_debug` | none | `Dictionary` | attackers, targets_found, attacks, chases, sample positions/distances |
| `get_health_debug` | none | `Dictionary` | damage_processed, deaths, despawned, entity_count, health_samples |
| `get_guard_debug` | none | `Dictionary` | arrived_flags, prev_arrivals_true, arrival_queue_len |

## UI Query API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `get_population_stats` | none | `Dictionary` | farmers_alive, guards_alive, raiders_alive, guard_kills, villager_kills |
| `get_town_population` | `town_idx: i32` | `Dictionary` | farmer_count, guard_count, raider_count for one town |
| `get_npc_info` | `idx: i32` | `Dictionary` | Full NPC details: name, job (string), level, xp, trait (string), town_id, hp, max_hp, energy, state (string), target_idx, x, y, faction |
| `get_npcs_by_town` | `town_idx, filter: i32` | `Array` | Array of NPC dicts (idx, name, job, level, hp, max_hp, state, trait). All strings. Filter: -1=all, 0=farmer, 1=guard, 2=raider |
| `get_selected_npc` | none | `i32` | Currently selected NPC index (-1 = none) |
| `set_selected_npc` | `idx: i32` | void | Set selected NPC for inspector |
| `get_npc_name` | `idx: i32` | `String` | NPC name by index |
| `get_npc_trait` | `idx: i32` | `i32` | NPC trait ID by index |
| `set_npc_name` | `idx, name: String` | void | Rename NPC |
| `get_bed_stats` | `town_idx: i32` | `Dictionary` | total_beds, free_beds for a town |
| `get_npc_at_position` | `x, y, radius: f32` | `i32` | Nearest alive NPC within radius, or -1 (for click selection) |

## World Data API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `init_world` | `town_count: i32` | void | Initialize world data structures |
| `add_town` | `name: String, cx, cy, faction: i32` | void | Register town with center and faction (0=Villager, 1=Raider) |
| `add_farm` | `x, y, town_idx: i32` | void | Register farm, init occupancy |
| `add_bed` | `x, y, town_idx: i32` | void | Register bed, init occupancy |
| `add_guard_post` | `x, y, town_idx, patrol_order: i32` | void | Register patrol post with order |
| `get_town_center` | `town_idx: i32` | `Vector2` | Town center position (works for all towns) |
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
| `init_food_storage` | `total_towns: i32` | void | Initialize per-town food counters (villager + raider towns unified) |
| `add_town_food` | `town_idx, amount: i32` | void | Add food to a town (farmer produced) |
| `get_town_food` | `town_idx: i32` | `i32` | Get food count for any town (villager or raider) |
| `get_food_events` | none | `Dictionary` | Deliveries and consumed counts since last call (clears queues) |

## Faction Stats API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `init_faction_stats` | `total_factions: i32` | void | Initialize per-faction stats (1 + num_raider_camps) |
| `get_faction_stats` | `faction_id: i32` | `Dictionary` | Stats for one faction: alive, dead, kills |
| `get_all_faction_stats` | none | `Array` | Array of faction stat dicts. Index = faction_id. |

Faction IDs: 0 = villagers (all towns share), 1+ = raider camps (each unique). `inc_alive()` called at spawn, `dec_alive()`/`inc_dead()` called at death.

## Time API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `get_game_time` | none | `Dictionary` | day, hour, minute, is_daytime, time_scale, paused |
| `set_time_scale` | `scale: f32` | void | Set time multiplier (1.0 = normal, 2.0 = 2x speed, clamped >= 0) |
| `set_paused` | `paused: bool` | void | Pause or unpause game time |

Time API accesses Bevy's `GameTime` resource directly through the BevyApp autoload (no static bridge needed).

## NPC Activity Log API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `get_npc_log` | `idx, limit: i32` | `Array` | Last N log entries (dicts with day, hour, minute, message), most recent first |

## Reset API

| Method | Params | Returns | Description |
|--------|--------|---------|-------------|
| `reset` | none | void | Full reset: clear all queues, GPU state, Bevy entities, free slots, world data, projectiles |
