# Roadmap

Target: 30,000 NPCs + 30,000 buildings @ 60fps with pure Bevy ECS + WGSL compute + GPU instanced rendering.

## How to Maintain This Roadmap

1. **Stages are the priority.** Read top-down. First unchecked stage is the current sprint.
2. **No duplication.** Each work item lives in exactly one place. Stages have future work. [completed.md](completed.md) has done work. [specs/](specs/) has implementation detail.
3. **Completed checkboxes are accomplishments.** Never delete them. When a stage is done, move its `[x]` items to [completed.md](completed.md).
4. **"Done when" sentences don't change** unless the game design changes.
5. **New features** go in the appropriate future stage.
6. **Describe current state, not history.** Use present-tense behavior in docs; put historical wording and change chronology in [completed.md](completed.md) or `CHANGELOG.md`.

## Completed

See [completed.md](completed.md) for completed work moved out of active stages.

## Done Soon (Authority Safety)

- [ ] Combat authority hardening: remove `gpu_state.health` as a hard liveness gate in `attack_system`; validate NPC target liveness from ECS/EntityMap (`Dead`/Health components, `EntityMap::get_npc`) before clearing `ManualTarget` or rejecting attacks.
- [ ] Tower authority hardening: in `building_tower_system`, treat `GpuReadState.combat_targets` as candidate-only and re-validate target via ECS/EntityMap (`exists`, `!dead`, enemy faction) before spawning projectiles.
- [ ] Authority contract enforcement: align all docs to [authority.md](authority.md) and ensure no doc implies throttled readbacks (`factions`, `threat_counts`) are authoritative gameplay state.

## Stages

Stages 1-13, 19: [x] Complete (see [completed.md](completed.md))

**Stage 14: Tension**

*Done when: a player who doesn't build or upgrade loses within 30 minutes - raids escalate, food runs out, town falls.*

- [ ] Raid escalation: wave size and stats increase every N game-hours
- [ ] Differentiate job base stats (raiders hit harder, archers are tankier, farmers are fragile)
- [ ] Loss condition: all town NPCs dead + no spawners -> game over screen
- [ ] Building construction time: 10s at 1x game speed (scales with time_scale), building is inert during construction
  - `ConstructionQueue` resource with `ConstructionEntry` (position, total/remaining secs, spawner_idx)
  - `place_building()` passes `respawn_timer = -1.0` (dormant) and pushes a `ConstructionEntry`
  - `construction_tick_system` (every frame): count down `remaining -= delta * time_scale`, on completion set spawner timer to `0.0` / unfreeze farm growth
  - Farms: add `under_construction: bool` to `BuildingInstance`, skip in `growth_system` when true
  - Spawner buildings (homes/tents): NPC doesn't spawn until construction completes (spawner stays dormant at `-1.0`)
  - Guard post turrets already start disabled — no extra suppression needed
  - Blue progress bar above building during construction via `ExtractResource` render data (same pattern as `BuildingHpRender`)
  - Render in `npc_render.rs` prepare function as misc instances (`atlas_id: 6.0`, blue tint, `health` = progress pct)
  - AI builds (`ai_player.rs`) use same `place_building()` path — construction delay applies to AI too
  - Files: `constants.rs` (BUILDING_CONSTRUCT_SECS), `resources.rs` (ConstructionQueue + BuildingInstance.under_construction), `world.rs` (place_building param), `systems/economy.rs` (new system + growth_system guard), `ui/mod.rs` + `systems/ai_player.rs` (pass queue), `lib.rs` + `gpu.rs` (register + extract), `npc_render.rs` (render bars)

**Stage 15: Logistics & Flow**

*Done when: player builds a road from town to gold mine, zooms out, and watches farmers carrying food and miners carrying gold streaming along the road — visible supply chains on infrastructure the player designed.*

Farmer delivery and core roads complete (see [completed.md](completed.md)).

Roads (remaining):
- [ ] Road collision bypass: NPCs on road cells skip NPC-NPC separation force in compute shader (no bumping on roads = smooth traffic flow)
- [ ] Road connects visually to adjacent road tiles (auto-tiling: straight, corner, T-junction, crossroads — 4-bit neighbor mask → tileset index lookup)

