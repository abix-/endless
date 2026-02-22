# AI Player System

Autonomous opponents that build, upgrade, and fight like the player. Each AI settlement gets a personality that drives all decisions through weighted random scoring — same pattern as NPC behavior.

**Source**: `rust/src/systems/ai_player.rs` (decisions, building, squads), `rust/src/systems/economy.rs` (migration)

## AI Kinds

| Kind | Spawned By | Buildings | NPCs |
|------|-----------|-----------|------|
| **Builder** | World gen (AI towns) | Farms, farmer homes, archer homes, crossbow homes, fighter homes, miner homes, waypoints | Farmers, archers, crossbows, fighters, miners |
| **Raider** | Migration system (dynamic raider towns) | Tents | Raiders |

## Personalities

Assigned randomly at creation. Drives every decision the AI makes. All personality-specific values in one place:

### Building Economy

| | Aggressive | Balanced | Economic |
|-|-----------|----------|----------|
| **Build weights** (farm/house/archer/waypoint) | 10/10/30/20 | 20/20/15/10 | 30/25/5/5 |
| **Farmer home target** | 1:1 with farms | farms + 1 | farms + 2 |
| **Archer home target** | 1:1 with homes | homes / 2 | 1 + homes / 3 |
| **Food reserve per spawner** | 0 | 1 | 2 |
| **Slot placement: farms** | `farm_slot_score` (adjacency + 2×2 block + line bonus) | `balanced_farm_ray_score` (cardinal axis rays from center) | same as Aggressive |
| **Slot placement: homes** | `farmer_home_border_score` (must border farms) | `balanced_house_side_score` (beside axis rays, not on them) | same as Aggressive |
| **Road weight** | 2.0 | 3.0 | 8.0 |
| **Road batch size** | 2 | 3 | 6 |
| **Road pattern** | Cardinal axes from center | 3×3 grid (`rem_euclid(3)`) | 4×4 grid (`rem_euclid(4)`) |

### Mining

| | Aggressive | Balanced | Economic |
|-|-----------|----------|----------|
| **Miners per mine** | 1 | 2 | 4 |
| **Mining expand weight** | 12 | 8 | 5 |

### Military & Squads

| | Aggressive | Balanced | Economic |
|-|-----------|----------|----------|
| **Total squads** | 3 | 2 | 2 |
| **Defense share** | 25% (1 reserve squad) | 45% (1 reserve squad) | 65% (1 reserve squad) |
| **Attack squads** | 2 (55/45 split) | 1 (remainder) | 1 (remainder) |
| **Retarget cooldown** | 15s | 25s | 40s |
| **Preferred targets** | all buildings (farms, homes, archers, crossbows, waypoints, tents, miners) | military (archer homes + crossbow homes + waypoints) | farms only |

All personalities share the same fallback target set if preferred kinds yield nothing: farms, farmer homes, archer homes, crossbow homes, fighter homes, waypoints, tents, miner homes. Towns, gold mines, and beds are never targeted.

### Policies

| | Aggressive | Balanced | Economic |
|-|-----------|----------|----------|
| **Archer aggressive** | yes | no | no |
| **Archer leash** | no | no | yes |
| **Farmer fight back** | yes | no | no |
| **Prioritize healing** | no | no | yes |
| **Archer flee HP** | 0% | default | 25% |
| **Farmer flee HP** | 30% | default | 50% |

### Upgrades

Dynamic weight vector built from `UPGRADES` registry (category + stat_kind lookups via `set()` helper). Scored alongside buildings in the same weighted random pool. Each NPC category (Archer, Fighter, Crossbow, Farmer, Miner) has independent upgrade branches with separate levels.

**Builder upgrade emphasis:**
- Aggressive: Archer/Fighter attack/HP, crossbow upgrades, expansion, arrow upgrades
- Balanced: mixed Archer/Fighter + economy + crossbow, strong expansion
- Economic: farm yield, farmer/miner HP, gold yield, strongest expansion, light crossbow

