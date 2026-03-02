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

Stages 1-15, 17, 23: [x] Complete (see [completed.md](completed.md))

**Stage 16: Performance**

*Done when: 50K NPCs + 50K buildings at 60fps.*

GPU extract, GPU-native rendering, linear scan elimination, worksite indexing, slot-indexed occupancy, query-first migration, NpcLogCache filtering, decision sub-profiling, visual upload optimization, GPU targets dirty tracking, damage debug gating, readback throttling, event-driven visual upload, decision-frame budgeting, and candidate-driven healing complete (see [completed.md](completed.md)).

ECS source-of-truth migration complete (see [completed.md](completed.md)). ECS owns all NPC gameplay state. EntityMap is index-only (slot↔Entity, grid, kind/town/spatial). No dual-writes. Hot loops use query-first + indexed lookup. GPU is movement authority; ECS Position is read-model synced in `gpu_position_readback`.

Remaining performance items (sorted by expected savings):

1. [ ] [High] Entity sleeping (Factorio-style): NPCs outside camera radius skip behavior/movement ticks. At 50k NPCs, typically 80%+ are off-camera.
   Expected saving: ~5-15+ ms/frame CPU when most NPCs off-camera; near-zero if camera covers all.
2. [ ] [Medium] Cache-friendly vectors for hot building iteration paths (keep HashMaps as authority, vectors for tight loops).
   Expected saving: ~1-3 ms/frame CPU on building-heavy ticks.
3. [ ] [Medium] Pre-allocate `GpuReadState` vecs: readback observers create new Vecs per frame. At 50k entities, positions = 1.6MB allocation per frame.
   Expected saving: ~0.5-1.5 ms/frame CPU plus allocator churn.
4. [ ] [Medium] `sync_building_hp_render` every-frame rebuild: gate behind dirty flag. Only damaged buildings (<1%) produce output.
   Expected saving: ~0.5-1.5 ms/frame CPU.
5. [ ] [Medium] `on_duty_tick_system` full iteration: narrow to OnDuty archers only.
   Expected saving: ~0.3-1.0 ms/frame CPU.
6. [ ] [Medium] Perf anti-pattern remediation pass: repeated query scans, `Vec::contains` → `HashSet`, per-item linear dedup.
   Expected saving: ~1-4 ms/frame total.
7. [ ] [Low] `decision_system` remaining log pressure (~10 `format!` calls).
8. [ ] [Low] `sync_terrain_tilemap` chunk granularity: rewrites all chunks on any terrain change.
9. [ ] [Low] SystemTimings Mutex contention: replace with AtomicU32 + f32::to_bits.
10. [ ] [Low] `NpcsByTownCache` `Vec::retain()` → `HashSet` for mass death spikes.
11. [ ] [Low] Perf guardrails: microbenchmarks + CI thresholds.
12. [ ] [Low] Message signal regression tests.

SystemParam bundle consolidation (code quality, not runtime perf):
- [ ] [Low] Create `GameLog` bundle: `{ combat_log: MessageWriter<CombatLogMsg>, game_time: Res<GameTime>, timings: Res<SystemTimings> }` and migrate systems still carrying this triple directly.
- [ ] [Low] Move/replace remaining ad-hoc bundles in `systems/behavior.rs` (keep only bundles with genuine local-only value; shared bundles live in `resources.rs`).
- [ ] [Low] Keep bundles flat (no nested `SystemParam` bundles inside other bundles) unless required to break Bevy param-count limits.

**Stage 17: Combat Depth**

*Done when: two archers with different traits fight the same raider noticeably differently - one flees early, the other berserks at low HP.*

Trait combinations, squad ignore-patrol, and target oscillation fix complete (see [completed.md](completed.md)).

Remaining:
- [ ] Unify `TraitKind` (4 variants) and `trait_name()` (9 names) into single 9-trait Personality system
- [ ] All 9 traits affect both `resolve_combat_stats()` and `decision_system` behavior weights
- [ ] Target switching (prefer non-fleeing enemies, prioritize low-HP targets)
- [ ] Terrain combat modifiers — biome at target's position affects incoming damage:
  - Forest cover: 25% miss chance on projectile hits (roll in `process_proj_hits` or `damage_system` using target position → `WorldGrid` cell → `Biome::Forest`)
  - Rock high ground: +20% attack range for NPCs standing on Rock tiles (apply as runtime multiplier in GPU targeting `combat_range` check, or adjust `CachedStats.range` dynamically)
  - Grass/Dirt/Water: no combat modifier
  - Implementation: target position already known from `EntityMap`; convert to grid coords, read `WorldCell.terrain` — no new components needed

