# Buildings as ECS Entities (Unified with NPCs)

## Problem

Buildings share the same GPU pipeline as NPCs (same `SlotAllocator`, same GPU updates: position, faction, health, speed=0, sprite). But they have a completely parallel lifecycle: `WorldData.buildings` + `BuildingHpState` + `BuildingSlotMap` + `BuildingSpatialGrid` + tombstone guards, instead of using the NPC ECS path (`materialize_npc` -> `NpcEntityMap` -> `death_system` -> `death_cleanup_system` -> despawn). This creates ~720 references of redundant infrastructure.

## Design Principle

**Building = NPC with speed=0 on building atlas.** `BUILDING_REGISTRY` is the source of truth for building type definitions (like `NPC_REGISTRY` for NPCs). Building instances live as ECS entities. Reuse the NPC lifecycle (`materialize_npc`, `NpcEntityMap`, `death_system`, `death_cleanup_system`). Add a `Building` marker component to distinguish from walking NPCs where needed.

`BUILDING_REGISTRY` keeps its static definition fields (`kind`, `hp`, `cost`, `tile`, `is_tower`, `tower_stats`, `spawner`, `label`, `placement`, etc.) but sheds all the WorldData fn pointers (`len`, `pos_town`, `tombstone`, `find_index`, `hps`, `hps_mut`, `place`, `save_vec`, `load_vec`) -- those exist only because buildings were data, not entities.

## What Gets Deleted Eventually

- `WorldData.buildings: BTreeMap<BuildingKind, Vec<PlacedBuilding>>` -- replaced by ECS queries
- ~~`BuildingHpState`~~ -- **DONE (Phase 2)**: deleted, replaced by entity `Health` component
- `BuildingSlotMap` -- replaced by `NpcEntityMap` (buildings get `NpcIndex` like NPCs)
- `BuildingSpatialGrid` -- rebuilt from entity queries or WorldGrid
- `BUILDING_REGISTRY` fn pointers: `len`, `pos_town`, `tombstone`, `find_index` (~~`hps`, `hps_mut`~~ deleted in Phase 2)
- `PlacedBuilding` struct
- All `is_alive()` / tombstone guards
- `building_damage_system` -- merged into NPC damage pipeline
- `sync_building_hp_render` -- merged into NPC health rendering

---

## Phase 1: Buildings Enter NPC Lifecycle

### Goal

Buildings spawn as ECS entities via `materialize_npc`, register in `NpcEntityMap`, and die via `death_system` -> `death_cleanup_system` -> despawn. Dual-write to WorldData so existing read paths still work during migration.

### New component (`components.rs`)

```rust
#[derive(Component, Clone, Copy)]
pub struct Building {
    pub kind: BuildingKind,
}
```

Buildings reuse: `NpcIndex`, `Position`, `Health`, `Faction`, `TownId`, `Speed(0.0)`.

### Step 1: Spawn building entities alongside WorldData

**`allocate_building_slot()` (`world.rs:518`)** currently does GPU init but no entity spawn. It already reads from `BUILDING_REGISTRY` to get `hp`, `tileset_index`, `is_tower`. Extend it to also spawn an ECS entity using the same registry data:
- `NpcIndex(slot)` -- same GPU slot, now in `NpcEntityMap`
- `Position`, `Health(def.hp)`, `Faction`, `TownId`, `Speed(0.0)`
- `Building { kind: def.kind }` marker
- Registered in `NpcEntityMap`

Problem: `allocate_building_slot` uses `GPU_UPDATE_QUEUE` directly (not `MessageWriter<GpuUpdateMsg>`), and doesn't have `Commands`. It's called from `allocate_all_building_slots()` which is called at init/load time.

**Solution**: Split into two steps:
1. Keep `allocate_building_slot()` for GPU init (unchanged) -- it reads `BUILDING_REGISTRY` for sprite/HP/tower
2. Add `spawn_building_entities()` that runs after init/load: iterates `BUILDING_REGISTRY`, uses each def's `pos_town` to find alive buildings, spawns entities with `Building { kind: def.kind }` + shared NPC components, registers in `NpcEntityMap`

For runtime placement (`place_building()`, `world.rs:270`): `BUILDING_REGISTRY` lookup by kind gives `hp`, `is_tower`, sprite -- use these to spawn entity after existing WorldData write.

### Step 2: Building death -> despawn (not tombstone)

**`destroy_building()` (`world.rs:832`)** currently tombstones. Add:
1. Look up entity from `NpcEntityMap` via the building's GPU slot
2. Insert `Dead` component (reuses `death_system` -> `death_cleanup_system` pipeline)
3. `death_cleanup_system` already handles: despawn entity, hide on GPU, free slot, remove from `NpcEntityMap`

Still tombstone in WorldData (dual-write) so existing read paths work.

### Step 3: Fix combat system building/NPC discrimination