**Raider upgrade emphasis:** Archer + Fighter HP, attack, attack speed, move speed. No economy or crossbow upgrades.

## Decision Loop

Runs every **5 seconds** (`DEFAULT_AI_INTERVAL`). Each tick, every active AI player does:

```
1. Skip if inactive (migration not settled yet)
2. Build/refresh town snapshot (cached; cleared when building_grid, mining, or patrol_perimeter dirty)
3. Count food and spawners
4. GATE: if food ≤ reserve → skip this tick entirely
5. Compute desire signals (food, military, gold, economy)
6. Build TownContext (center, food, has_slots, slot_fullness, MineAnalysis for Builders)
7. Count buildings via building_counts() → HashMap<BuildingKind, usize>, compute targets and deficits
8. Phase 1 — BUILDING: score eligible building actions, retry loop (weighted pick → execute → on failure remove variant and re-pick)
9. Phase 2 — UPGRADE: re-check food/gold after Phase 1 spend, score eligible upgrades, pick, execute
10. Invalidate snapshot on successful build/upgrade, log to combat log
```

Each tick can produce up to **two** actions: one building and one upgrade. Phase 2 re-reads food/gold after Phase 1 spending.

### Retry Loop (Phase 1)

When the picked building action fails to execute (e.g., no valid road candidates, waypoint cell blocked), the AI removes that action variant from the score pool using `std::mem::discriminant` and re-picks from remaining candidates. This prevents wasted ticks — previously a dominant action like Roads (score 48) could fail silently every tick for hours while lower-scored actions (Farm=11, MinerHome=9) never got a chance.

### Debug Logging

`UserSettings.debug_ai_decisions` toggle (Settings → Debug → "AI Decision Logging"). When enabled, failed actions are logged to `last_actions` as `[dbg] Roads FAILED (Roads=48.0 Farm=11.4 ...)` showing the action and top scores at time of failure. Appears in the faction inspector's Recent Actions list.

### Food Reserve Gate

```
reserve = food_reserve_per_spawner × spawner_count
```

Every spawner building (farmer home, archer home, crossbow home, fighter home, miner home) counts. The AI won't spend food if at or below reserve. This prevents self-starvation but also slows building as the town grows.

### Desire Signals

Four desire signals computed once per tick, used as multiplicative gates on building scores:

**Food desire** — drives farm and farmer home construction:
```
food_desire = clamp(1.0 - (food - reserve) / reserve, 0.0, 1.0)
```
- `0.0` → food is at 2× reserve or higher (comfortable, no farms/houses built)
- `1.0` → food is at reserve floor (maximum urgency)
- Aggressive (reserve=0) uses absolute fallback: food < 5 → 0.8, food < 10 → 0.4, else 0.0

**Military desire** — drives barracks, crossbow homes, and waypoint construction:
```
barracks_gap = (target - barracks) / target
waypoint_gap = (barracks - waypoints) / barracks
military_desire = clamp(barracks_gap × 0.75 + waypoint_gap × 0.25, 0.0, 1.0)
```

**Gold desire** — drives mining and gold-costing upgrades:
```
cheapest_gold = cost of cheapest affordable gold upgrade
gold_desire = clamp((1 - gold / cheapest_gold) × gold_desire_mult, 0..1)
```
Falls back to `base_mining_desire()` if no gold upgrades exist.

**Economy desire** — floors other desires while town has empty slots:
```
economy_desire = 1.0 - slot_fullness
food_desire = max(food_desire, economy_desire)
military_desire = max(military_desire, economy_desire)
gold_desire = max(gold_desire, economy_desire)
```
Prevents building scores from collapsing to zero while the town still has room to grow.

## Building Scoring

Each eligible action gets a score = `base_weight × need_multiplier`. All scores go into a weighted random draw. Desire signals act as multiplicative gates — when desire is 0, the corresponding building category scores 0.

### Need Multipliers