AI road building:
- [ ] AI towns auto-build roads between fountain and farm clusters, mine routes, and waypoint paths
- [ ] AI road placement uses A* or straight-line between key building positions, placing `BuildingKind::Road` on each cell along the path
- [ ] Road building happens during AI build tick, costs food same as player (1 food/tile)
- [ ] Files: `systems/ai_player.rs` (road building logic)

**Stage 16: Performance**

*Done when: 30K NPCs + 30K buildings at 60fps. `NpcGpuState` ExtractResource clone eliminated, and `command_buffer_generation_tasks` drops from ~10ms to ~1ms at default zoom on a 250x250 world.*

GPU extract optimization and GPU-native NPC rendering complete (see [completed.md](completed.md)).
Linear scan elimination complete (see [completed.md](completed.md)).

Performance order at 30k NPC + 30k buildings (highest expected savings first):

1. [ ] [Critical] Replace global worksite scans in `decision_system` with indexed nearest lookup (per-kind/per-town candidate sets + bounded/ring spatial search), because current `iter_kind*` and `f32::MAX` nearest queries scale poorly at high building counts.
   Expected saving: ~8-16 ms/frame CPU in busy sim ticks (largest hot-path win).
2. [ ] [Critical] Move worksite occupancy hot-path from position-hash lookups to slot-indexed occupancy counters, because repeated hash lookups in high-N decision loops add avoidable CPU cost.
   Expected saving: ~2-6 ms/frame CPU when many workers are selecting/revalidating worksites.
3. [ ] [Critical] `build_visual_upload` triple inefficiency: (a) NPC loop uses `iter_npcs()` HashMap iteration instead of query-first, (b) building backfill scans all 60K entity slots instead of `iter_instances()` (~30K buildings only), (c) fills 1.92M sentinel floats every frame — track `prev_entity_count` and only fill new range.
   Files: `gpu.rs:299-423`. Expected saving: ~3-8 ms/frame CPU at 60K entities.
4. [ ] [Critical] Per-index dirty tracking for GPU targets buffer: `dirty_targets` triggers 480KB bulk upload every frame when only ~100-500 targets change. Add `target_dirty_indices: Vec<usize>` to `EntityGpuState`, use `write_dirty_f32` instead of `write_bulk` (same pattern as positions/arrivals).
   Files: `gpu.rs` (EntityGpuState + populate_gpu_state), `npc_render.rs:773` (extract). Expected saving: ~2-5 ms/frame GPU bandwidth.
5. [ ] [High] Remove per-NPC EntityMap HashMap lookups from `cooldown_system` and `energy_system`: both iterate 30K+ entities and do `entity_map.get_npc(slot)` per entity per frame (60K HashMap lookups total). Fix: add `Without<Dead>, Without<Building>` query filters, query `&Activity` directly in energy_system.
   Files: `combat.rs:19-49`, `energy.rs:15-41`. Expected saving: ~2-4 ms/frame CPU.
6. [ ] [High] `death_system` double `iter_npcs()` scan: Phase 1a and Phase 2b both do full HashMap iteration (60K iterations/frame even with zero deaths). Fix: query-first for Phase 1a, reuse Phase 1a dead_slots for Phase 2b.
   Files: `health.rs:148-179`. Expected saving: ~1-3 ms/frame CPU.
7. [ ] [High] Add decision-frame budgeting (max non-combat decisions per frame + adaptive interval by population), because fixed bucketing still allows expensive spikes at large NPC counts.
   Expected saving: ~2-5 ms/frame average and materially lower p95/p99 spikes.
8. [ ] [High] Gate `NpcLogCache` writes and `format!` churn behind debug/selection/sampling policy, because per-NPC string work in hot loops scales with population.
   Expected saving: ~1-3 ms/frame CPU (higher in debug-heavy sessions).
9. [ ] [High] Extend decision sub-profiling to cover squad sync, transit gates, and worksite selection scan counts, because current sub-timers under-report where frame time is spent.
   Expected saving: indirect only (enables targeting next 2-10 ms wins), near-zero immediate runtime win.
