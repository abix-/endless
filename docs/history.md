# Development History

This file keeps durable delivery notes that do not belong in the player-facing [completed.md](completed.md) snapshot or the current-state architecture docs.

Use this file for:

- retired stage summaries
- shipped migration notes
- intentional removals

Use the canonical system docs for current behavior. Use Git history for commit-by-commit detail.

## Major Stage Summaries

### Stage 16 and 16.5: Performance and ECS migration

Completed work in this phase includes:

- ECS became the authoritative home for NPC gameplay state; GPU remained movement authority only
- query-first runtime migration replaced broad HashMap lookups in hot loops
- worksite indexing, slot-indexed occupancy, and building-side ECS consolidation removed major scale bottlenecks
- GPU extract, GPU-native rendering, event-driven visual uploads, target dirty tracking, readback throttling, and candidate-driven healing all landed
- Bevy Messages replaced older dirty-flag and queue bridges in key paths

Current docs:

- [performance.md](performance.md)
- [authority.md](authority.md)
- [rendering.md](rendering.md)
- [messages.md](messages.md)
- [resources.md](resources.md)

### Stage 17: Combat depth and personality

Completed work in this phase includes:

- 7-axis personality replaced the older 4-trait model
- trait-driven combat stat and behavior weighting are both live
- squad ignore-patrol and target-stability fixes landed with the combat-depth pass

Current docs:

- [behavior.md](behavior.md)
- [combat.md](combat.md)

### Stage 18: Loot, equipment, and armory

Completed work in this phase includes:

- unified `CarriedLoot`
- `LootItem`, `Rarity`, and multi-slot `NpcEquipment`
- direct loot-to-carrier flow with home deposit to town inventory
- armory UI, merchant inventory, auto-equip, and save/load persistence
- equipment death drops and visual equipment rendering

Current docs:

- [combat.md](combat.md)
- [armory-ui.md](armory-ui.md)
- [save-load.md](save-load.md)
- [rendering.md](rendering.md)

### Tech tree and Player AI Manager rollout

Completed work in this phase includes:

- prerequisite-gated, multi-resource tech tree purchases
- registry-driven upgrade metadata and UI
- stamina upgrades in the relevant job branches
- the Player AI Manager controls in the Policies tab

Current docs:

- [resources.md](resources.md)
- [ai-player.md](ai-player.md)

### Stage 19: Code health and testing

Completed work in this phase includes:

- CI for `cargo test` and `cargo clippy`
- unwrap audit in production code paths
- large file splits for left-panel, resources, and economy code
- test framework expansion across unit, system, and in-app integration coverage

Current docs:

- [README.md](README.md)
- [roadmap.md](roadmap.md)

### Stage 20 and 21: Pathfinding and walls

Completed work in this phase includes:

- HPA* pathfinding with LOS bypass, route spreading, and arrival parity
- incremental rebuilds on building changes
- core wall placement, HP, and auto-tiling

Current docs:

- [performance.md](performance.md)
- [authority.md](authority.md)
- [rendering.md](rendering.md)

### Stage 24: Save/load rollout

Completed work in this phase includes:

- full-state serialization
- F5 quicksave and F9 quickload
- main-menu load flow
- rotating autosaves
- save/load toast feedback

Current docs:

- [save-load.md](save-load.md)
- [ui.md](ui.md)

### Stage 32: CRD architecture

Completed work in this phase includes:

- Def -> Instance -> Controller adoption across major entity families
- NPC cleanup and building ECS decomposition
- `TownDef` and item registry work
- activity-controller reconciliation model

Current docs:

- [k8s.md](k8s.md)
- [npc-activity-controller.md](npc-activity-controller.md)
- [resources.md](resources.md)

## Historical Feature Notes

### UI and help

Shipped UI work that now lives in canonical docs includes:

- left-panel persistence for tabs and collapsible sections
- `HelpCatalog` tooltips and Help tab content
- centered armory modal replacing the old inventory-side-panel behavior
- save/load toast feedback and build-failure toasts
- wall-clock camera pan so camera speed no longer changes with simulation speed

Current docs:

- [ui.md](ui.md)
- [armory-ui.md](armory-ui.md)
- [rendering.md](rendering.md)

### Audio

Shipped audio work includes:

- jukebox music playback
- arrow shoot SFX
- 24 NPC death SFX variants
- spatial culling and per-kind dedup

Current docs:

- [audio.md](audio.md)

### Save/load-compatible migrations

Several large feature rollouts were delivered specifically with backward-compatible loading in mind, including squad state, loot and equipment persistence, AI state, and town-resource expansion. Current details now live in the current-state docs instead of a giant checklist.

Current docs:

- [save-load.md](save-load.md)
- [resources.md](resources.md)
- [ai-player.md](ai-player.md)

## Performance Milestones

Reached milestones include:

- 10K NPC full-game integration at high framerate during the early GPU era
- 30K NPC / 30K building passes after query-first and visual-upload work
- 50K NPC / 50K building validation after healing and render-path improvements

Current docs:

- [performance.md](performance.md)

## Intentional Removals

These items were intentionally dropped and should not be treated as missing coverage:

- Sprite atlas browser tool: removed with the Godot-era tooling cleanup and not needed in the Bevy runtime
- World-space town labels: not carried forward from the older scene-based UI approach

## Related Docs

- [completed.md](completed.md): player-facing feature snapshot
- [roadmap.md](roadmap.md): future work and active stages
