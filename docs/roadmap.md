# Roadmap

Target: 50,000 NPCs + 50,000 buildings @ 60fps with pure Bevy ECS + WGSL compute + GPU instanced rendering.

## How to Maintain This Roadmap

1. **Stages are the priority.** Read top-down. First unchecked stage is the current sprint.
2. **No duplication.** Each work item lives in exactly one place. Stages have future work. [completed.md](completed.md) has done work. [specs/](specs/) has implementation detail.
3. **Completed checkboxes are accomplishments.** Never delete them. When a stage is done, move its `[x]` items to [completed.md](completed.md).
4. **"Done when" sentences don't change** unless the game design changes.
5. **New features** go in the appropriate future stage.
6. **Describe current state, not history.** Use present-tense behavior in docs; put historical wording and change chronology in [completed.md](completed.md) or `CHANGELOG.md`.

## Completed

See [completed.md](completed.md) for completed work moved out of active stages.

## Stages

Stages 1-15, 18, 19: [x] Complete (see [completed.md](completed.md))

**Current Sprint (priority order):**
1. Loot cycle stress test — benchmark TownEquipment growth under 50K NPCs over extended play, cap or prune unbounded accumulation
2. Path recalculation on building place/remove (Stage 20) — dirty affected HPA* chunks, rebuild entrance nodes. Unblocks Stage 21 gates
3. Entity sleeping (Stage 16 item 1) — camera-radius culling, 5-15ms/frame savings

**Stage 16: Performance**

*Done when: 50K NPCs + 50K buildings at 60fps.*

GPU extract, GPU-native rendering, linear scan elimination, worksite indexing, slot-indexed occupancy, query-first migration, NpcLogCache filtering, decision sub-profiling, visual upload optimization, GPU targets dirty tracking, damage debug gating, readback throttling, event-driven visual upload, decision-frame budgeting, and candidate-driven healing complete (see [completed.md](completed.md)).

ECS source-of-truth migration complete (see [completed.md](completed.md)). ECS owns all NPC gameplay state. EntityMap is index-only (slot↔Entity, grid, kind/town/spatial). No dual-writes. Hot loops use query-first + indexed lookup. GPU is movement authority; ECS Position is read-model synced in `gpu_position_readback`.

Remaining performance items (sorted by expected savings):

1. [ ] [High] Entity sleeping (Factorio-style): NPCs outside camera radius skip behavior/movement ticks. At 50k NPCs, typically 80%+ are off-camera.
   Expected saving: ~5-15+ ms/frame CPU when most NPCs off-camera; near-zero if camera covers all.
2. [ ] [Medium] Cache-friendly vectors for hot building iteration paths (keep HashMaps as authority, vectors for tight loops).
   Expected saving: ~1-3 ms/frame CPU on building-heavy ticks.
3. [x] ~~[Medium] Pre-allocate `GpuReadState` vecs: readback observers create new Vecs per frame. At 50k entities, positions = 1.6MB allocation per frame.~~
   ~~Expected saving: ~0.5-1.5 ms/frame CPU plus allocator churn.~~
4. [x] ~~[Medium] `sync_building_hp_render` gated behind `BuildingHealState.needs_healing` — skips full building query when no buildings are damaged (99%+ of frames).~~
5. [x] ~~[Medium] `on_duty_tick_system` full iteration: narrow to OnDuty archers only.~~
   ~~Expected saving: ~0.3-1.0 ms/frame CPU.~~ Added `With<PatrolRoute>` query filter — iterates ~200 patrol units instead of 50K NPCs.
6. [x] ~~[Medium] Perf anti-pattern remediation pass: repeated query scans, `Vec::contains` → `HashSet`, per-item linear dedup.~~
   ~~Expected saving: ~1-4 ms/frame total.~~ Audit found most patterns already remediated. Fixed: flash_dirty temp Vec, pathfinding dirty_chunks Vec→HashSet + dead code removal.
