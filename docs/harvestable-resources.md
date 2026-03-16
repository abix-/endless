# Resource Gathering

All resource gathering uses one unified system: WorksiteDef + ProductionState + ActivityKind::Work. There are no per-resource activity kinds or separate gathering loops.

## Resource types

| Resource | Source | Node/Building kind | Storage | Worker building | Worker job | Growth | Destruction |
|----------|--------|-------------------|---------|----------------|------------|--------|-------------|
| Food | Biome | `Farm` | `FoodStore` | `FarmerHome` | `Farmer` | Timed (day only, tended=faster) | No (regrows) |
| Food (cow) | Building | `Farm` (cow mode) | `FoodStore` | `FarmerHome` | `Farmer` | Timed (always, food cost) | No (regrows) |
| Gold | Worldgen | `GoldMine` | `GoldStore` | `MinerHome` | `Miner` | Timed (tended only) | No (regrows) |
| Wood | Forest biome | `TreeNode` | `WoodStore` | `LumberMill` | `Woodcutter` | Instant (always ready) | Yes (one-shot) |
| Stone | Rock biome | `RockNode` | `StoneStore` | `Quarry` | `Quarrier` | Instant (always ready) | Yes (one-shot) |

Iron is out of scope.

## Unified architecture

Every worksite follows the same pipeline:

```
BuildingDef.worksite: WorksiteDef    -- registry config (yield_item, one_shot, etc.)
ProductionState                      -- ECS component (ready flag, growth progress)
growth_system                        -- controller: advances progress -> sets ready
decision_system (ActivityKind::Work) -- controller: NPC claims worksite, takes yield when ready
behavior_system (ReturnLoot)         -- controller: NPC deposits carried_loot to town store
```

### WorksiteDef (registry -- Def layer)

```rust
pub struct WorksiteDef {
    pub max_occupants: i32,
    pub drift_radius: f32,
    pub upgrade_job: &'static str,
    pub yield_item: ResourceKind,
    pub town_scoped: bool,
    pub one_shot: bool,  // true = destroy entity after yield (TreeNode, RockNode)
}
```

### ProductionState (ECS -- Instance layer)

All worksites get `ProductionState` on spawn (already implemented). For one-shot nodes, `growth_system` sets `ready = true` immediately. For farms/mines, growth is timed.

### growth_system (Controller)

Handles ALL worksite types via a single `match building.kind` block:
- `Farm`: timed growth, day-only for crops, always for cows
- `GoldMine`: timed growth, requires occupant
- `TreeNode | RockNode`: instant ready (always available when awake)

### decision_system -- Work activity (Controller)

One `ActivityKind::Work` handles all resource jobs. The flow:

1. **Idle -> Work(Transit)**: decision_system selects nearest worksite via `WorkIntent::Claim`
2. **Work(Transit) -> Work(Active)**: NPC walks to worksite, arrives, begins working
3. **Work(Active) -> yield**: when `ProductionState.ready`, take yield via `ProductionState::take_yield()`, add to `carried_loot` via `WorksiteDef.yield_item` match
4. **Post-yield**: if `WorksiteDef.one_shot`, destroy the worksite entity. Release worksite claim.
5. **Work -> ReturnLoot(Transit)**: NPC walks home with carried resources
6. **ReturnLoot -> deposit**: `behavior_system` deposits all carried_loot fields (food, gold, wood, stone) to town stores

No separate `ActivityKind::Chop` or `ActivityKind::Quarry`. All workers use `ActivityKind::Work`.

### carried_loot writeback

The decision_system writeback block must persist ALL carried_loot fields (food, gold, wood, stone), not just a subset. The generic worksite path writes to the correct field via `WorksiteDef.yield_item` -> `ResourceKind` match.

## CRD compliance

All types follow the Def -> Instance -> Controller pattern (see `docs/k8s.md`):

- **TreeNode/RockNode**: `BuildingDef` entries in `BUILDING_REGISTRY` with `worksite: Some(WorksiteDef { one_shot: true, ... })`. Spawned via `place_building()`, destroyed after yield.
- **LumberMill/Quarry**: `BuildingDef` entries with `spawner: Some(SpawnerDef { job, count: 1 })`. Same pattern as `FarmerHome`/`MinerHome`.
- **Woodcutter/Quarrier**: `NpcDef` entries in `NPC_REGISTRY`. One `Job` variant + one registry entry each.
- **Wood/Stone**: `ResourceKind` enum variants + `WoodStore`/`StoneStore` ECS components (same pattern as `FoodStore`/`GoldStore`).

Adding a new resource type requires: 1 BuildingKind + 1 registry entry + 1 ResourceKind variant + 1 Store component + 1 Job + 1 NPC registry entry + growth_system match arm. No new activity kinds, no new loops, no new writeback paths.

## Node model

Each resource node is a regular building:

- `BuildingKind::TreeNode` / `BuildingKind::RockNode`
- Not player-buildable (`player_buildable: false`)
- Neutral faction, no town ownership
- Spawned by worldgen on biome-matching cells
- One work cycle destroys the node permanently (via `WorksiteDef.one_shot: true`)
- GPU instanced rendering through the standard building pipeline
- Saved/loaded as buildings in EntityMap

## Terrain

Resource nodes render on top of Grass terrain, regardless of the source biome. When a node is placed on a Forest or Rock cell, the underlying terrain should display as Grass so the node sprite is visible against a clean background. The node entity itself represents the tree/rock -- the biome tile underneath should not also show a tree/rock.

## Worldgen

Spawn nodes during `generate_world()` after terrain generation:

1. Iterate candidate positions within world bounds
2. Check biome: `TreeNode` only on `Biome::Forest` cells, `RockNode` only on `Biome::Rock` cells
3. Min spacing between nodes (`TREE_MIN_SPACING`, `ROCK_MIN_SPACING`)
4. Skip cells with existing buildings
5. Snap to grid center, place via `place_building()`

Config: `tree_density: f32`, `rock_density: f32` on `WorldGenConfig`.

## LOD rendering

When zoomed out, resource nodes render as colored LOD boxes instead of sprites. The LOD box color should match the resource type:

| Node | LOD color |
|------|-----------|
| `TreeNode` | Green (wood) |
| `RockNode` | Gray (stone) |
| `Farm` | Yellow (food) |
| `GoldMine` | Gold (gold) |

This uses the existing LOD distance threshold -- no new system needed, just a color lookup from the building kind or `WorksiteDef.yield_item`.

## Pathfinding

Resource nodes do NOT block pathfinding. Forest biome cost (143) and Rock biome cost (2500) already handle movement penalties.

## Save/load

Nodes are regular buildings -- saved and loaded through existing building serialization. No migration needed for old saves.

## Out of scope

- Spending wood/stone on building costs (future slice)
- Iron ore, blacksmith crafting (Stage 26)
- Node regrowth or replenishment
- Deforestation visual effects
