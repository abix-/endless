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
- [ ] Building construction time: 10s at 1x game speed (scales with time_scale), building is inert during construction
  - `ConstructionQueue` resource with `ConstructionEntry` (position, total/remaining secs, spawner_idx, growth_idx)
  - `build_and_pay()` passes `respawn_timer = -1.0` (dormant) to `register_spawner()` and pushes a `ConstructionEntry`
  - `construction_tick_system` (every frame): count down `remaining -= delta * time_scale`, on completion set spawner timer to `0.0` / unfreeze farm growth
  - Farms: add `under_construction: Vec<bool>` to `GrowthStates`, skip in `growth_system` when true
  - Spawner buildings (homes/tents): NPC doesn't spawn until construction completes (spawner stays dormant at `-1.0`)
  - Guard post turrets already start disabled — no extra suppression needed
  - Blue progress bar above building during construction via `ExtractResource` render data (same pattern as `BuildingHpRender`)
  - Render in `npc_render.rs` prepare function as misc instances (`atlas_id: 6.0`, blue tint, `health` = progress pct)
  - AI builds (`ai_player.rs`) use same `build_and_pay()` path — construction delay applies to AI too
  - Files: `constants.rs` (BUILDING_CONSTRUCT_SECS), `resources.rs` (ConstructionQueue + GrowthStates.under_construction), `world.rs` (build_and_pay param), `systems/economy.rs` (new system + growth_system guard), `ui/mod.rs` + `systems/ai_player.rs` (pass queue), `lib.rs` + `gpu.rs` (register + extract), `npc_render.rs` (render bars)

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
- [x] Add shared `WorldState` bundle in `resources.rs` for world/grid/building mutation paths.
- [x] Adopt `WorldState` in high-churn systems: `ai_decision_system`, `building_damage_system`, `sync_guard_post_slots`, `build_place_click_system`, `process_destroy_system`, `game_startup_system`, `game_cleanup_system`, `migration_settle_system`, `process_upgrades_system`.
- [x] Add shared `EconomyState` bundle in `resources.rs` for food/gold/mine/events/population mutation.
- [x] Adopt `EconomyState` in core paths: `decision_system`, `arrival_system`, `camp_forage_system`, `process_upgrades_system`.
- [ ] Create `GameLog` bundle in `resources.rs`: `{ combat_log: ResMut<CombatLog>, game_time: Res<GameTime>, timings: Res<SystemTimings> }` and migrate systems still carrying this triple directly.
- [ ] Move/replace remaining ad-hoc bundles in `systems/behavior.rs` (keep only bundles with genuine local-only value; shared bundles live in `resources.rs`).
- [ ] Keep bundles flat (no nested `SystemParam` bundles inside other bundles) unless required to break Bevy param-count limits.
- [ ] Re-baseline and document actual parameter-count reductions after refactor, then verify with `cargo check`.

**Stage 14b: AI Expansion & Mine Control**

*Done when: AI towns grow beyond their starting 7×7 grid, compete for gold mines via patrol routes, and a passive AI that doesn't expand gets outcompeted and dies.*

Chunk 1 — AI expansion brain (`systems/ai_player.rs` only):
- [x] Add `miner_home_target()` to `AiPersonality` (Aggressive: houses/4, Balanced: houses/2, Economic: houses/1) — replace hardcoded `houses / 3`
- [x] Dynamic expansion priority: calculate slot fullness (used/total), multiply expansion upgrade weight (idx 15) by `2 + 4*(fullness-0.7)/0.3` when fullness > 0.7
- [x] Emergency expansion: when `!has_slots`, apply 10× multiplier to expansion weight so it dominates all other upgrades
- [x] Boost base expansion weights: Aggressive 4→8, Balanced 3→10, Economic 4→12

Chunk 2 — Disable turrets on waypoints (code preserved for future Tower building):
- [x] `attack_enabled` defaults to `false` on new waypoints (turret code preserved for future Tower)
- [x] UI: build menu shows "Patrol waypoint" help text, inspector hides turret toggle
- [x] Full rename GuardPost → Waypoint across 35 files with serde back-compat aliases

Chunk 3 — Wilderness waypoint placement + AI territorial expansion:
- [x] `place_waypoint_at_world_pos()` in `world.rs` — snaps to grid cell, validates empty + not water, deducts food
- [x] Player wilderness placement: waypoint ghost snaps to world grid, build_place_click bypasses town grid
- [x] AI mine extension: `find_mine_waypoint_pos()` finds closest uncovered gold mine, `count_uncovered_mines()` scores urgency
- [x] AI waypoint scoring moved outside `has_slots` block — wilderness placement independent of town grid
- [x] AI execution: try mine position first, fallback to in-grid placement
- [ ] AI patrol routes automatically cover placed waypoints (PatrolRoute rebuild already handles this via `build_patrol_route`)

**Stage 14c: Generic Growth & Contestable Mines**

*Done when: mines grow gold like farms grow food (tended-only, 4-hour cycle), any faction's miner can harvest a ready mine, and FarmStates is generalized to GrowthStates handling both farms and mines.*

Refactor FarmStates → GrowthStates (see plan file `velvet-crunching-torvalds.md`):
- [ ] `GrowthStates` resource with `GrowthKind` enum (Farm/Mine), replaces both `FarmStates` and `MineStates`
- [ ] `push_farm(pos, town_idx)` / `push_mine(pos)` — mines have no town owner (contestable)
- [ ] `harvest()` generalized: Farm credits food to town, Mine credits `MINE_EXTRACT_PER_CYCLE` gold to harvester's town
- [ ] `growth_system` replaces both `farm_growth_system` and `mine_regen_system` — farms: passive + tended rates (unchanged), mines: tended-only (`MINE_TENDED_GROWTH_RATE` = 0.25/hr, 4 hours to ready)
- [ ] Miner behavior: walk to mine → claim occupancy → tend (accelerate growth) → harvest when Ready → return with gold. Same pattern as farmer but for gold.
- [ ] Mine progress bar rendered at mine position (atlas_id=6.0, gold color) via GrowthStates misc instance buffer — not on the miner
- [ ] Delete: `MineStates`, `MiningProgress`, `MinerProgressRender`, `sync_miner_progress_render`, `mine_regen_system`, `MINE_MAX_GOLD`, `MINE_REGEN_RATE`, `MINE_WORK_HOURS`
- [ ] Bulk rename `FarmStates` → `GrowthStates` across ~20 files (resources, systems, tests, UI, save)

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

