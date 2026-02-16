# Roadmap

Target: 20,000+ NPCs @ 60fps with pure Bevy ECS + WGSL compute + GPU instanced rendering.

## How to Maintain This Roadmap

1. **Stages are the priority.** Read top-down. First unchecked stage is the current sprint.
2. **No duplication.** Each work item lives in exactly one place. Stages have future work. [completed.md](completed.md) has done work. [specs/](specs/) has implementation detail.
3. **Completed checkboxes are accomplishments.** Never delete them. When a stage is done, move its `[x]` items to [completed.md](completed.md).
4. **"Done when" sentences don't change** unless the game design changes.
5. **New features** go in the appropriate future stage.

## Completed

See [completed.md](completed.md) - 267 items across 30 subsystems.

## Stages

Stages 1-13, 23: [x] Complete (see [completed.md](completed.md))

**Stage 14: Tension**

*Done when: a player who doesn't build or upgrade loses within 30 minutes - raids escalate, food runs out, town falls.*

- [ ] Raid escalation: wave size and stats increase every N game-hours
- [ ] Differentiate job base stats (raiders hit harder, archers are tankier, farmers are fragile)
- [x] Food consumption: NPCs eat hourly, eating restores HP/energy
- [x] Starvation effects: no food -> HP drain, speed penalty (desertion TBD)
- [ ] Loss condition: all town NPCs dead + no spawners -> game over screen
- [x] Building costs rebalanced (difficulty-scaled: Easy/Normal/Hard)

**Stage 15: Performance**

*Done when: `NpcGpuState` ExtractResource clone eliminated, and `command_buffer_generation_tasks` drops from ~10ms to ~1ms at default zoom on a 250x250 world.*

GPU extract optimization (see [specs/gpu-visual-direct-upload.md](specs/gpu-visual-direct-upload.md)):
- [x] Zero-clone GPU upload: `NpcGpuState` + `NpcVisualUpload` via `Extract<Res<T>>` + `queue.write_buffer()` (eliminates 6.4MB/frame clone)
- [x] Delete `ExtractResourcePlugin::<GpuReadState>` — was cloning ~1.2MB/frame to render world where nothing read it
- [x] `ProjBufferWrites` zero-clone: `Extract<Res<T>>` + `extract_proj_data` replaces both `write_proj_buffers` and `prepare_proj_buffers`; shared `write_dirty_f32`/`write_dirty_i32` helpers DRY with NPC extract

Chunked tilemap (see [specs/chunked-tilemap.md](specs/chunked-tilemap.md)):
- [ ] Split single 250x250 TilemapChunk per layer into 32x32 tile chunks
- [ ] Bevy frustum-culls off-screen chunk entities - only visible chunks generate draw commands
- [ ] `sync_building_tilemap` updates only chunks whose grid region changed, not all 62K+ tiles

Entity sleeping:
- [ ] Entity sleeping (Factorio-style: NPCs outside camera radius sleep)

GPU-native NPC rendering (see [specs/gpu-native-npc-rendering.md](specs/gpu-native-npc-rendering.md)):
- [x] Vertex shader reads positions/health directly from compute shader's `NpcGpuBuffers` storage buffers (bind group 2), eliminating CPU→GPU instance buffer rebuild
- [x] `NpcVisualBuffers` resource: `visual` `[f32;8]` per slot (sprite/flash/color) + `equip` `[f32;24]` per slot (6 layers), full-buffer uploaded per frame (V1)
- [x] `vertex_npc` shader entry point: instance offset encoding (`slot = instance_index % npc_count`, `layer = instance_index / npc_count`), `npc_count` in `CameraUniform`
- [x] One pipeline with `storage_mode` specialization key `(hdr, samples, storage_mode)`, two entry points (`vertex` / `vertex_npc`)
- [x] Farm sprites + building HP bars split to `NpcMiscBuffers` with `RawBufferVec<InstanceData>` + `DrawMisc` command
- [ ] Throttle readback: factions every 60 frames, threat_counts every 30 frames, `buffer_range()` sized to `npc_count`
- [ ] Pre-allocate `GpuReadState` vecs and `copy_from_slice` instead of per-frame `Vec` allocation (GpuReadState extraction already deleted)