**Stage 18: Loot & Equipment**

*Done when: raider dies → loot auto-acquired to killer's carry → NPC keeps fighting → threshold triggers return home → deposit to town inventory → player equips item on NPC → stats increase and sprite changes.*

Design: no loot bags on the ground. Kill → loot goes directly into killer's `CarriedLoot` component → NPC keeps fighting → carry threshold triggers return home → deposit food/gold to storage + equipment to `TownInventory` → player equips via UI → stat bonus + sprite change.

**Chunk 0: Unified CarriedLoot** (refactor — prerequisite for all other chunks)

Replace 3 fragmented carry mechanisms with 1. Currently: (1) `Activity::Returning { loot: Vec<(ItemKind, i32)> }` for food/gold from kills/theft, (2) `CarriedGold(i32)` for miners, (3) loot stacking in Activity::Returning while NPC keeps fighting. All replaced by single `CarriedLoot` component.

- [x] New `CarriedLoot` component (always-present on ALL NPCs, like NpcWorkState): `{ food: i32, gold: i32, equipment: Vec<LootItem> }` with `is_empty()`, `total_items()`, `visual_key() -> u8`
- [x] Remove `CarriedGold` component — absorbed into `CarriedLoot.gold`
- [x] Simplify `Activity::Returning` — remove the `loot` payload (just means "going home", loot lives in CarriedLoot)
- [x] Migrate all loot-write sites: `death_system` NPC-killer/building-killer loot → `CarriedLoot.food/.gold`, farm theft → `CarriedLoot.food`, miner gold harvest → `CarriedLoot.gold`, farm harvest delivery → `CarriedLoot.food`
- [x] Refactor `arrival_system`: read `CarriedLoot`, deposit food/gold/equipment to town storage, drain to zero after deposit. Equipment items → `TownInventory` (from Chunk 1)
- [x] Move loot-based `visual_key()` from `Activity` to `CarriedLoot` for GPU dirty tracking
- [x] Update NPC inspector to read from `CarriedLoot` instead of `Activity::Returning.loot` + `CarriedGold`
- [x] Save/load: `NpcSaveData` gets `carried_food: i32, carried_gold: i32` fields (replaces inline loot in activity serialization). `#[serde(default)]` for backward compat.
- [x] Run ALL existing tests after Chunk 0 — arrival, raider-cycle, miner-cycle, combat, vertical-slice must pass

**Chunk 1: LootItem data types + NPC registry + TownInventory**

- [x] `EquipmentSlot` enum: expanded to 9 D2 variants (Helm/Armor/Weapon/Shield/Gloves/Boots/Belt/Amulet/Ring)
- [x] `Rarity` enum: Common (60%), Uncommon (25%), Rare (12%), Epic (3%) — `gold_cost()` 25/75/200/500, `color()` white/green/blue/purple, `stat_range()` 5-10%/10-20%/20-35%/35-50%
- [x] `LootItem` struct: `id: u64`, `slot: EquipmentSlot`, `rarity: Rarity`, `stat_bonus: f32`, `sprite: (f32, f32)`, `name: String` (Clone, Debug, Serialize, Deserialize)
- [x] New NpcDef fields in NPC_REGISTRY: `equipment_drop_rate: f32` (raiders: 0.30, others: 0.0), `equip_slots: &'static [EquipmentSlot]`
- [x] `NextLootItemId` resource: u64 counter
- [x] `roll_loot_item()` helper: rarity-weighted random → LootItem with atlas sprites from curated list
- [x] `TownInventory` resource: `Vec<Vec<LootItem>>` indexed by town — `add(town_idx, item)`, `remove(town_idx, item_id) -> Option<LootItem>`

**Chunk 2: Equipment drop on kill + carry accumulation**