10. [ ] [Medium] Add cache-friendly vectors for hot building iteration paths (keep HashMaps as authority, vectors for tight loops), because data locality and branch predictability matter at 10k+ entities.
    Expected saving: ~1-3 ms/frame CPU on building-heavy ticks.
11. [ ] [Medium] `healing_system` iterates all 30K NPCs mutably every frame even though most aren't in healing zones. Consider faction-based pre-filter or spatial bucketing to reduce mutable query touchpoints.
    Files: `health.rs:485`. Expected saving: ~1-2 ms/frame CPU.
12. [ ] `decision_system`: reduce per-frame allocation/log pressure in hot paths (avoid unconditional `format!`/log string churn for high-N NPC loops; gate expensive logs behind debug/selection or lower-frequency sampling).
    Expected saving: ~1-2 ms/frame CPU; partly overlaps with log-cache gating item.
13. [ ] Entity sleeping (Factorio-style: NPCs outside camera radius sleep).
    Expected saving: highly scenario-dependent, ~3-12+ ms/frame CPU when most NPCs are off-camera; low gain if camera covers active area.
14. [ ] `sync_terrain_tilemap` updates only chunks whose grid region changed, not all chunks on every terrain dirty signal.
    Expected saving: ~0.5-2 ms/frame CPU on terrain-edit-heavy periods; minimal during steady-state.
15. [ ] Throttle readback: factions every 60 frames, threat_counts every 30 frames, `buffer_range()` sized to `npc_count`.
    Expected saving: ~0.3-1.2 ms/frame CPU/GPU sync overhead, plus reduced stall risk.
16. [ ] Pre-allocate `GpuReadState` vecs and `copy_from_slice` instead of per-frame `Vec` allocation.
    Expected saving: ~0.2-0.8 ms/frame CPU plus less allocator churn.
17. [ ] `sync_building_hp_render`: rebuild only when `BuildingHpState`/`WorldData` changes (or via dirty flag), not every frame.
    Expected saving: ~0.3-1.5 ms/frame CPU depending on building damage activity.
18. [ ] Narrow `on_duty_tick_system` workset so only on-duty archers are iterated each frame.
    Expected saving: ~0.2-1.0 ms/frame CPU depending on military population ratio.
19. [ ] `damage_system` debug stats: gate `query.iter().count()` and sample collection behind debug flag to avoid unconditional extra iteration each frame.
    Expected saving: ~0.1-0.6 ms/frame CPU in non-debug gameplay.
20. [ ] Perf anti-pattern remediation pass (UI + systems): remove repeated query scans in hot paths, pre-index slot/entity lookups once per frame/tick, replace nested `Vec::contains` membership checks with `HashSet`, and avoid per-item linear dedupe scans in overlays.
    Expected saving: broad/follow-up bucket, ~1-4 ms/frame total after targeted fixes.
21. [ ] SystemTimings Mutex contention: 20+ lock/unlock cycles per frame. Replace with AtomicU32 + f32::to_bits per slot.
    Expected saving: ~0.2-1.0 ms/frame CPU at high frame rates.
22. [ ] `NpcsByTownCache` uses `Vec<usize>` with `retain()` on death (O(n) per death, ~6K entries per town). Switch to `HashSet<usize>`.
    Expected saving: negligible per-frame but prevents worst-case spikes during mass death events.
23. [ ] Add perf guardrails for hot paths: microbenchmarks for inspector/squad/AI helper paths and CI thresholds that fail on material regressions.
    Expected saving: indirect only (prevents regressions, no direct runtime reduction).
24. [ ] Message signal regression tests: verify `emit_all()` fires on startup/load and drain systems consume correctly.
    Expected saving: correctness only, no direct runtime reduction.
- [x] Remove linear HP lookup in inspector rendering (`bottom_panel_system`) - replaced by direct `entity_map.get_npc(idx)` lookup.

ECS source-of-truth migration (plan: `~/.claude/plans/prancy-sauteeing-badger.md`):

ECS owns all NPC gameplay state. EntityMap is index-only (slot↔Entity, grid, kind/town/spatial).
No dual-writes. Hot loops use query-first + indexed lookup.
GPU is movement authority; ECS Position is read-model synced in `gpu_position_readback`.