7. [ ] [Low] `decision_system` remaining log pressure (~10 `format!` calls).
8. [ ] [Low] `sync_terrain_tilemap` chunk granularity: rewrites all chunks on any terrain change.
9. [ ] [Low] SystemTimings Mutex contention: replace with AtomicU32 + f32::to_bits.
10. [x] ~~`NpcsByTownCache` removed — `EntityMap.npc_by_town` is the single source of truth via `slots_for_town()`.~~
11. [ ] [Low] Perf guardrails: microbenchmarks + CI thresholds.
12. [ ] [Low] Message signal regression tests.

SystemParam bundle consolidation (code quality, not runtime perf):
- [ ] [Low] Create `GameLog` bundle: `{ combat_log: MessageWriter<CombatLogMsg>, game_time: Res<GameTime>, timings: Res<SystemTimings> }` and migrate systems still carrying this triple directly.
- [ ] [Low] Move/replace remaining ad-hoc bundles in `systems/behavior.rs` (keep only bundles with genuine local-only value; shared bundles live in `resources.rs`).
- [ ] [Low] Keep bundles flat (no nested `SystemParam` bundles inside other bundles) unless required to break Bevy param-count limits.

**Stage 17: Combat Depth**

*Done when: two archers with different traits fight the same raider noticeably differently - one flees early, the other berserks at low HP.*

Trait combinations, squad ignore-patrol, target oscillation fix, 7-axis spectrum personality, and behavior weight integration complete (see [completed.md](completed.md)).

Remaining:
- [ ] Target switching (prefer non-fleeing enemies, prioritize low-HP targets)
- [ ] Terrain combat modifiers — biome at target's position affects incoming damage:
  - Forest cover: 25% miss chance on projectile hits (roll in `process_proj_hits` or `damage_system` using target position → `WorldGrid` cell → `Biome::Forest`)
  - Rock high ground: +20% attack range for NPCs standing on Rock tiles (apply as runtime multiplier in GPU targeting `combat_range` check, or adjust `CachedStats.range` dynamically)
  - Grass/Dirt/Water: no combat modifier
  - Implementation: target position already known from `EntityMap`; convert to grid coords, read `WorldCell.terrain` — no new components needed

**Stage 18: Loot & Equipment**

*Done when: raider dies → loot auto-acquired to killer's carry → NPC keeps fighting → threshold triggers return home → deposit to town inventory → player equips item on NPC → stats increase and sprite changes.*

Design: no loot bags on the ground. Kill → loot goes directly into killer's `CarriedLoot` component → NPC keeps fighting → carry threshold triggers return home → deposit food/gold to storage + equipment to `TownEquipment` → player equips via UI → stat bonus + sprite change.

All 6 chunks complete (see [completed.md](completed.md)): unified CarriedLoot, LootItem/Rarity/EquipmentSlot types, equipment drops + carry accumulation, NpcEquipment (9 D2 slots) + stat integration, Armory UI tab (I key), Merchant building (buy/sell/reroll), save/load persistence + loot-cycle test. Additional: auto-equip system (hourly, distributes items to best NPC), immediate Armory auto-equip actions (selected NPC or whole town via the same auto-equip rules), equipment drops on death (50% per item to killer), inventory/armory UI overhaul (Equipped/Unequipped/All views, slot filters, sorting, bulk sell common, comparison tooltips, multi-town support), and Inspector++ NPC tabs (Overview/Loadout/Economy/Log) with per-NPC personal log and carried-loot detail.

Remaining:
- [ ] Loot cycle stress test: benchmark `TownEquipment` growth at 50K NPCs over extended play (2+ hours simulated). If unbounded, add inventory cap or periodic pruning of lowest-rarity items.

**Stage 19: Code Health** — [x] Complete (see [completed.md](completed.md))

**Stage 20: Pathfinding**

*Done when: NPCs navigate around obstacles using A\* or flow fields instead of pure boids steering. Raiders path around walls to find openings. Placing a building that would fully block access is rejected.*

