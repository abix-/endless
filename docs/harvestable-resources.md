# Harvestable Resources

Trees and rocks become first-class building entities that NPCs harvest for wood and stone. Harvested nodes are destroyed permanently. This spec covers harvest and storage only -- building costs using wood/stone are a later slice.

## Resource types

| Resource | Source biome | Node kind | Storage component | Harvester building | Worker job | Activity |
|----------|-------------|-----------|-------------------|-------------------|------------|----------|
| Wood | Forest | `TreeNode` | `WoodStore` | `LumberMill` | `Woodcutter` | `Chop` |
| Stone | Rock | `RockNode` | `StoneStore` | `Quarry` | `Quarrier` | `Quarry` |

Iron is out of scope.

## CRD compliance

All new types follow the Def -> Instance -> Controller pattern (see `docs/k8s.md`):

- **TreeNode/RockNode**: `BuildingDef` entries in `BUILDING_REGISTRY`. Spawned via `place_building()`, destroyed via `destroy_building()`. One enum variant + one registry entry each.
- **Chop/Quarry**: `ActivityDef` entries in `ACTIVITY_REGISTRY`. Fieldless `ActivityKind` variants. One enum variant + one registry entry each.
- **LumberMill/Quarry**: `BuildingDef` entries with `spawner: Some(SpawnerDef { job, count: 1 })`. Same pattern as `FarmerHome`/`MinerHome`.
- **Woodcutter/Quarrier**: `NpcDef` entries in `NPC_REGISTRY`. One `Job` variant + one registry entry each.
- **Wood/Stone**: `ResourceKind` enum variants + `WoodStore`/`StoneStore` ECS components (same pattern as `FoodStore`/`GoldStore`).

No new entity types, no parallel arrays, no god-structs.

## Node model

Each resource node is a regular building:

- `BuildingKind::TreeNode` / `BuildingKind::RockNode`
- Not player-buildable (`player_buildable: false`)
- Neutral faction (`FACTION_NEUTRAL`), no town ownership (`TOWN_NONE`)
- Spawned by worldgen on biome-matching cells
- One harvest cycle destroys the node permanently -- no HP tracking, no depleted state, no regrowth
- GPU instanced rendering through the standard building pipeline
- Clickable in inspector (shows type and yield)
- Saved/loaded as buildings in EntityMap

## Worldgen

Spawn nodes during `generate_world()` after terrain generation, same pattern as GoldMine (`world.rs:2109-2151`):

1. Iterate candidate positions within world bounds
2. Check biome: `TreeNode` only on `Biome::Forest` cells, `RockNode` only on `Biome::Rock` cells
3. Min spacing between nodes (configurable constants: `TREE_MIN_SPACING`, `ROCK_MIN_SPACING`)
4. Skip cells that already have a building (`entity_map.has_building_at()`)
5. Snap to grid center via `grid.world_to_grid()` / `grid.grid_to_world()`
6. Place via `place_building()`

Config fields on `WorldGenConfig`:

- `tree_density: f32` -- fraction of Forest cells that get a TreeNode (e.g. 0.3)
- `rock_density: f32` -- fraction of Rock cells that get a RockNode (e.g. 0.2)

Density is approximate -- min-spacing and occupied-cell checks reduce actual count.

Terrain rendering is unchanged -- biome tiles still render underneath. Node entities render on top as building sprites.

## Rendering

Each node type gets a `TileSpec` entry in its `BuildingDef`. Use existing roguelike sprite sheet positions or `TileSpec::External` for custom art.

Nodes render identically to other buildings -- no special rendering path.

## Harvester buildings

Two new spawner buildings:

| Kind | Worker job | Target node | Workers per building |
|------|-----------|-------------|---------------------|
| `LumberMill` | `Woodcutter` | `TreeNode` | 1 |
| `Quarry` | `Quarrier` | `RockNode` | 1 |

Player-buildable. Same spawner pattern as `FarmerHome`/`MinerHome`: `BuildingDef` with `spawner: Some(SpawnerDef { job, count: 1 })`.

## NPC jobs

Two new `Job` variants with `NpcDef` entries in `NPC_REGISTRY`:

| Job | Base stats | Notes |
|-----|-----------|-------|
| `Woodcutter` | Similar to Farmer (non-combat worker) | Spawned by LumberMill |
| `Quarrier` | Similar to Farmer (non-combat worker) | Spawned by Quarry |

## Harvest loop

New `ActivityKind` variants: `Chop`, `Quarry`. Registered in `ACTIVITY_REGISTRY`.

Lifecycle (same shape as Mine):

```
Idle -> Chop(Transit) -> Chop(Holding) -> ReturnLoot(Transit) -> ReturnLoot(Holding) -> Idle
```

1. **Idle**: decision system scores Chop/Quarry based on nearest available node
2. **Chop/Quarry(Transit)**: walk to nearest unoccupied TreeNode/RockNode (use `find_nearest_free()` pattern from MinerHome targeting)
3. **Chop/Quarry(Holding)**: work at node, accumulate resource over time (same tick pattern as mine tending)
4. **Harvest complete**: call `destroy_building()` on the node -- node disappears permanently. NPC gains resource in carried loot.
5. **ReturnLoot(Transit)**: walk home
6. **ReturnLoot(Holding)**: deposit wood/stone to town storage
7. Return to idle, seek next node

Worker targeting uses `work_targeting.rs` to assign workers to nearest available node. When a node is destroyed, worker returns to idle and seeks a new node on next decision tick.

Worksite occupancy uses existing `NpcWorkState.worksite` pattern -- one worker per node at a time.

## Resource storage

Add `Wood` and `Stone` to `ResourceKind` enum (`constants/upgrades.rs`).

New ECS components on town entities:

- `WoodStore(i32)` -- wood held by this town
- `StoneStore(i32)` -- stone held by this town

Same pattern as `FoodStore(i32)` / `GoldStore(i32)`.

Deposit: ReturnLoot holding phase adds carried resource to the appropriate store component.

UI: show wood/stone counts in the top bar stats panel alongside food/gold.

## Pathfinding

Resource nodes do NOT block pathfinding:

- `BuildingDef.blocks_path: false` (same as farms, roads)
- Forest biome cost (143 vs 100 for grass) already slows movement through forests
- Rock biome cost (2500) already penalizes rock terrain

## Save/load

- Nodes are regular buildings -- saved and loaded through existing building serialization
- No migration needed for old saves (nodes only exist in newly generated worlds)
- Old saves without nodes continue to work -- forests/rocks remain as terrain only

## Out of scope

- Spending wood/stone on building costs (future slice)
- Iron ore, blacksmith crafting (Stage 26 later items)
- Node regrowth or replenishment
- Deforestation visual effects on terrain
- Villager job assignment UI

## Slices

1. **Spec + registry** (this doc): define the model, add `BuildingKind::TreeNode`/`RockNode` + `ActivityKind::Chop`/`Quarry` + `Job::Woodcutter`/`Quarrier` to registries with sprites
2. **Worldgen + rendering**: spawn nodes during world generation, render as buildings
3. **Harvest loop**: LumberMill/Quarry buildings, Chop/Quarry activities, work targeting, `destroy_building()` on harvest, town storage (WoodStore/StoneStore), top bar UI
4. **Save/load + tests**: persistence verification, miner-cycle-style integration test for woodcutter/quarrier