- [x] Slice A: DirectControl + ManualTarget + Squad flags → ECS
  - `NpcFlags` component (direct_control + 4 other booleans), `SquadId` component, `ManualTarget` component
  - Files: components.rs, resources.rs, spawn.rs, render.rs, behavior.rs, combat.rs, save.rs, health.rs, economy.rs, ui/game_hud.rs
- [x] Slice B: Activity + Movement + Arrival + Position → ECS
  - `Activity` as ECS component, `Position` as ECS read-model (GPU movement authority), `NpcFlags.at_destination`
  - Query-first `gpu_position_readback`: iterates ECS archetypes, not HashMap
  - Files: components.rs, resources.rs, spawn.rs, movement.rs, behavior.rs, combat.rs, energy.rs, health.rs, economy.rs, render.rs, gpu.rs, save.rs, ui/game_hud.rs, ui/left_panel.rs, ui/roster_panel.rs, + 6 test files
- [x] Slice C: Combat + Health + Death → ECS
  - Health, Energy, Speed, CombatState, CachedStats, BaseAttackType, AttackTimer, LastHitBy → ECS components
  - NpcFlags.healing/starving replaces NpcInstance booleans
  - Query-first: healing_system, cooldown_system, energy_system, starvation_system, attack_system (with AttackQueries SystemParam bundle)
  - Files: components.rs, resources.rs, spawn.rs, combat.rs, health.rs, energy.rs, behavior.rs, economy.rs, stats.rs, gpu.rs, save.rs, ui/game_hud.rs, ui/left_panel.rs, ui/roster_panel.rs, + 10 test files
- [x] Slice D: Economy + AI + Save/Load + GPU + UI → ECS, delete NpcInstance
  - NpcInstance (40-field struct) replaced with NpcEntry (6-field index: slot, entity, job, faction, town_idx, dead)
  - Equipment, Personality, Home, PatrolRoute, CarriedGold, WorkPosition → ECS components
  - NpcFlags.migrating replaces NpcInstance.migrating
  - is_military/is_stealer → Job::is_military()/Job::Raider check
  - SaveNpcQueries SystemParam bundle for save/autosave
  - BuildingInspectorData extended with 7 ECS queries
  - MigrationResources extended with NpcFlags + Home queries
  - Files: resources.rs, spawn.rs, economy.rs, health.rs, stats.rs, ai_player.rs, behavior.rs, gpu.rs, render.rs, save.rs, game_hud.rs, left_panel.rs, + 4 test files

Scale remediation plan (30k NPC + 30k buildings):
- Items 1-2: Decision system worksite scan + occupancy counter optimization (Critical)
- Items 3-4: GPU pipeline — visual upload rewrite + targets dirty tracking (Critical)
- Items 5-6: Eliminate per-NPC HashMap lookups in cooldown/energy/death systems (High)
- Items 7-9: Decision budgeting + log gating + sub-profiling (High)
- Items 10-11: Cache-friendly building iteration + healing system pre-filter (Medium)
SystemParam bundle consolidation:
Current shared bundles include `DirtyWriters`, `AiDirtyReaders`, and `AiBuildRes`; remaining consolidation work is listed below.
- [ ] Create `GameLog` bundle: `{ combat_log: MessageWriter<CombatLogMsg>, game_time: Res<GameTime>, timings: Res<SystemTimings> }` and migrate systems still carrying this triple directly.
- [ ] Move/replace remaining ad-hoc bundles in `systems/behavior.rs` (keep only bundles with genuine local-only value; shared bundles live in `resources.rs`).
- [ ] Keep bundles flat (no nested `SystemParam` bundles inside other bundles) unless required to break Bevy param-count limits.

**Stage 16.5: Buildings as ECS Entities** (see [specs/buildings-as-entities.md](specs/buildings-as-entities.md))

*Done when: `EntityMap` is the sole source of truth for building infrastructure (`WorldData.buildings`, `PlacedBuilding`, tombstone guards are no longer active paths).*

Phases 1-2, EntityMap migration, WorldData deletion, GPU building buffers, and unified entity collision complete (see [completed.md](completed.md)).

Remaining:
- [ ] `WorldGrid.cells[].building` stores `Option<Entity>`

**Stage 17: AI Expansion & Mine Control**