- [x] A* pathfinding on the world grid (pathfinding.rs, movement.rs)
- [x] Terrain movement costs — Grass/Dirt=100, Forest=143, Rock=500, Water=800 (high but passable so NPCs can escape if pushed by physics). Road speed multiplier applied separately in GPU shader.
- [x] NPC pathfinding integration: all NPCs use A* paths for long-distance navigation with LOS bypass for short distances
- [x] Route spreading: successive A* calls inflate costs along found paths (PATH_SPREAD_COST=100, PATH_SPREAD_RADIUS=1) to spread NPC routes apart
- [x] Intermediate waypoint relaxed threshold (96px vs 40px for final destination) prevents pile-up from boid separation
- [x] Arrival detection parity for LOS/direct targets and waypoint paths: `gpu_position_readback` now marks `at_destination` for transit activities even when movement has no waypoints (direct `SetTarget`), with regression coverage for transit/no-path and non-transit/no-path cases
- [ ] Path recalculation on building place/remove (incremental update, not full rebuild)
- [ ] Path validation: reject building placements that fully block access to critical locations

Prerequisite for Stage 21 (wall gates) and Stage 25 (tower defense maze).

**Stage 21: Walls & Defenses**

*Done when: player builds a stone wall perimeter with a gate, raiders path around it or attack through it, chokepoints make guard placement strategic.*

Core wall system complete (see [completed.md](completed.md)).

Wall auto-tiling complete (see [completed.md](completed.md)).

Remaining:
- [ ] Gate building (walls with a passthrough that friendlies use, raiders must breach)
- [ ] Pathfinding integration: raiders route around walls to find openings, attack walls when no path exists (uses Stage 20 pathfinding)
- [ ] Guard towers (upgrade from guard post - elevated, +range, requires wall adjacency)

**Stage 22: Economy Depth**

*Done when: player must choose between feeding NPCs and buying upgrades - food is a constraint, not a score.*

- [ ] HP regen tiers (1x idle, 3x sleeping, 10x fountain)
- [ ] FoodEfficiency upgrade wired into `decision_system` eat logic
- [ ] Economy pressure: upgrades cost more food, NPCs consume more as population grows

**Stage 23: NPC Skills & Proficiency** (see [specs/npc-skills.md](specs/npc-skills.md))

*Done when: two NPCs with the same job but different proficiencies produce measurably different outcomes (farm output, combat effectiveness, dodge/survival), and those differences are visible in UI.*

- [ ] Add per-NPC skill set with proficiency values (0-100) keyed by role/action
- [ ] Skill growth from doing the work (farming raises farming, combat raises combat, dodging raises dodge)
- [ ] Proficiency modifies effectiveness:
- [ ] Farming proficiency affects farm growth/harvest efficiency
- [ ] Combat proficiency affects attack efficiency (accuracy/damage/cooldown contribution)
- [ ] Dodge proficiency affects projectile avoidance / survival in combat
- [ ] Render skill/proficiency details in inspector + roster sorting/filtering support
- [ ] Keep base-role identity intact (job still determines behavior class; proficiency scales effectiveness)

**Stage 24: Save Slots**

*Done when: player builds up a town for 20 minutes, quits, relaunches, and continues exactly where they left off - NPCs in the same positions, same HP, same upgrades, same food.*

Core save/load shipped (see [completed.md](completed.md)).
- [ ] Save slot selection (3 slots)

**Stage 25: Tower Defense (Wintermaul Wars-inspired)**

*Done when: player builds towers in a maze layout to shape enemy pathing, towers have elemental types with rock-paper-scissors counters, income accrues with interest, and towers upgrade/evolve into advanced forms.*

Chunk 1 — Maze & Pathing (depends on Stage 20 Pathfinding):
- [ ] Open-field tower placement on a grid (towers block pathing, enemies path around them)
- [ ] Maze validation — path from spawn to goal must always exist (reject placements that fully block)
- [ ] Visual path preview (show calculated enemy route through current maze)