- [x] In `death_system` NPC-killer branch: after food/gold, check `npc_def(dead_job).equipment_drop_rate` → roll → `roll_loot_item()` → push to killer's `CarriedLoot.equipment`
- [x] Combat log `CombatEventKind::Loot` with rarity-labeled message (reuses existing Loot event kind)
- [x] When killed NPC carried equipment (`CarriedLoot.equipment` not empty): killer is NPC → 50% per item transfers to killer's carry (failed = destroyed); killer is tower/fountain → items deposit directly to tower's town `TownInventory`
- [x] Loot threshold in `decision_system` (Priority ~4.5): if `carried_loot.equipment.len() >= LOOT_CARRY_THRESHOLD` AND not in combat → `Activity::Returning`. Only for NPCs with `npc_def(job).equip_slots` non-empty.

**Chunk 3: NpcEquipment (D2 slots) + stat integration + DRY consolidation**

- [x] `EquipmentSlot` expanded to 9 D2 variants: Helm/Armor/Weapon/Shield (sprite-visible) + Gloves/Boots/Belt/Amulet/Ring (stat-only)
- [x] `NpcEquipment` component (10 slots, always-present): weapon/helm/armor/shield sprites with NpcDef fallback, total_weapon_bonus/total_armor_bonus/total_speed_bonus/total_stamina_bonus
- [x] Remove `EquippedWeapon`, `EquippedArmor`, `EquippedHelmet` — all replaced by NpcEquipment
- [x] GPU: 3 equipment queries → 1 NpcEquipment query, stride 24→28, LAYER_COUNT 7→8, shield layer added, shader slot*6u→slot*7u
- [x] `resolve_combat_stats()`: weapon_bonus/armor_bonus params, damage *= (1+weapon_bonus), max_health *= (1+armor_bonus). All 3 call sites updated with real equipment bonuses.
- [x] `materialize_npc()`: inserts NpcEquipment from overrides, removed conditional Equipped* inserts
- [x] Save/load: NpcEquipment in NpcSaveData + NpcSpawnOverrides, backward compat with legacy weapon/helmet/armor fields
- [x] Dead code cleanup: behavior.rs weapon_q/helmet_q/armor_q removed, game_hud shows all D2 slots
- [x] `EquipItemMsg`/`UnequipItemMsg` messages + `process_equip_system`: moves item between TownInventory ↔ NpcEquipment, re-resolves CachedStats + MarkVisualDirty
- [x] `re_resolve_npc_stats()` helper: shared stat re-resolution + GPU update (used by equip, unequip, upgrades)

**Chunk 4: Inventory UI tab**

- [x] `LeftPanelTab::Inventory` — keybind: `I` (Factions moved to `G`)
- [x] Inventory tab: scrollable list, rarity-colored, equip/unequip buttons, selected NPC equipment display
- [x] NPC inspector: rarity-colored equipment display with stat bonuses, carried equipment count

**Chunk 5: Merchant building**

- [x] `BuildingKind::Merchant` — Economy category, player_buildable, 50 gold, TownGrid placement
- [x] 1-per-town enforcement in `place_building()` + build menu
- [x] `MerchantInventory` resource: per-town `MerchantStock { items: Vec<LootItem>, refresh_timer: f32 }`
- [x] Merchant refresh system: initial stock on placement, refresh every 12 game-hours
- [x] Merchant inspector UI: stock with buy buttons, sell from inventory, reroll (50g)
- [x] Save/load: MerchantInventory persisted in save data

**Chunk 6: Save/load + test**

- [x] Save/load: CarriedLoot, NpcEquipment, TownInventory, MerchantInventory, NextLootItemId — all `#[serde(default)]`
- [ ] `loot-cycle` integration test: spawn → kill → verify carry → return → deposit → equip → verify stats

Key files: `components.rs` (CarriedLoot, NpcEquipment, simplified Activity::Returning), `constants.rs` (EquipmentSlot, Rarity, LootItem, NpcDef fields, roll_loot_item), `resources.rs` (TownInventory, MerchantInventory, NextLootItemId, LeftPanelTab::Inventory), `systems/health.rs` (equipment drops, carried loot transfer), `systems/behavior.rs` (carry migration, loot threshold), `systems/stats.rs` (equipment in resolve_combat_stats, equip messages), `systems/spawn.rs` (insert CarriedLoot + NpcEquipment), `systems/economy.rs` (merchant refresh, arrival deposit), `gpu.rs` (NpcEquipment query), `ui/left_panel.rs` (inventory tab), `ui/game_hud.rs` (merchant + equipment inspectors), `save.rs` (persistence), `world.rs` (BuildingKind::Merchant)

**Stage 19: Pathfinding**