**Farms:** `food_desire × max(houses - farms, 0)`
- Zero when food is comfortable (food_desire = 0) or farms ≥ houses
- Scales with both food pressure and farm deficit vs houses

**Farmer homes:** `food_desire × min(deficit, 10)` when deficit > 0, else 0
- Deficit capped at 10 to prevent runaway scores with large targets (Economic: 2× farms)

**Archer homes:** `military_desire × deficit` when deficit > 0, else `military_desire × 0.5`
- Maintenance trickle (0.5) when at target keeps slow replacement of losses

**Crossbow homes:** Only scored when archer homes ≥ 2. Uses barracks base weight × 0.6:
- Below target: `military_desire × deficit`
- At target: `military_desire × 0.5`

**Miner homes:** Only scored when miner deficit > 0: `gold_desire × deficit`. Uses house base weight `hw`.

**Roads:** `road_weight × road_need` where `road_need = min(road_candidates, economy_buildings - roads/2)`. Pre-checks actual candidate availability via `count_road_candidates()` — if no road-pattern slots are available near economy buildings, roads aren't scored at all. Scored when `road_weight > 0` and food ≥ 4× road cost. Places roads in personality-specific grid patterns (see Personalities table) near economy buildings (farms, farmer homes, miner homes) within Chebyshev distance ≤ 2, scored by adjacency count. Batch places multiple roads per action (batch size per personality).

**Waypoints:** `military_desire × gap` where gap = total military homes − waypoints. Scored when waypoints < total military homes. Waypoints are placed on the personality's outer ring pattern (block corners on build area perimeter).

### Raider AI

Simpler: only scores `BuildTent` at flat weight 30.0 when it has slots and food.

## Slot Placement

Buildings use scored slot selection with fallback to center-nearest. Scorer functions are personality-specific (see Personalities section above). All non-Balanced personalities use the default scorers.

| Building | Scoring Strategy |
|----------|-----------------|
| Farm | Adjacency to existing farms, 2×2 block completion bonus, line bonus. Center-biased bootstrap for first farms. |
| Farmer home | Must border at least one farm. Edge adjacency weighted highest. |
| Archer home | Near economic core (farms + homes), anti-clump penalty for adjacent archer/crossbow homes. |
| Crossbow home | Same scoring as archer home (`archer_fill_score`). |
| Miner home | Minimizes distance to nearest gold mine. Center-biased fallback when no mines exist. |
| Waypoint | Personality's outer ring: block corners on build area perimeter adjacent to road intersections, min spacing per personality (Aggressive:3, Balanced:4, Economic:5). First unfilled ring slot wins. |

**Road and waypoint-aware placement:** All non-road building placement (both `find_inner_slot` and snapshot empty slots) filters out slots that match the personality's road pattern (`is_road_slot`) or waypoint ring (`waypoint_ring_slots`). This prevents buildings from being placed on future road or waypoint positions.

**Fallback:** If no snapshot or scorer produces a candidate, `find_inner_slot` picks the empty non-road slot closest to town center. All buildings use the unified `place_building` (world-position-based) — callers convert town grid coords to world_pos before calling.

## Mining & Expansion

`MineAnalysis` is computed once per AI tick (single pass over all gold mines):
- Counts mines in/outside mining radius
- Collects all alive mine positions (for miner home slot scoring)

**Flow** (miner and expand are mutually exclusive per tick):
1. Miner deficit > 0 → score BuildMinerHome (uses house base weight `hw`)
2. Miner deficit == 0 AND mines exist outside radius → score ExpandMiningRadius (increases policy radius by 300px, max 5000px)
3. Uncovered mines exist → score BuildWaypoint (independent of above, placed near nearest uncovered mine)

## Expansion Upgrade

The TownArea upgrade has special rules beyond normal upgrade scoring:
- Phase 2 blocks all non-expansion upgrades while town has empty slots (`has_slots && !is_expansion → skip`)
- Expansion itself is delayed while `has_slots` and AI can afford any building (cheapest of farm/farmer home/archer home/miner home) — ensures Phase 1 fills slots before expanding
- Urgency ramps with slot fullness: 70%→100% = 2×→6× weight
- Hard 10× boost when no empty slots remain