Chunk 2 — Tower Upgrades & Evolution:
- [ ] Multi-tier upgrade path (Lv1 -> Lv2 -> Lv3, increasing stats + visual change)
- [ ] At max tier, evolve into specialized variants (e.g. Fire Lv3 -> Inferno AoE or Sniper Flame)
- [ ] Evolved towers get unique abilities (slow, DoT, chain lightning, lifesteal)

Chunk 3 — Elements & Waves:
- [ ] `Element` enum: Fire, Ice, Nature, Lightning, Arcane, Dark (6 elements)
- [ ] Element weakness matrix (Fire->Nature->Lightning->Ice->Fire, Arcane<->Dark)
- [ ] Creep waves carry an element - weak-element towers deal 2x, strong-element towers deal 0.5x
- [ ] Tower/creep element shown via tint or icon overlay

Chunk 4 — Economy & Sending:
- [ ] Per-wave gold income (base + bonus for no leaks)
- [ ] Interest on banked gold each wave (5% per round, capped)
- [ ] Leak penalty - lives lost per creep that reaches the goal
- [ ] Spend gold to send extra creeps into opponent's lane
- [ ] Send menu with creep tiers (cheap/fast, tanky, elemental, boss)
- [ ] Income bonus from sending (reward aggressive play)

**Stage 26: Resources & Jobs**

*Done when: player builds a lumber mill near Forest tiles, assigns a woodcutter, collects wood, and builds a stone wall using wood + stone instead of food - multi-resource economy with job specialization.*

- [ ] Resource types: wood (Forest biome), stone (Rock biome), iron (ore nodes, rare)
- [ ] Harvester buildings: lumber mill, quarry (same spawner pattern as FarmerHome/ArcherHome, 1 worker each)
- [ ] Resource storage per town (new resource types as ECS components on town entities, same pattern as FoodStore/GoldStore)
- [ ] Building costs use mixed resources (walls=stone, archer homes=wood+stone, upgrades=food+iron, etc.)
- [ ] Crafting: blacksmith building consumes iron -> produces weapons/armor (feeds into Stage 18 loot system)
- [ ] Villager job assignment UI (drag workers between roles - farming, woodcutting, mining, smithing, military)

**Stage 27: Armies & Marching**

*Done when: player recruits 15 archers into an army, gives a march order to a neighboring raider town, and the army walks across the map as a formation - arriving ready to fight.*

- [ ] Army formation from existing squads (select squad -> "Form Army" -> army entity with member list)
- [ ] March orders: right-click map location -> army walks as group (use existing movement system, group speed = slowest member)
- [ ] Unit types via tech tree unlocks: levy (cheap, weak), archer (ranged), men-at-arms (tanky, expensive)
- [ ] Army supply: marching armies consume food from origin town's storage, starve without supply
- [ ] Field battles: two armies in proximity -> combat triggers (existing combat system handles it)

**Stage 28: Conquest**

*Done when: player marches an army to a raider town, defeats defenders, and claims the town - raider town converts to player-owned town with buildings intact, player now manages two towns.*

Initial game setup: 1 player town, 1 AI builder town, 1 AI raider town on a small starting map. Conquest of these towns triggers the first expansion (Stage 30).

- [ ] Town siege: army arrives at hostile settlement -> attacks defenders + buildings
- [ ] Building HP: walls have HP - attackers must breach defenses (archer homes/farmer homes HP already done)
- [ ] Town capture: all defenders dead + town center HP -> 0 = captured -> converts to player town
- [ ] AI expansion: AI players can attack each other and the player (not just raid - full conquest attempts)

**Stage 29: Diplomacy**

*Done when: a raider town sends a messenger offering a truce for 3 food/hour tribute - accepting stops raids, refusing triggers an immediate attack wave.*