Every-frame review backlog:
- [x] `prepare_proj_buffers` deleted — merged into `extract_proj_data` (ExtractSchedule, zero-clone)
- [x] `ProjPositionState` + `GpuReadState` extraction eliminated — zero-clone or not extracted
- [ ] `decision_system`: reduce per-frame allocation/log pressure in hot paths (avoid unconditional `format!`/log string churn for high-N NPC loops; gate expensive logs behind debug/selection or lower-frequency sampling).
- [ ] `damage_system` debug stats: gate `query.iter().count()` and sample collection behind debug flag to avoid unconditional extra iteration each frame.
- [ ] `top_bar_system` HUD counts: replace repeated `spawner_state` full scans with cached/incremental counters.
- [ ] `sync_building_hp_render`: rebuild only when `BuildingHpState`/`WorldData` changes (or via dirty flag), not every frame.
- [x] Gate `rebuild_building_grid_system` so `BuildingSpatialGrid::rebuild()` only runs when world/building data changes (or via dirty flag), not every frame.
- [x] Replace `decision_system` threat checks (`count_nearby_factions`) with GPU spatial grid query â€” piggybacks on existing Mode 2 combat targeting scan, packed u32 readback (enemies<<16|allies)
- [x] Optimize `healing_system` town-zone checks (faction-indexed town lists / cached radii) to reduce per-frame NPC x town iteration.
- [x] Optimize `guard_post_attack_system` target acquisition to avoid full guard-post x NPC scans on fire-ready ticks. Option D: guard posts get NPC `SlotAllocator` indices; GPU spatial grid auto-populates `combat_targets[gp_slot]`. `sync_guard_post_slots` (dirty-flag gated) allocates/frees slots; attack system reads one array index per post — O(1).
- [x] Make combat log UI incremental (cache merged entries and skip per-frame full rebuild/sort when source logs are unchanged).
- [x] DirtyFlags lifecycle hardening: eliminate stale cache/flag carry-over across state transitions and load paths.
  - All load/startup/cleanup paths reset via `*dirty = DirtyFlags::default()` (ui/mod.rs game_startup, game_load, game_cleanup + save.rs load_game).
  - `game_cleanup_system` clears `HealingZoneCache.by_faction`.
- [ ] DirtyFlags regression tests (state transitions + load): add automated coverage for cleanup/enter behavior.
  - Add tests (likely in `tests/vertical_slice.rs` or dedicated `tests/dirty_flags.rs`) that exercise:
    1. `OnExit(AppState::Playing)` cleanup resets `DirtyFlags` and clears `HealingZoneCache`.
    2. `OnEnter(AppState::Playing)` startup path sets `dirty.healing_zones = true`.
    3. Menu-load and in-game load both set `dirty.healing_zones = true` and clear `dirty.patrol_swap`.
    4. `update_healing_zone_cache` rebuilds then clears `dirty.healing_zones`.
  - Done when tests fail on current bug states and pass after fixes, guarding against regressions.
- [x] Change `squad_cleanup_system` from always-on per-frame maintenance to event/interval-driven updates keyed to membership/spawn/death changes.
- [ ] Narrow `on_duty_tick_system` workset so only on-duty archers are iterated each frame.
- [ ] Remove linear HP lookup in inspector rendering (`bottom_panel_system`) by using direct selected-NPC lookup/cached handle.