*Done when: NPCs navigate around obstacles using A\* or flow fields instead of pure boids steering. Raiders path around walls to find openings. Placing a building that would fully block access is rejected.*

- [ ] A* or flow field pathfinding on the world grid
- [ ] Terrain movement costs — biome affects GPU movement speed via existing `tile_flags` pattern:
  - Cost table: Grass/Dirt=1.0x (base), Road=1.5x (already done), Forest=0.7x, Rock=0.5x, Water=impassable
  - GPU shader (`npc_compute.wgsl`): after the existing `TILE_ROAD` speed check (~line 169), add `TILE_FOREST`/`TILE_ROCK` branches that multiply `speed` by 0.7/0.5. `TILE_WATER` blocks entry (same wall-collision pattern as `TILE_WALL`)
  - Tile flag constants already exist in `constants.rs`: `TILE_GRASS=1, TILE_FOREST=2, TILE_WATER=4, TILE_ROCK=8, TILE_DIRT=16`
  - `populate_tile_flags()` in `gpu.rs` already writes biome → flag bits per cell — no Rust-side changes needed for the flag data
  - A* cost function should use the same multipliers so pathfinding agrees with GPU movement
- [ ] NPC pathfinding integration: raiders route around walls, all NPCs use paths for long-distance navigation
- [ ] Path recalculation on building place/remove (incremental update, not full rebuild)
- [ ] Path validation: reject building placements that fully block access to critical locations

Prerequisite for Stage 20 (wall gates) and Stage 24 (tower defense maze).

**Stage 20: Walls & Defenses**

*Done when: player builds a stone wall perimeter with a gate, raiders path around it or attack through it, chokepoints make guard placement strategic.*

Core wall system complete (see [completed.md](completed.md)).

Wall auto-tiling complete (see [completed.md](completed.md)).

Remaining:
- [ ] Gate building (walls with a passthrough that friendlies use, raiders must breach)
- [ ] Pathfinding integration: raiders route around walls to find openings, attack walls when no path exists (uses Stage 19 pathfinding)
- [ ] Guard towers (upgrade from guard post - elevated, +range, requires wall adjacency)

**Stage 21: Economy Depth**

*Done when: player must choose between feeding NPCs and buying upgrades - food is a constraint, not a score.*

- [ ] HP regen tiers (1x idle, 3x sleeping, 10x fountain)
- [ ] FoodEfficiency upgrade wired into `decision_system` eat logic
- [ ] Economy pressure: upgrades cost more food, NPCs consume more as population grows

**Stage 22: NPC Skills & Proficiency** (see [specs/npc-skills.md](specs/npc-skills.md))

*Done when: two NPCs with the same job but different proficiencies produce measurably different outcomes (farm output, combat effectiveness, dodge/survival), and those differences are visible in UI.*

- [ ] Add per-NPC skill set with proficiency values (0-100) keyed by role/action
- [ ] Skill growth from doing the work (farming raises farming, combat raises combat, dodging raises dodge)
- [ ] Proficiency modifies effectiveness:
- [ ] Farming proficiency affects farm growth/harvest efficiency
- [ ] Combat proficiency affects attack efficiency (accuracy/damage/cooldown contribution)
- [ ] Dodge proficiency affects projectile avoidance / survival in combat
- [ ] Render skill/proficiency details in inspector + roster sorting/filtering support
- [ ] Keep base-role identity intact (job still determines behavior class; proficiency scales effectiveness)

**Stage 23: Save Slots**

*Done when: player builds up a town for 20 minutes, quits, relaunches, and continues exactly where they left off - NPCs in the same positions, same HP, same upgrades, same food.*

Core save/load shipped (see [completed.md](completed.md)).
- [ ] Save slot selection (3 slots)

**Stage 24: Tower Defense (Wintermaul Wars-inspired)**

*Done when: player builds towers in a maze layout to shape enemy pathing, towers have elemental types with rock-paper-scissors counters, income accrues with interest, and towers upgrade/evolve into advanced forms.*

Chunk 1 — Maze & Pathing (depends on Stage 19 Pathfinding):
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

**Stage 25: Resources & Jobs**

*Done when: player builds a lumber mill near Forest tiles, assigns a woodcutter, collects wood, and builds a stone wall using wood + stone instead of food - multi-resource economy with job specialization.*