*Done when: AI towns grow beyond their starting 7×7 grid, compete for gold mines via patrol routes, and a passive AI that doesn't expand gets outcompeted and dies.*

Chunks 1-3 complete (see [completed.md](completed.md)).

Remaining:
- [ ] AI patrol routes automatically cover placed waypoints (PatrolRoute rebuild already handles this via `build_patrol_route`)

**Stage 18: Generic Growth & Contestable Mines**

*Done when: mines grow gold like farms grow food (tended-only, 4-hour cycle), any faction's miner can harvest a ready mine, and growth is unified on BuildingInstance for both farms and mines.*

Growth field unification complete (see [completed.md](completed.md)).

Remaining:
- [ ] `harvest()` generalized: Farm credits food to town, Mine credits `MINE_EXTRACT_PER_CYCLE` gold to harvester's town
- [ ] `growth_system` replaces both `farm_growth_system` and `mine_regen_system` — farms: passive + tended rates (unchanged), mines: tended-only (`MINE_TENDED_GROWTH_RATE` = 0.25/hr, 4 hours to ready)
- [ ] Miner behavior: walk to mine → claim occupancy → tend (accelerate growth) → harvest when Ready → return with gold. Same pattern as farmer but for gold.
- [ ] Mine progress bar rendered at mine position (atlas_id=6.0, gold color) via EntityMap misc instance buffer — not on the miner
- [ ] Delete: `MineStates`, `MiningProgress`, `MinerProgressRender`, `sync_miner_progress_render`, `mine_regen_system`, `MINE_MAX_GOLD`, `MINE_REGEN_RATE`, `MINE_WORK_HOURS`

**Stage 20: Combat Depth**

*Done when: two archers with different traits fight the same raider noticeably differently - one flees early, the other berserks at low HP.*

- [ ] Unify `TraitKind` (4 variants) and `trait_name()` (9 names) into single 9-trait Personality system
- [ ] All 9 traits affect both `resolve_combat_stats()` and `decision_system` behavior weights
- [ ] Trait combinations (multiple traits per NPC)
- [ ] Target switching (prefer non-fleeing enemies, prioritize low-HP targets)
- [ ] Squad behavior: add option for squad-assigned archers to ignore patrol responsibilities
- [ ] When "Ignore Patrol" is enabled, archers with `SquadId` must never enter `OnDuty`/patrol route flow; they only follow squad target (or squad-idle behavior) while still respecting survival rules (combat/flee/rest/heal)
- [ ] Eliminate guard target oscillation between squad targets and patrol route posts (`OnDuty`/`Patrolling` conflict): enforce squad-target precedence, add no-spam target writes, and verify via `NpcTargetThrashDebug` sink counters (`SinkTargetChanges/s`, `SinkPingPong/s`)

**Stage 21: NPC Skills & Proficiency** (see [specs/npc-skills.md](specs/npc-skills.md))

*Done when: two NPCs with the same job but different proficiencies produce measurably different outcomes (farm output, combat effectiveness, dodge/survival), and those differences are visible in UI.*

- [ ] Add per-NPC skill set with proficiency values (0-100) keyed by role/action
- [ ] Skill growth from doing the work (farming raises farming, combat raises combat, dodging raises dodge)
- [ ] Proficiency modifies effectiveness:
- [ ] Farming proficiency affects farm growth/harvest efficiency
- [ ] Combat proficiency affects attack efficiency (accuracy/damage/cooldown contribution)
- [ ] Dodge proficiency affects projectile avoidance / survival in combat
- [ ] Render skill/proficiency details in inspector + roster sorting/filtering support
- [ ] Keep base-role identity intact (job still determines behavior class; proficiency scales effectiveness)

**Stage 22: Walls & Defenses**

*Done when: player builds a stone wall perimeter with a gate, raiders path around it or attack through it, chokepoints make guard placement strategic.*

Core wall system complete (see [completed.md](completed.md)).

Remaining:
- [ ] Wall auto-tiling (connect adjacent walls visually: straight, corner, T-junction, crossroads)
- [ ] Gate building (walls with a passthrough that friendlies use, raiders must breach)
- [ ] Pathfinding update: raiders route around walls to find openings, attack walls when no path exists
- [ ] Guard towers (upgrade from guard post - elevated, +range, requires wall adjacency)

