# AI Player System

Autonomous opponents that build, upgrade, and fight like the player. Each AI settlement gets a personality that drives all decisions through weighted random scoring — same pattern as NPC behavior.

**Source**: `rust/src/systems/ai_player.rs` (decisions, building, squads), `rust/src/systems/economy.rs` (migration)

## AI Kinds

| Kind | Spawned By | Buildings | NPCs |
|------|-----------|-----------|------|
| **Builder** | World gen (AI towns) | Farms, farmer homes, archer homes, crossbow homes, fighter homes, miner homes, waypoints | Farmers, archers, crossbows, fighters, miners |
| **Raider** | Migration system (dynamic camps) | Tents | Raiders |

## Personalities

Assigned randomly at creation. Drives every decision the AI makes. All personality-specific values in one place:

### Building Economy

| | Aggressive | Balanced | Economic |
|-|-----------|----------|----------|
| **Build weights** (farm/house/archer/waypoint) | 10/10/30/20 | 20/20/15/10 | 30/25/5/5 |
| **Farmer home target** | 1:1 with farms | farms + 1 | 2× farms |
| **Archer home target** | 1:1 with homes | homes / 2 | 1 + homes / 3 |
| **Food reserve per spawner** | 0 | 1 | 2 |
| **Slot placement: farms** | `farm_slot_score` (adjacency + 2×2 block + line bonus) | `balanced_farm_ray_score` (cardinal axis rays from center) | same as Aggressive |
| **Slot placement: homes** | `farmer_home_border_score` (must border farms) | `balanced_house_side_score` (beside axis rays, not on them) | same as Aggressive |

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
5. Compute hunger signal
6. Build TownContext (center, food, has_slots, slot_fullness, MineAnalysis for Builders)
7. Count buildings via building_counts() → HashMap<BuildingKind, usize>, compute targets and deficits
8. Score all eligible actions (buildings + upgrades)
9. Weighted random pick → execute one action
10. Invalidate snapshot on successful build, log to combat log
```

### Food Reserve Gate

```
reserve = food_reserve_per_spawner × spawner_count
```

Every spawner building (farmer home, archer home, crossbow home, fighter home, miner home) counts. The AI won't spend food if at or below reserve. This prevents self-starvation but also slows building as the town grows.

### Hunger Signal

Drives farm and farmer home urgency when food margin is thin. Computed after the reserve gate:

```
hunger = clamp(1.0 - (food - reserve) / reserve, 0.0, 1.0)
```

- `hunger = 0.0` → food is at 2× reserve or higher (comfortable)
- `hunger = 1.0` → food is at reserve floor (maximum urgency)
- Aggressive (reserve=0) uses absolute fallback: food < 5 → 0.8, food < 10 → 0.4, else 0.0

Hunger boosts farm and farmer home need multipliers (see scoring below).

## Building Scoring

Each eligible action gets a score = `base_weight × need_multiplier`. All scores go into a weighted random draw.

### Need Multipliers

**Farms:**
```
farm_need = 1.0 + max(houses - farms, 0) + hunger × 4.0
```
- Ratio signal: farms only get boosted when homes exceed farms
- Hunger signal: up to +4.0 when at food floor (e.g., Balanced base 20 × 5.0 = score 100)

**Farmer homes:**
```
if house_deficit > 0:  1.0 + deficit + hunger × 3.0
elif hunger > 0.3:     1.0 + hunger × 2.0     ← builds homes past target when hungry
else:                  0.5                      ← relaxed when at target and fed
```

**Archer homes:**
```
if barracks_deficit > 0:  1.0 + deficit + military_desire × 3.0
else:                     0.5 + military_desire
```

**Crossbow homes:** Only scored when archer homes ≥ 2 (established military base). Uses barracks base weight × 0.6:
```
if xbow_homes < archer_homes / 2:  1.0 + military_desire × 2.0
else:                               0.3 + military_desire
```

**Miner homes:** Only scored when miner deficit > 0: `1.0 + deficit`. Uses house base weight `hw`.

**Waypoints:** Scored when uncovered mines exist (mine_need = 1.0 + uncovered_count, no slot requirement) or when waypoints < total military homes (archer + crossbow) AND town has slots. Waypoint cost check is independent of `has_slots` since waypoints can be placed in wilderness.

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
| Waypoint | Two-stage: (1) wilderness near nearest uncovered mine if spacing OK, (2) in-town perimeter slot maximizing spacing then radial distance. |

**Fallback:** If no snapshot or scorer produces a candidate, `find_inner_slot` picks the empty slot closest to town center. Waypoints use `place_waypoint_at_world_pos` (world-position-based, not town-slot-based) so they can be placed both in-town and in wilderness.

## Mining & Expansion

`MineAnalysis` is computed once per AI tick (single pass over all gold mines):
- Counts mines in/outside mining radius
- Finds uncovered mines (no friendly waypoint within 200px)
- Tracks nearest uncovered mine for waypoint targeting

**Flow** (miner and expand are mutually exclusive per tick):
1. Miner deficit > 0 → score BuildMinerHome (uses house base weight `hw`)
2. Miner deficit == 0 AND mines exist outside radius → score ExpandMiningRadius (increases policy radius by 300px, max 5000px)
3. Uncovered mines exist → score BuildWaypoint (independent of above, placed near nearest uncovered mine)

## Expansion Upgrade

The TownArea upgrade has special rules beyond normal upgrade scoring:
- Delayed while building deficits exist and town has slots (homes, total military homes, or miners below target)
- Urgency ramps with slot fullness: 70%→100% = 2×→6× weight
- Hard 10× boost when no empty slots remain

## Squad Commander

`ai_squad_commander_system` runs every frame (not every 5s). Both Builder and Raider AIs use squads. Squad counts and splits are personality-driven (see Personalities section). All military unit types (`SquadUnit` component: archers, crossbows, fighters, raiders) participate.

### Squad Roles (Builder AIs)

- **Reserve** (squad index 0): patrol enabled, no attack target, gets defense share % of military units
- **Attack** (squad indices 1+): patrol disabled, wave-based targeting, gets remainder of units
- **Idle** (excess squads beyond desired count): target_size = 0

### Raider Squads

Raider camps get 1 squad containing all raiders. No reserve/attack split — the single squad always attacks. Targets nearest enemy farm via `pick_raider_farm_target()`. Replaces the old `RaidQueue` group-formation system.

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
1. Compute owned territory (farms + all unit homes + miner homes, via `territory_building_sets!` macro using `WorldData.homes(kind)` BTreeMap access)
2. Compute one-cell perimeter ring around territory (orthogonal directions only)
3. Prune in-town waypoints no longer on perimeter (uses full `destroy_building` teardown)
4. Preserve wilderness waypoints (outside town build area)
5. Recalculate clockwise patrol order (angle-based sort around town center)

## Migration (Dynamic Raider Camps)

`migration_spawn_system` + `migration_attach_system` + `migration_settle_system` in `economy.rs`.

### Spawn Trigger
Every **12 game hours**, if `camp_count < player_alive / VILLAGERS_PER_CAMP (20)` and camps < 20. In other words, one camp is "earned" per 20 player NPCs alive.

### Flow
1. **Spawn**: 3 + player_alive/scaling raiders at random map edge (scaling = Easy:6, Normal:4, Hard:2)
2. **Walk**: Group walks toward nearest player town using Home + Wander behavior
3. **Settle**: When average group position is within 3000px of any town:
   - Snap town center to group position
   - Place camp buildings (center + tents)
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