SystemParam bundle consolidation:
- [ ] Create `GameLog` bundle in `resources.rs`: `{ combat_log: ResMut<CombatLog>, game_time: Res<GameTime>, timings: Res<SystemTimings> }`. This triple appears in 8+ systems: `arrival_system`, `spawn_npc_system`, `death_cleanup_system`, `spawner_respawn_system`, `ai_decision_system`, `xp_grant_system`, `healing_system`, `process_upgrades_system`. Each system drops its 3 direct params in favor of `log: GameLog`.
- [ ] Move `FarmParams` and `EconomyParams` from `systems/behavior.rs` to `resources.rs` (they're `pub` but only imported by behavior.rs today). Update imports in behavior.rs.
- [ ] Adopt `FarmParams` + `EconomyParams` in `arrival_system` (13->8 params): replace direct `farm_states`, `world_data`, `food_storage`, `gold_storage`, `food_events` with the two bundles. Access via `farms.states`, `economy.food_storage`, etc.
- [ ] Do NOT refactor systems where `WorldData` mutability mismatches - `ai_decision_system` and `build_menu_system` need `ResMut<WorldData>` but `FarmParams` has `Res<WorldData>`. Leave those as-is.
- [ ] Do NOT nest bundles (e.g. `GameLog` inside `DecisionExtras`). Flat bundles only.
- [ ] Expected param count reductions: `arrival_system` 13->8, `spawn_npc_system` 15->13, `death_cleanup_system` 9->7, `spawner_respawn_system` 9->7, `ai_decision_system` 15->13, `xp_grant_system` 10->8, `healing_system` 10->8, `process_upgrades_system` 10->9. Pure refactor - no behavioral changes. Verify with `cargo check`.

**Stage 16: Combat Depth**

*Done when: two archers with different traits fight the same raider noticeably differently - one flees early, the other berserks at low HP.*

- [ ] Unify `TraitKind` (4 variants) and `trait_name()` (9 names) into single 9-trait Personality system
- [ ] All 9 traits affect both `resolve_combat_stats()` and `decision_system` behavior weights
- [ ] Trait combinations (multiple traits per NPC)
- [ ] Target switching (prefer non-fleeing enemies, prioritize low-HP targets)
- [ ] Squad behavior: add option for squad-assigned archers to ignore patrol responsibilities
- [ ] When "Ignore Patrol" is enabled, archers with `SquadId` must never enter `OnDuty`/patrol route flow; they only follow squad target (or squad-idle behavior) while still respecting survival rules (combat/flee/rest/heal)

**Stage 16b: NPC Skills & Proficiency** (see [specs/npc-skills.md](specs/npc-skills.md))

*Done when: two NPCs with the same job but different proficiencies produce measurably different outcomes (farm output, combat effectiveness, dodge/survival), and those differences are visible in UI.*

- [ ] Add per-NPC skill set with proficiency values (0-100) keyed by role/action
- [ ] Skill growth from doing the work (farming raises farming, combat raises combat, dodging raises dodge)
- [ ] Proficiency modifies effectiveness:
- [ ] Farming proficiency affects farm growth/harvest efficiency
- [ ] Combat proficiency affects attack efficiency (accuracy/damage/cooldown contribution)
- [ ] Dodge proficiency affects projectile avoidance / survival in combat
- [ ] Render skill/proficiency details in inspector + roster sorting/filtering support
- [ ] Keep base-role identity intact (job still determines behavior class; proficiency scales effectiveness)

**Stage 17: Walls & Defenses**

*Done when: player builds a stone wall perimeter with a gate, raiders path around it or attack through it, chokepoints make guard placement strategic.*

- [ ] Wall building type (straight segments on grid, connects to adjacent walls)
- [ ] Wall HP + raiders attack walls blocking their path to farms
- [ ] Gate building (walls with a passthrough that friendlies use, raiders must breach)
- [ ] Pathfinding update: raiders route around walls to find openings, attack walls when no path exists
- [ ] Guard towers (upgrade from guard post - elevated, +range, requires wall adjacency)

**Stage 18: Save/Load**

*Done when: player builds up a town for 20 minutes, quits, relaunches, and continues exactly where they left off - NPCs in the same positions, same HP, same upgrades, same food.*

- [x] Serialize full game state (WorldData, SpawnerState, TownUpgrades, TownPolicies, FoodStorage, GameTime, NPC positions/states/stats)
- [x] F5 quicksave / F9 quickload with JSON serialization
- [x] Toast notification ("Game Saved" / "Game Loaded") with fade
- [x] Load from main menu (currently in-game only)
- [x] Autosave every N game-hours (3 rotating slots, configurable interval on main menu)
- [ ] Save slot selection (3 slots)

**Stage 19: Loot & Equipment**

*Done when: raider dies -> drops loot bag -> archer picks it up -> item appears in town inventory -> player equips it on an archer -> archer's stats increase and sprite changes.*

- [ ] `LootItem` struct: slot (Weapon/Armor), stat bonus (damage% or armor%)
- [ ] Raider death -> chance to drop `LootBag` entity at death position (30% base rate)
- [ ] Archers detect and collect nearby loot bags (priority above patrol, below combat)
- [ ] `TownInventory` resource, inventory UI tab
- [ ] `Equipment` component: weapon + armor slots, feeds into `resolve_combat_stats()`
- [ ] Equipped items reflected in NPC equipment sprite layers

**Stage 20: Tech Trees** (see [specs/tech-tree.md](specs/tech-tree.md))

*Done when: player spends Food or Gold to buy tech-tree upgrades with prerequisites (no research building), and branch progression visibly unlocks stronger nodes (e.g., ArcherHome Lv2 unlock path, Military damage tier path).*

Chunk 1: Prerequisites + Currency [x] (see [completed.md](completed.md))
Chunk 2: Per-NPC-Type Redesign [x] (see [completed.md](completed.md))

Chunk 3: Energy Nodes
- [ ] Add `UpgradeType` variants: `MilitaryStamina`, `FarmerStamina`, `MinerStamina` (bump `UPGRADE_COUNT`)
- [ ] Wire into `energy_system`: per-town per-job drain modifier based on stamina upgrade level
- [ ] Prereqs: MilitaryStamina after MoveSpeed, FarmerStamina after FarmerMoveSpeed, MinerStamina after MinerMoveSpeed
- [ ] AI weights for new nodes

Chunk 4: Player AI Manager
- [ ] Tech-tree unlock node for `Player AI Manager`
- [ ] `PlayerAiManager` resource: `unlocked`, `enabled`, `build_enabled`, `upgrade_enabled`
- [ ] Reuse `AiKind::Builder` decision logic for faction 0 town, gated by unlock + toggle
- [ ] UI: hidden until unlocked, then show enable toggle + build/upgrade toggles + status label

**Stage 21: Economy Depth**

*Done when: player must choose between feeding NPCs and buying upgrades - food is a constraint, not a score.*

- [ ] HP regen tiers (1x idle, 3x sleeping, 10x fountain)
- [ ] FoodEfficiency upgrade wired into `decision_system` eat logic
- [ ] Economy pressure: upgrades cost more food, NPCs consume more as population grows

**Stage 22: Diplomacy**

*Done when: a raider camp sends a messenger offering a truce for 3 food/hour tribute - accepting stops raids, refusing triggers an immediate attack wave.*

- [ ] Camp reputation system (hostile -> neutral -> friendly, based on food tribute and combat history)
- [ ] Tribute offers: camps propose truces at reputation thresholds
- [ ] Trade routes between player towns (send food caravan from surplus town to deficit town)
- [ ] Allied camps stop raiding, may send fighters during large attacks
- [ ] Betrayal: allied camps can turn hostile if tribute stops or player is weak

**Stage 24: Resources & Jobs**

*Done when: player builds a lumber mill near Forest tiles, assigns a woodcutter, collects wood, and builds a stone wall using wood + stone instead of food - multi-resource economy with job specialization.*

- [ ] Resource types: wood (Forest biome), stone (Rock biome), iron (ore nodes, rare)
- [ ] Harvester buildings: lumber mill, quarry (same spawner pattern as FarmerHome/ArcherHome, 1 worker each)
- [ ] Resource storage per town (like FoodStorage but for each type - gold already done via GoldStorage)
- [ ] Building costs use mixed resources (walls=stone, archer homes=wood+stone, upgrades=food+iron, etc.)
- [ ] Crafting: blacksmith building consumes iron -> produces weapons/armor (feeds into Stage 19 loot system)
- [ ] Villager job assignment UI (drag workers between roles - farming, woodcutting, mining, smithing, military)

**Stage 25: Armies & Marching**

*Done when: player recruits 15 archers into an army, gives a march order to a neighboring camp, and the army walks across the map as a formation - arriving ready to fight.*

- [ ] Army formation from existing squads (select squad -> "Form Army" -> army entity with member list)
- [ ] March orders: right-click map location -> army walks as group (use existing movement system, group speed = slowest member)
- [ ] Unit types via tech tree unlocks: levy (cheap, weak), archer (ranged), men-at-arms (tanky, expensive)
- [ ] Army supply: marching armies consume food from origin town's storage, starve without supply
- [ ] Field battles: two armies in proximity -> combat triggers (existing combat system handles it)

**Stage 26: Conquest**

*Done when: player marches an army to a raider camp, defeats defenders, and claims the town - camp converts to player-owned town with buildings intact, player now manages two towns.*

- [ ] Camp/town siege: army arrives at hostile settlement -> attacks defenders + buildings
- [ ] Building HP: walls have HP - attackers must breach defenses (archer homes/farmer homes HP already done)
- [ ] Town capture: all defenders dead + town center HP -> 0 = captured -> converts to player town
- [ ] AI expansion: AI players can attack each other and the player (not just raid - full conquest attempts)
- [ ] Victory condition: control all settlements on the map

**Stage 27: World Map**

*Done when: player conquers all towns on "County of Palm Beach", clicks "Next Region" on the world map, and starts a new county with harder AI and more camps - campaign progression.*

- [ ] World map screen: grid of regions (counties), each is a separate game map
- [ ] Region difficulty scaling (more camps, tougher AI, scarcer resources)
- [ ] Persistent bonuses between regions (tech carries over, starting resources from tribute)
- [ ] "Country" = set of regions. "World" = set of countries. Campaign arc.

**Stage 28: Tower Defense (Wintermaul Wars-inspired)**

*Done when: player builds towers in a maze layout to shape enemy pathing, towers have elemental types with rock-paper-scissors counters, income accrues with interest, and towers upgrade/evolve into advanced forms.*

Maze building:
- [ ] Open-field tower placement on a grid (towers block pathing, enemies path around them)
- [ ] Pathfinding recalculation on tower place/remove (A* or flow field on grid)
- [ ] Maze validation - path from spawn to goal must always exist (reject placements that fully block)
- [ ] Visual path preview (show calculated enemy route through current maze)

Elemental rock-paper-scissors:
- [ ] `Element` enum: Fire, Ice, Nature, Lightning, Arcane, Dark (6 elements)
- [ ] Element weakness matrix (Fire->Nature->Lightning->Ice->Fire, Arcane<->Dark)
- [ ] Creep waves carry an element - weak-element towers deal 2x, strong-element towers deal 0.5x
- [ ] Tower/creep element shown via tint or icon overlay

Income & interest:
- [ ] Per-wave gold income (base + bonus for no leaks)
- [ ] Interest on banked gold each wave (5% per round, capped)
- [ ] Leak penalty - lives lost per creep that reaches the goal

Sending creeps:
- [ ] Spend gold to send extra creeps into opponent's lane
- [ ] Send menu with creep tiers (cheap/fast, tanky, elemental, boss)
- [ ] Income bonus from sending (reward aggressive play)

Tower upgrades & evolution:
- [ ] Multi-tier upgrade path (Lv1 -> Lv2 -> Lv3, increasing stats + visual change)
- [ ] At max tier, evolve into specialized variants (e.g. Fire Lv3 -> Inferno AoE or Sniper Flame)
- [ ] Evolved towers get unique abilities (slow, DoT, chain lightning, lifesteal)

Sound (bevy_audio) should be woven into stages as they're built - not deferred to a dedicated stage.

## Backlog

### DRY & Single Source of Truth
- [ ] Replace hardcoded town indices in HUD (player/camp assumptions) with faction/town lookup helpers
- [ ] Add regression tests that enforce no behavior drift between player and AI build flows, startup and respawn flows, and both destroy entry points

### UI & UX
- [ ] Persist left panel UI state (active tab + expanded/collapsed sections) in `UserSettings`
- [ ] Add `show_active_radius` debug toggle in Bevy UI
- [ ] Upgrade tab town snapshot: show `farmers/archers/farms/next spawn` summary
- [ ] Combat log window sizing: allow resize + persist width/height in `UserSettings`
- [ ] Guard post turret upgrades: per-post `range_level`, `damage_level` with food costs
- [ ] HP bar display mode toggle (Off / When Damaged / Always)
- [ ] Combat log scope/timestamp modes (Off/Own/All + Off/Time/Day+Time)
- [ ] Double-click locked slot to unlock (alternative to context action)
- [ ] Terrain tile click inspector (biome/tile coordinates)

## Specs

Implementation guides for upcoming stages. Once built, spec content rolls into regular docs and the spec file is deleted.

| Spec | Stage | File |
|---|---|---|
| GPU Extract Optimization | 15 | [specs/gpu-extract-optimization.md](specs/gpu-extract-optimization.md) |
| Chunked Tilemap | 15 | [specs/chunked-tilemap.md](specs/chunked-tilemap.md) |
| Tech Tree (Chunks 3-4) | 20 | [specs/tech-tree.md](specs/tech-tree.md) |
| NPC Skills & Proficiency | 16b | [specs/npc-skills.md](specs/npc-skills.md) |
| GPU-Native NPC Rendering | 15 | [specs/gpu-native-npc-rendering.md](specs/gpu-native-npc-rendering.md) |

## Performance

| Milestone | NPCs | FPS | Status |
|-----------|------|-----|--------|
| CPU Bevy | 5,000 | 60+ | [x] |
| GPU physics | 10,000+ | 140 | [x] |
| Full behaviors | 10,000+ | 140 | [x] |
| Combat + projectiles | 10,000+ | 140 | [x] |
| GPU spatial grid | 10,000+ | 140 | [x] |
| Full game integration | 10,000 | 130 | [x] |
| Max scale tested | 50,000 | TBD | [x] buffers sized |
| Future (chunked tilemap) | 50,000+ | 60+ | Planned |

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) - GPU spatial grid approach
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash) - marker component pattern
- [Bevy Render Graph](https://docs.rs/bevy/latest/bevy/render/render_graph/) - compute + render pipeline
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) - sprite batching, per-layer draw queues
- [Factorio FFF #421](https://www.factorio.com/blog/post/fff-421) - entity update optimization, lazy activation

