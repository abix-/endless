# Harvestable Resources

Trees and rocks as persistent world objects that NPCs harvest for wood and stone.

## Goal

Forest and rock biome content transitions from terrain-only visuals to first-class world objects with identity, state, depletion, and visible change. Players see NPCs chopping trees and mining rocks, collecting resources that enter town storage.

## Resource types

| Resource | Source biome | Node kind | Storage component | Harvester building | Worker job |
|----------|-------------|-----------|-------------------|-------------------|------------|
| Wood | Forest | TreeNode | WoodStore | LumberMill | Woodcutter |
| Stone | Rock | RockNode | StoneStore | Quarry | Quarrier |

Iron is out of scope for the initial delivery.

## Node model

Each resource node is a building entity using the existing `BuildingKind` + `ProductionState` pattern:

- `BuildingKind::TreeNode` / `BuildingKind::RockNode`
- Not player-buildable (`player_buildable: false`)
- Spawned by worldgen on biome-matching cells
- Has HP representing remaining resource (e.g. TreeNode HP=5 means 5 harvests before depletion)
- `ProductionState` tracks current harvest progress (same pattern as farm growth)
- Depleted nodes transition to a "stump" or "rubble" visual state
- Depleted nodes remain as obstacles but yield nothing

## Worldgen

During world generation, for each cell:

- Forest biome: spawn a `TreeNode` with probability ~0.4 (not every forest cell gets a tree node -- some are just terrain)
- Rock biome: spawn a `RockNode` with probability ~0.3

Nodes are placed at grid-snapped positions like all buildings. They share the unified GPU slot pool.

Terrain rendering is unchanged -- the biome tile (forest/rock with grass composite) still renders underneath. The node entity renders on top as a building sprite.

## Harvest loop

Follows the existing Work activity pattern (Work + Transit + Worksite -> Work + Active + Worksite):

1. Harvester NPC assigned to LumberMill/Quarry building
2. Decision system picks nearest unoccupied TreeNode/RockNode as work target
3. NPC walks to node (Transit phase)
4. NPC harvests at node (Active/Holding phase) -- ticks ProductionState
5. When harvest tick completes: node loses 1 HP, NPC gains 1 resource unit in carried loot
6. NPC transitions to ReturnLoot, walks home, deposits resource in town storage
7. Repeat until node is depleted (HP=0)

Worksite occupancy uses the existing `NpcWorkState.worksite` pattern -- one worker per node at a time.

## Visual states

Each node has 3-4 visual states based on remaining HP fraction:

| State | HP fraction | Tree visual | Rock visual |
|-------|------------|-------------|-------------|
| Full | > 0.66 | Full tree | Full boulder |
| Partial | 0.33-0.66 | Smaller tree / missing branches | Cracked rock |
| Depleted | 0 | Stump | Rubble pile |

Visual state is derived from `health / max_health` in the GPU visual upload, same as building construction progress.

## Town storage

Add `WoodStore(i32)` and `StoneStore(i32)` as ECS components on town entities, following the existing `FoodStore(i32)` / `GoldStore(i32)` pattern. Display in the top HUD bar next to food and gold.

## Save/load

Resource nodes save/load using the existing building save pipeline. Node HP and ProductionState persist automatically since they use standard building components.

## Pathfinding

Resource nodes are NOT pathfinding obstacles. They occupy terrain cells visually but NPCs walk through them (same as farms). The biome terrain cost already handles routing preference.

## Out of scope (initial delivery)

- Spending wood/stone on building costs (future: mixed-resource economy)
- Iron ore nodes
- Blacksmith crafting
- Node regrowth / replenishment
- Deforestation visual effects on terrain

## Slices

1. **Spec + registry**: This doc + `BuildingKind::TreeNode` / `RockNode` in building registry with sprites
2. **Worldgen + rendering**: Spawn nodes during world generation, render with depletion states
3. **Harvest loop**: LumberMill/Quarry buildings, Woodcutter/Quarrier jobs, work targeting, town storage
4. **Save/load + tests**: Persistence verification, round-trip tests, scenario coverage