- [ ] Town reputation system (hostile -> neutral -> friendly, based on food tribute and combat history)
- [ ] Tribute offers: raider towns propose truces at reputation thresholds
- [ ] Trade routes between player towns (send food caravan from surplus town to deficit town)
- [ ] Allied raider towns stop raiding, may send fighters during large attacks
- [ ] Betrayal: allied raider towns can turn hostile if tribute stops or player is weak

**Stage 30: Endless Expansion**

*Done when: player conquers both starter AI towns, picks an expansion direction, map grows with new AI towns, and the cycle repeats — the game ends only when hardware can't keep up.*

The game starts small (3 towns) and grows outward each time the player conquers all hostile towns in the current map. Each expansion adds a new map chunk with fresh AI towns at increasing difficulty. There is no victory screen — the simulation runs until CPU/GPU hits its limit. Every player's "ending" is unique to their hardware.

- [ ] Expansion trigger: detect when all hostile towns on current map are conquered
- [ ] Direction picker UI: player chooses which direction to expand (N/S/E/W or quadrant)
- [ ] Map chunk generation: extend world grid in chosen direction, generate terrain + new AI towns
- [ ] Progressive difficulty: each expansion wave adds more towns, tougher AI, higher NPC counts
- [ ] Performance-aware scaling: monitor framerate, warn player when approaching hardware limits
- [ ] No end condition: cycle repeats indefinitely (expand -> conquer -> expand)

**Stage 31: Underground Caverns**

*Done when: player sends a party of NPCs into a cavern entrance, they descend into a procedurally generated underground layer, fight cave creatures, and return with rare loot.*

Cavern entrances spawn on the surface map (naturally on Rock biome, or revealed by expansion). Each entrance leads to a procedural underground layer — a separate grid with tunnels, chambers, and creature dens. NPCs explore autonomously: navigate tunnels, fight creatures, collect loot, and return home when injured or loaded up. Deeper caverns = tougher creatures + rarer loot.

- [ ] Cavern entrance building/object: placed on Rock tiles or spawned during map generation
- [ ] Underground layer generation: procedural tunnel/chamber layout (noise-based or cellular automata)
- [ ] Creature types: cave dwellers with unique combat behaviors (melee swarmers, ranged spitters, boss creatures in deep chambers)
- [ ] NPC delving behavior: send party -> descend -> explore -> fight -> loot -> return when hurt or full
- [ ] Depth tiers: each cavern has multiple depth levels, deeper = harder + better loot
- [ ] Cavern loot table: rare ores, unique equipment, crafting materials not found on surface (feeds into Stage 18 loot + Stage 26 resources)
- [ ] Fog of war: underground areas revealed as NPCs explore, persists between visits
- [ ] Creature respawn: caverns repopulate over time, making them replayable

**Stage 32: CRD Architecture (Code Quality)**

*Done when: every entity type (NPC, Building, Town, Activity, Item) follows the Def→Instance→Controller pattern — static registry defines the type, ECS components hold all runtime state, systems reconcile.*

Current CRD compliance:

| Entity     | Def Registry                    | Instance Pattern                          | Score |
|------------|--------------------------------|------------------------------------------|-------|
| NPCs       | NpcDef + NPC_REGISTRY          | ECS components (NpcStats)                | 95%   |
| Buildings  | BuildingDef + BUILDING_REGISTRY| slim spatial index + ECS components      | 90%   |
| Activities | ActivityDef + ACTIVITY_REGISTRY| Activity component + fieldless kind      | 90%   |
| Towns      | TownDef + TOWN_REGISTRY        | ECS town entities (TownAccess)           | 60%   |
| Items      | None (procedural gen)          | LootItem + NpcEquipment                  | 60%   |

Can be done incrementally alongside other stages. Each chunk is independent.

Chunk 1 — NPC Instance Cleanup (80% → 95%):
- [x] Move NpcMeta (name, level, XP) from NpcMetaCache parallel array onto ECS entities as NpcStats component
- [ ] Simplify `materialize_npc()` — read NpcDef internally instead of 9+ loose params
- [ ] Remove CombatConfig/JobStats duplication (reference NpcDef directly)