## Squad Commander

`ai_squad_commander_system` runs every frame (not every 5s). Both Builder and Raider AIs use squads. Squad counts and splits are personality-driven (see Personalities section). All military unit types (`SquadUnit` component: archers, crossbows, fighters, raiders) participate.

### Squad Roles (Builder AIs)

- **Reserve** (squad index 0): patrol enabled, no attack target, gets defense share % of military units
- **Attack** (squad indices 1+): patrol disabled, wave-based targeting, gets remainder of units
- **Idle** (excess squads beyond desired count): target_size = 0

### Raider Squads

Raider towns get 1 squad containing all raiders. No reserve/attack split — the single squad always attacks. Targets nearest enemy farm via `pick_raider_farm_target()`. Replaces the old `RaidQueue` group-formation system.

All squads have `rest_when_tired = true` (except raider squads: `rest_when_tired = false`).

### Wave-Based Attack Cycle

Attack squads use a gather→dispatch→retreat model instead of continuous retargeting:

1. **Gathering**: Squad accumulates members via `squad_cleanup_system` recruitment. No target set — units idle or patrol near base.
2. **Threshold**: When `members.len() >= wave_min_start` AND cooldown expired, pick a target.
3. **Dispatch**: Set squad target, `wave_active = true`, record `wave_start_count = members.len()`. All squad members redirect to target via squad sync in `decision_system`.
4. **End conditions**: Wave ends when target is destroyed OR alive members drop below `wave_retreat_below_pct` % of `wave_start_count` (heavy losses).
5. **Reset**: Clear target, `wave_active = false`, apply retarget cooldown with jitter. Squad returns to gathering.

### Wave Thresholds by Personality

| | Aggressive | Balanced | Economic | Raider |
|-|-----------|----------|----------|--------|
| **wave_min_start** | 3 | 5 | 8 | RAID_GROUP_SIZE (3) |
| **wave_retreat_pct** | 25% | 40% | 60% | 30% |

Search radius: 5000px from town center. Cooldown includes ±2s jitter. Initial cooldowns are desynchronized (0.3–1.0× base) to prevent synchronized AI waves.

## Perimeter Maintenance

`sync_patrol_perimeter_system` (dirty-flag-gated on `patrol_perimeter`):
1. Compute personality's ideal outer ring via `waypoint_ring_slots(tg)` (block corners on build area perimeter)
2. Prune waypoints not in the ideal ring (uses full `destroy_building` teardown) — when town area expands, the ring shifts outward and inner waypoints are destroyed
3. Recalculate clockwise patrol order (angle-based sort around town center)

## Migration (Dynamic Raider Towns)

`migration_spawn_system` + `migration_attach_system` + `migration_settle_system` in `economy.rs`.

### Spawn Trigger
Every **12 game hours**, if `raider_count < player_alive / VILLAGERS_PER_RAIDER (20)` and raider_towns < 20. In other words, one raider town is "earned" per 20 player NPCs alive.

### Flow
1. **Spawn**: 3 + player_alive/scaling raiders at random map edge (scaling = Easy:6, Normal:4, Hard:2)
2. **Walk**: Group walks toward nearest player town using Home + Wander behavior
3. **Settle**: When average group position is within 3000px of any town:
   - Snap town center to group position
   - Place buildings (town center + tents)
   - Stamp dirt terrain
   - Register tent spawners
   - Activate AI player
4. **Cancel**: If all members die before settling, migration is cancelled

Group size capped at 20 raiders. Random personality assigned at spawn.

## Building Costs

| Building | Food Cost |
|----------|----------|
| Farm | 2 |
| Farmer Home | 2 |
| Miner Home | 4 |
| Archer Home | 4 |
| Crossbow Home | 8 |
| Fighter Home | 5 |
| Waypoint | 1 |
| Tent | 3 |