- [ ] Resource types: wood (Forest biome), stone (Rock biome), iron (ore nodes, rare)
- [ ] Harvester buildings: lumber mill, quarry (same spawner pattern as FarmerHome/ArcherHome, 1 worker each)
- [ ] Resource storage per town (like FoodStorage but for each type - gold already done via GoldStorage)
- [ ] Building costs use mixed resources (walls=stone, archer homes=wood+stone, upgrades=food+iron, etc.)
- [ ] Crafting: blacksmith building consumes iron -> produces weapons/armor (feeds into Stage 18 loot system)
- [ ] Villager job assignment UI (drag workers between roles - farming, woodcutting, mining, smithing, military)

**Stage 26: Armies & Marching**

*Done when: player recruits 15 archers into an army, gives a march order to a neighboring raider town, and the army walks across the map as a formation - arriving ready to fight.*

- [ ] Army formation from existing squads (select squad -> "Form Army" -> army entity with member list)
- [ ] March orders: right-click map location -> army walks as group (use existing movement system, group speed = slowest member)
- [ ] Unit types via tech tree unlocks: levy (cheap, weak), archer (ranged), men-at-arms (tanky, expensive)
- [ ] Army supply: marching armies consume food from origin town's storage, starve without supply
- [ ] Field battles: two armies in proximity -> combat triggers (existing combat system handles it)

**Stage 27: Conquest**

*Done when: player marches an army to a raider town, defeats defenders, and claims the town - raider town converts to player-owned town with buildings intact, player now manages two towns.*

- [ ] Town siege: army arrives at hostile settlement -> attacks defenders + buildings
- [ ] Building HP: walls have HP - attackers must breach defenses (archer homes/farmer homes HP already done)
- [ ] Town capture: all defenders dead + town center HP -> 0 = captured -> converts to player town
- [ ] AI expansion: AI players can attack each other and the player (not just raid - full conquest attempts)
- [ ] Victory condition: control all settlements on the map

**Stage 28: Diplomacy**

*Done when: a raider town sends a messenger offering a truce for 3 food/hour tribute - accepting stops raids, refusing triggers an immediate attack wave.*

- [ ] Town reputation system (hostile -> neutral -> friendly, based on food tribute and combat history)
- [ ] Tribute offers: raider towns propose truces at reputation thresholds
- [ ] Trade routes between player towns (send food caravan from surplus town to deficit town)
- [ ] Allied raider towns stop raiding, may send fighters during large attacks
- [ ] Betrayal: allied raider towns can turn hostile if tribute stops or player is weak

**Stage 29: World Map**

*Done when: player conquers all towns on "County of Palm Beach", clicks "Next Region" on the world map, and starts a new county with harder AI and more raider towns - campaign progression.*

- [ ] World map screen: grid of regions (counties), each is a separate game map
- [ ] Region difficulty scaling (more raider towns, tougher AI, scarcer resources)
- [ ] Persistent bonuses between regions (tech carries over, starting resources from tribute)
- [ ] "Country" = set of regions. "World" = set of countries. Campaign arc.

Sound (bevy_audio) woven into stages. Done: arrow shoot SFX, NPC death SFX (24 variants), spatial camera culling, per-kind dedup. Remaining: building place, wall hit, loot pickup (Stages 17-20); element sounds + wave horn (Stage 24).

## Backlog

### DRY & Single Source of Truth
- [ ] Replace hardcoded town indices in HUD with faction/town lookup helpers
- [ ] Add regression tests that enforce no behavior drift between player and AI build flows, startup and respawn flows, and both destroy entry points

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
| NPC Skills & Proficiency | 22 | [specs/npc-skills.md](specs/npc-skills.md) |

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
| Entity sleeping + healing (items 3-4) | 50,000 | 50,000 | 60+ | healing done, sleeping planned |
| Future (chunked tilemap) | 50,000+ | 50,000+ | 60+ | Planned |

## References

- [Simon Green's CUDA Particles](https://developer.download.nvidia.com/assets/cuda/files/particles.pdf) - GPU spatial grid approach
- [FSM in ECS](https://www.richardlord.net/blog/ecs/finite-state-machines-with-ash) - marker component pattern
- [Bevy Render Graph](https://docs.rs/bevy/latest/bevy/render/render_graph/) - compute + render pipeline
- [Factorio FFF #251](https://www.factorio.com/blog/post/fff-251) - sprite batching, per-layer draw queues
- [Factorio FFF #421](https://www.factorio.com/blog/post/fff-421) - entity update optimization, lazy activation