Chunk 2 — Building Instance Consolidation (70% → 90%):
- [x] Replace BuildingInstance god struct with ECS components: ProductionState, TowerBuildingState, SpawnerState, ConstructionProgress, WaypointOrder, WallLevel, MinerHomeConfig
- [x] Slim BuildingInstance to 6-field spatial index (kind, position, town_idx, slot, faction, occupants)
- [ ] Simplify `place_building()` signature — read BuildingDef internally

Chunk 3 — TownDef Registry (40% → 80%):
- [x] Add TownDef struct + TOWN_REGISTRY (player, ai_builder, ai_raider templates)
- [ ] Data-driven town generation — template defines building layout, NPC roster, faction kind
- [x] Consolidate Town + TownUpgrades + PolicySet under ECS town entities (TownAccess SystemParam, FoodStore/GoldStore/TownPolicy/TownUpgradeLevel/TownEquipment components)

Chunk 4 — ActivityDef Registry (50% → 90%):
- [x] Add ActivityDef struct + ACTIVITY_REGISTRY static table (label, distraction, sleep_visual, is_restful, is_working per kind)
- [x] Make ActivityKind fieldless (Copy+Eq+Hash), move per-instance data to Activity struct fields (target_pos, worksite, recover_until)
- [x] Replace inline match arms in ActivityKind methods with registry lookups (def().distraction, def().label, def().is_working, def().is_restful)
- [x] Adding a new activity = 1 enum variant + 1 registry entry

Chunk 5 — ItemDef Registry (60% → 85%):
- [ ] Add ItemDef struct for item templates (base stats, sprite options, name patterns per slot+rarity)
- [ ] Procedural generation references ItemDef templates with random variation
- [ ] Unifies sprite tables + name generation + stat ranges into one registry

Sound (bevy_audio) woven into stages. Done: arrow shoot SFX, NPC death SFX (24 variants), spatial camera culling, per-kind dedup. Remaining: building place, wall hit, loot pickup (Stages 17-21); element sounds + wave horn (Stage 25).

## Backlog

### DRY & Single Source of Truth
- [ ] Replace hardcoded town indices in HUD with faction/town lookup helpers
- [ ] Add regression tests that enforce no behavior drift between player and AI build flows, startup and respawn flows, and both destroy entry points

### Testing
- [x] Unit test infrastructure: `#[cfg(test)]` modules in stats.rs, constants.rs, components.rs (65 pure function tests via `cargo test`)
- [x] System-level tests: headless `App::new()` + `FixedUpdate` tests for energy, regen, starvation, game_time, cooldown, damage, construction systems + population helpers (52 tests)
- [x] Pure function tests: generate_name, generate_personality in spawn.rs (6 tests)

### UI & UX
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
| NPC Skills & Proficiency | 23 | [specs/npc-skills.md](specs/npc-skills.md) |

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
| Worksite indexing + occupancy | 30,000 | 30,000 | 60+ | [x] done |
| Query-first + log gating + sub-profiling | 30,000 | 30,000 | 60+ | [x] done |
| Visual upload + targets dirty tracking | 30,000 | 30,000 | 60+ | [x] done |
| GPU iter + decision budgeting (items 1-2) | 50,000 | 50,000 | 60+ | [x] done |
| Entity sleeping + healing (items 3-4) | 50,000 | 50,000 | 60+ | healing done, sleeping TBD |
| Future (chunked tilemap) | 50,000+ | 50,000+ | 60+ | Planned |

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) - GPU spatial grid approach
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash) - marker component pattern
- [Bevy Render Graph](https://docs.rs/bevy/latest/bevy/render/render_graph/) - compute + render pipeline
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) - sprite batching, per-layer draw queues
- [Factorio FFF #421](https://www.factorio.com/blog/post/fff-421) - entity update optimization, lazy activation