**Stage 23: Save/Load**

*Done when: player builds up a town for 20 minutes, quits, relaunches, and continues exactly where they left off - NPCs in the same positions, same HP, same upgrades, same food.*

Core save/load shipped (see [completed.md](completed.md)).
- [ ] Save slot selection (3 slots)

**Stage 24: Loot & Equipment**

*Done when: raider dies -> drops loot bag -> archer picks it up -> item appears in town inventory -> player equips it on an archer -> archer's stats increase and sprite changes.*

- [ ] `LootItem` struct: slot (Weapon/Armor), stat bonus (damage% or armor%)
- [ ] Raider death -> chance to drop `LootBag` entity at death position (30% base rate)
- [ ] Archers detect and collect nearby loot bags (priority above patrol, below combat)
- [ ] `TownInventory` resource, inventory UI tab
- [ ] `Equipment` component: weapon + armor slots, feeds into `resolve_combat_stats()`
- [ ] Equipped items reflected in NPC equipment sprite layers

**Stage 25: Tech Trees** (see [specs/tech-tree.md](specs/tech-tree.md))

*Done when: player spends Food or Gold to buy tech-tree upgrades with prerequisites (no research building), and branch progression visibly unlocks stronger nodes (e.g., ArcherHome Lv2 unlock path, Military damage tier path).*

Chunks 1-2 complete (see [completed.md](completed.md)).

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

**Stage 26: Economy Depth**

*Done when: player must choose between feeding NPCs and buying upgrades - food is a constraint, not a score.*

- [ ] HP regen tiers (1x idle, 3x sleeping, 10x fountain)
- [ ] FoodEfficiency upgrade wired into `decision_system` eat logic
- [ ] Economy pressure: upgrades cost more food, NPCs consume more as population grows

**Stage 27: Diplomacy**

*Done when: a raider town sends a messenger offering a truce for 3 food/hour tribute - accepting stops raids, refusing triggers an immediate attack wave.*

- [ ] Town reputation system (hostile -> neutral -> friendly, based on food tribute and combat history)
- [ ] Tribute offers: raider towns propose truces at reputation thresholds
- [ ] Trade routes between player towns (send food caravan from surplus town to deficit town)
- [ ] Allied raider towns stop raiding, may send fighters during large attacks
- [ ] Betrayal: allied raider towns can turn hostile if tribute stops or player is weak

**Stage 28: Resources & Jobs**

*Done when: player builds a lumber mill near Forest tiles, assigns a woodcutter, collects wood, and builds a stone wall using wood + stone instead of food - multi-resource economy with job specialization.*

- [ ] Resource types: wood (Forest biome), stone (Rock biome), iron (ore nodes, rare)
- [ ] Harvester buildings: lumber mill, quarry (same spawner pattern as FarmerHome/ArcherHome, 1 worker each)
- [ ] Resource storage per town (like FoodStorage but for each type - gold already done via GoldStorage)
- [ ] Building costs use mixed resources (walls=stone, archer homes=wood+stone, upgrades=food+iron, etc.)
- [ ] Crafting: blacksmith building consumes iron -> produces weapons/armor (feeds into Stage 24 loot system)
- [ ] Villager job assignment UI (drag workers between roles - farming, woodcutting, mining, smithing, military)

**Stage 29: Armies & Marching**

*Done when: player recruits 15 archers into an army, gives a march order to a neighboring raider town, and the army walks across the map as a formation - arriving ready to fight.*

- [ ] Army formation from existing squads (select squad -> "Form Army" -> army entity with member list)
- [ ] March orders: right-click map location -> army walks as group (use existing movement system, group speed = slowest member)
- [ ] Unit types via tech tree unlocks: levy (cheap, weak), archer (ranged), men-at-arms (tanky, expensive)
- [ ] Army supply: marching armies consume food from origin town's storage, starve without supply
- [ ] Field battles: two armies in proximity -> combat triggers (existing combat system handles it)

**Stage 30: Conquest**

*Done when: player marches an army to a raider town, defeats defenders, and claims the town - raider town converts to player-owned town with buildings intact, player now manages two towns.*