**`attack_system` (`combat.rs:174`)**: `!npc_map.0.contains_key(&ti)` rejects buildings. Once buildings are in `NpcEntityMap`, this check breaks. Change to:
```rust
// Reject building targets for melee (buildings are attacked via separate logic)
if let Some(&entity) = npc_map.0.get(&ti) {
    if building_query.contains(entity) { continue; } // skip buildings for NPC melee
}
```
Requires adding `building_query: Query<(), With<Building>>` to attack_system.

**`process_proj_hits` (`combat.rs:321`)**: `building_slots.get_building(npc_idx)` to detect building hits. Change to:
```rust
if let Some(&entity) = npc_map.0.get(&(npc_idx as usize)) {
    if building_query.contains(entity) {
        // emit BuildingDamageMsg
    } else {
        // emit DamageMsg
    }
}
```

### Step 4: Save/Load

- **Load**: After loading WorldData, `spawn_building_entities()` creates entities for all alive buildings (same as init)
- **Save**: No change yet (still serializes WorldData). Future phase migrates save to query entities.
- Pre-load: despawn all building entities: `Query<Entity, With<Building>>`

### Step 5: Cleanup/restart

`cleanup_game()` (`ui/mod.rs`) already despawns all NPC entities. Add building entities to that query (or they'll be caught by `With<NpcIndex>` if we add that marker).

### Files modified (Phase 1)

| File | Changes |
|------|---------|
| `components.rs` | Add `Building { kind }` component |
| `world.rs` | `place_building`: spawn entity. `destroy_building`: insert `Dead`. New `spawn_building_entities()`. |
| `systems/combat.rs` | `attack_system`: building query filter. `process_proj_hits`: entity-based building check. |
| `systems/health.rs` | `death_cleanup_system`: handle `Building` entities (skip NPC-specific cleanup like farm release, pop stats for buildings) |
| `save.rs` | Load path: despawn building entities, then spawn from WorldData |
| `ui/mod.rs` | `cleanup_game`: include building entity despawn |
| `lib.rs` | Register `Building` component |

### What stays unchanged (Phase 1)

- `WorldData.buildings` -- still written to (dual-write)
- `BuildingHpState` -- still maintained (Phase 2 removes it)
- `BuildingSpatialGrid` -- still rebuilt from WorldData (Phase 3 removes it)
- `BuildingSlotMap` -- still maintained alongside `NpcEntityMap` (removed when all consumers migrated)
- All AI/economy/behavior systems -- they read WorldData, unchanged
- Save format -- unchanged

---

## Phase 2: HP -> Health component

- `building_damage_system` writes to `Health` component instead of `BuildingHpState`
- `healing_system` already queries `Health` -- buildings with `Health` auto-heal
- `sync_building_hp_render` queries entities
- Delete `BuildingHpState`

## Phase 3: BuildingSpatialGrid -> entity queries

- Rebuild from `Query<(&Building, &Position, &Faction, &TownId, &NpcIndex)>`
- Or replace with WorldGrid cell lookups

## Phase 4: Merge damage pipelines

- Delete `BuildingDamageMsg` -- projectile hits write `DamageMsg` for both NPCs and buildings
- `damage_system` handles both (building death triggers `destroy_building` for grid cleanup)

## Phase 5: AI/Economy/Behavior

- Replace `world_data.farms()` / `waypoints()` with entity queries
- Delete `town_building_slots!` macro

## Phase 6: Save/Load

- Serialize building entities directly
- Delete WorldData.buildings serialization

## Phase 7: Final cleanup

- Delete `WorldData.buildings`, `PlacedBuilding`, tombstone pattern
- Strip `BUILDING_REGISTRY` fn pointers: remove `len`, `pos_town`, `count_for_town`, `hps`, `hps_mut`, `save_vec`, `load_vec`, `place`, `tombstone`, `find_index`, `is_unit_home`. Keep static definition fields: `kind`, `hp`, `cost`, `tile`, `label`, `help`, `tooltip`, `placement`, `is_tower`, `tower_stats`, `spawner`, `on_place`, `display`, `player_buildable`, `raider_buildable`
- `WorldGrid.cells[].building` stores `Option<Entity>`
- Delete `BuildingSlotMap` (fully replaced by `NpcEntityMap`)

---

## Verification (Phase 1)

1. `cargo check` -- compiles
2. `cargo test` -- existing tests pass
3. Manual: place buildings -> verify they render. Destroy buildings -> verify despawn (no tombstone ghosts). Save -> load -> buildings still there. Combat -> buildings take damage -> destroyed -> entity gone. New game -> clean state.

## Key existing code to reuse

- `materialize_npc()` (`spawn.rs:101`) -- building spawn shares GPU init pattern
- `death_system` / `death_cleanup_system` (`health.rs:66,85`) -- building death reuses this
- `SlotAllocator` (`resources.rs:412`) -- already shared
- `NpcEntityMap` (`resources.rs:102`) -- buildings register here too
- `GPU_UPDATE_QUEUE` / `GpuUpdate` -- same messages for both