- [ ] Town siege: army arrives at hostile settlement -> attacks defenders + buildings
- [ ] Building HP: walls have HP - attackers must breach defenses (archer homes/farmer homes HP already done)
- [ ] Town capture: all defenders dead + town center HP -> 0 = captured -> converts to player town
- [ ] AI expansion: AI players can attack each other and the player (not just raid - full conquest attempts)
- [ ] Victory condition: control all settlements on the map

**Stage 31: World Map**

*Done when: player conquers all towns on "County of Palm Beach", clicks "Next Region" on the world map, and starts a new county with harder AI and more raider towns - campaign progression.*

- [ ] World map screen: grid of regions (counties), each is a separate game map
- [ ] Region difficulty scaling (more raider towns, tougher AI, scarcer resources)
- [ ] Persistent bonuses between regions (tech carries over, starting resources from tribute)
- [ ] "Country" = set of regions. "World" = set of countries. Campaign arc.

**Stage 32: Tower Defense (Wintermaul Wars-inspired)**

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

### Bugs
- [ ] No active roadmap bugs listed right now.

### DRY & Single Source of Truth
- [x] Centralize world lifecycle startup/load flows to shared helpers (`world::materialize_generated_world`, `save::restore_world_from_save`) so game startup, menu load, in-game load, and AI world-setup tests cannot drift
- [ ] Replace hardcoded town indices in HUD with faction/town lookup helpers
- [ ] Add regression tests that enforce no behavior drift between player and AI build flows, startup and respawn flows, and both destroy entry points

### UI & UX
- [x] Factions tab shows current policy snapshot for the selected faction (read-only intel view)
- [x] Selected-NPC target overlay line now renders in test scenes (`AppState::Running`) as well as normal gameplay
- [ ] Persist left panel UI state (active tab + expanded/collapsed sections) in `UserSettings`
- [ ] Add `show_active_radius` debug toggle in Bevy UI
- [ ] Upgrade tab town snapshot: show `farmers/archers/farms/next spawn` summary
- [ ] Combat log window sizing: allow resize + persist width/height in `UserSettings`
- [ ] HP bar display mode toggle (Off / When Damaged / Always)
- [ ] Combat log scope/timestamp modes (Off/Own/All + Off/Time/Day+Time)
- [ ] Double-click locked slot to unlock (alternative to context action)
- [ ] Terrain tile click inspector (biome/tile coordinates)

## Specs

Implementation guides for upcoming stages. After delivery, spec content rolls into regular docs and the standalone spec file is retired.

| Spec | Stage | File |
|---|---|---|
| Chunked Tilemap | 16 | [specs/chunked-tilemap.md](specs/chunked-tilemap.md) |
| Tech Tree (Chunks 3-4) | 25 | [specs/tech-tree.md](specs/tech-tree.md) |
| NPC Skills & Proficiency | 21 | [specs/npc-skills.md](specs/npc-skills.md) |

## Performance

| Milestone | NPCs | Buildings | FPS | Status |
|-----------|------|-----------|-----|--------|
| CPU Bevy | 5,000 | — | 60+ | [x] |
| GPU physics | 10,000+ | — | 140 | [x] |
| Full behaviors | 10,000+ | — | 140 | [x] |
| Combat + projectiles | 10,000+ | — | 140 | [x] |
| GPU spatial grid | 10,000+ | — | 140 | [x] |
| Full game integration | 10,000 | — | 130 | [x] |
| Max scale tested | 50,000 | — | TBD | [x] buffers sized |
| HashMap elimination (items 3-6) | 30,000 | 30,000 | 60+ | [ ] next |
| Decision budgeting (items 1-2, 7) | 30,000 | 30,000 | 60+ | [ ] planned |
| Future (chunked tilemap) | 50,000+ | 50,000+ | 60+ | Planned |

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) - GPU spatial grid approach
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash) - marker component pattern
- [Bevy Render Graph](https://docs.rs/bevy/latest/bevy/render/render_graph/) - compute + render pipeline
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) - sprite batching, per-layer draw queues
- [Factorio FFF #421](https://www.factorio.com/blog/post/fff-421) - entity update optimization, lazy activation




