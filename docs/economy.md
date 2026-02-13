# Economy System

## Overview

Economy systems handle time progression, food production, starvation, camp foraging, and AI decisions. All run in `Step::Behavior` and use `GameTime.hour_ticked` for hourly event gating. Defined in `rust/src/systems/economy.rs` and `rust/src/systems/ai_player.rs`.

## Data Flow

```
game_time_system (every frame)
    │
    ▼ sets hour_ticked = true when hour changes
    │
    ├─ farm_growth_system (every frame, uses game-time delta)
    │   └─ FarmStates: Growing → Ready when progress >= 1.0
    │
    ├─ camp_forage_system (hourly)
    │   └─ Each raider camp gains CAMP_FORAGE_RATE food
    │
    ├─ spawner_respawn_system (hourly)
    │   └─ Detects dead NPCs linked to House/Barracks/Tent, counts down 12h timer, spawns replacement
    │
    ├─ starvation_system (hourly)
    │   └─ NPCs with zero energy → Starving marker
    │
    ├─ mine_regen_system (every frame, uses game-time delta)
    │   └─ MineStates: gold slowly regenerates when mine is unoccupied
    │
    ├─ farm_visual_system (every frame)
    │   └─ FarmStates Growing→Ready: spawn FarmReadyMarker; Ready→Growing: despawn
    │
    ├─ job_reassign_system (every frame, after decision_system)
    │   └─ Converts idle farmers↔miners per town to match MinerTarget
    │
    ├─ ai_decision_system (real-time interval, default 5s)
    │   └─ Per AI settlement: build → unlock slots → buy upgrades (food-gated, personality-driven)
    │
    └─ squad_cleanup_system (every frame)
        └─ Removes dead NPC slots from Squad.members via NpcEntityMap check
```

## Systems

### game_time_system
- Advances `GameTime.total_seconds` based on `time.delta_secs() * time_scale`
- Sets `hour_ticked = true` when the hour changes (single source of truth for hourly events)
- Respects `paused` flag
- Other systems check `game_time.hour_ticked` instead of tracking their own timers

### farm_growth_system
- Runs every frame, advances farm growth based on elapsed game time
- **FarmStates resource**: tracks `Growing` vs `Ready` state and progress (0.0-1.0) per farm
- **Hybrid growth model**:
  - Passive: `FARM_BASE_GROWTH_RATE` (0.08/hour) — ~12 game hours to full growth
  - Tended: `FARM_TENDED_GROWTH_RATE` (0.25/hour) — ~4 game hours with farmer working
- Farm transitions to `Ready` when progress >= 1.0
- Checks `BuildingOccupancy.is_occupied()` to determine if a farmer is tending
- **FarmYield upgrade**: growth rate multiplied by `1.0 + level * 0.15` per-town (reads `TownUpgrades` via `farm.town_idx`)

### camp_forage_system
- Runs when `game_time.hour_ticked` is true
- Each raider camp (faction > 0) gains `CAMP_FORAGE_RATE` (1) food per hour
- Passive income ensures raiders can survive even if they never steal

### spawner_respawn_system
- Runs when `game_time.hour_ticked` is true
- Each `SpawnerEntry` in `SpawnerState` links a House (farmer), Barracks (guard), or Tent (raider) to an NPC slot
- If `npc_slot >= 0` and NPC is dead (not in `NpcEntityMap`): starts 12h respawn timer
- Timer decrements 1.0 per game hour; on expiry: allocates slot via `SlotAllocator`, emits `SpawnNpcMsg`, logs to `CombatLog`
- Newly-built spawners start with `respawn_timer: 0.0` — the `>= 0.0` check catches these, spawning an NPC on the next hourly tick
- Tombstoned entries (position.x < -9000) are skipped (building was destroyed)
- House → Farmer (nearest **free** farm in own town via `find_nearest_free` — skips occupied farms), Barracks → Guard (nearest guard post, home = building position), Tent → Raider (home = tent position). All spawner types look up faction from `world_data.towns[town_idx].faction`.

### starvation_system
- Runs when `game_time.hour_ticked` is true
- NPCs with `energy <= 0` get `Starving` marker
- Starving NPCs: speed set to `CachedStats.speed * STARVING_SPEED_MULT` (0.5) via `GpuUpdate::SetSpeed`
- When energy rises above 0 (eating or resting): `Starving` removed, speed restored to `CachedStats.speed`

## Farm Growth

Farms have a growth cycle instead of infinite food:

```
     farm_growth_system advances progress
              │
              ▼
┌──────────┐      progress >= 1.0      ┌─────────┐
│ Growing  │ ────────────────────────▶ │  Ready  │ (shows food icon)
│progress++│                           │progress=1│
└──────────┘                           └────┬────┘
      ▲                                     │
      │            farmer/raider arrival    │
      └─────────────────────────────────────┘
              reset to Growing, progress=0

```

**Farmer harvest** (decision_system, GoingToWork → Working arrival):
- Uses `find_location_within_radius()` to find farm within FARM_ARRIVAL_RADIUS (20px) of **WorkPosition** (not current position)
- If farm is Ready: add food to town storage, reset farm to Growing
- Logs "Harvested → Working" vs "→ Working (tending)"

**Raider steal** (decision_system, Raiding arrival):
- Uses `find_location_within_radius()` to find farm within FARM_ARRIVAL_RADIUS
- Only steals if farm is Ready — reset farm to Growing, set CarryingFood + Returning
- If farm not ready: find a different farm (excludes current position, skips tombstoned); if no other farm found, return home
- Logs "Stole food → Returning" vs "Farm not ready, seeking another" vs "No other farms, returning"

**Visual feedback**: `farm_visual_system` watches `FarmStates` for state transitions and spawns/despawns `FarmReadyMarker` entities. Uses `Local<Vec<FarmGrowthState>>` to detect transitions without extra resources. Growing→Ready spawns a marker; Ready→Growing (harvest) despawns it.

## Starvation

Energy is the single survival resource. When energy hits zero, the NPC is starving:

```
energy_system drains energy while active
        │
        ▼  starvation_system (hourly)
energy <= 0?
        │
    YES ▼
┌────────────┐
│  Starving  │  - HP capped at 50%
│   marker   │  - Speed reduced to 50%
└────┬───────┘
     │ energy > 0 (eating or resting)
     ▼
Starving marker removed
Speed restored to CachedStats.speed
```

**Recovery paths:**
- **Eat**: consumes 1 food from town storage, instantly restores energy to 100. No travel required.
- **Rest**: walk home to spawner building (House/Barracks), recover energy slowly (6 hours 0→100). Works even when starving.

**Constants:**
- `STARVING_HP_CAP`: 0.5 (50% of MaxHealth)
- `STARVING_SPEED_MULT`: 0.5 (50% of normal speed)

Starvation applies to **both villagers and raiders**. If raiders can't steal food and their camp runs out, they'll starve and become easier to kill.

The HP cap is enforced by `healing_system` — starving NPCs can't heal above 50% MaxHealth even inside a healing aura.

## Group Raid System

Raiders coordinate into groups before raiding (like Factorio biters):

```
decision_system: Raider picks Work
        │
        ▼
Add (entity, idx) to RaidQueue.waiting[faction]
(only if not already in queue)
        │
        ▼
queue.len() >= 5?
        │
   NO ──┼──▶ Wander near camp (stays in queue)
        │
   YES ──▶ Dispatch ALL waiting raiders:
           - Remove Wandering marker
           - Insert Raiding marker
           - Set target to nearest farm
           - Clear queue
```

**RaidQueue resource:**
```rust
pub struct RaidQueue {
    pub waiting: HashMap<i32, Vec<(Entity, usize)>>,
}
```

**Key behaviors:**
- Raiders join queue when picking Work action in decision_system
- Queue checked inline (no separate system)
- All 5+ raiders dispatched to same farm target
- Dead raiders removed from queue in death_cleanup_system
- Transit skip includes Raiding and Returning (no mid-journey decisions)

**Constants:**
- `RAID_GROUP_SIZE`: 5 (minimum raiders to form a group)

Solo raiders **wait at camp** instead of raiding alone. They wander near home until a group forms.

## Resources

| Resource | Purpose | Updated By |
|----------|---------|------------|
| GameTime | total_seconds, time_scale, paused, hour_ticked | game_time_system |
| FoodStorage | `Vec<i32>` — food count per town/camp | harvest, steal, forage, respawn |
| FoodEvents | delivered/consumed event logs | arrival_system, decision_system |
| FarmStates | Growing/Ready state + progress per farm | farm_growth_system, harvest/steal |
| GoldStorage | `Vec<i32>` — gold count per town/camp | mining delivery, UI |
| MineStates | gold, max_gold, positions per mine | mine_regen_system, mining behavior |
| BuildingOccupancy | private map, methods: claim/release/is_occupied/count/clear | decision_system, death_cleanup, game_startup, spawner_respawn |
| CampState | max_pop, respawn_timers, forage_timers | camp_forage_system |
| RaidQueue | `HashMap<faction, Vec<(Entity, slot)>>` | decision_system, death_cleanup |
| SpawnerState | `Vec<SpawnerEntry>` — building→NPC links + respawn timers | spawner_respawn_system, game_startup |
| MinerTarget | `Vec<i32>` — desired miner count per town | left_panel (UI), ai_decision_system | job_reassign_system |
| PopulationStats | alive/working/dead per (job, town) | spawn, death, state transitions |

## Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| FARM_BASE_GROWTH_RATE | 0.08/hour | Passive growth (~12h to harvest) |
| FARM_TENDED_GROWTH_RATE | 0.25/hour | Tended growth (~4h to harvest) |
| CAMP_FORAGE_RATE | 1 food/hour | Passive raider food income |
| TENT_BUILD_COST | 1 | Food cost to build a Tent |
| STARVING_HP_CAP | 0.5 | 50% MaxHealth cap while starving |
| STARVING_SPEED_MULT | 0.5 | 50% speed while starving |
| RAID_GROUP_SIZE | 5 | Min raiders to form a raid group |
| HOUSE_BUILD_COST | 1 | Food cost to build a House |
| BARRACKS_BUILD_COST | 1 | Food cost to build a Barracks |
| SPAWNER_RESPAWN_HOURS | 12.0 | Game hours before dead NPC respawns from building |
| MINE_MAX_GOLD | 200.0 | Maximum gold a mine can hold |
| MINE_REGEN_RATE | 2.0/hour | Gold regeneration rate (when unoccupied) |
| MINE_EXTRACT_PER_CYCLE | 5 | Gold extracted per mining work cycle |
| MINE_MIN_SETTLEMENT_DIST | 300.0px | Minimum distance from mine to any town/camp center |
| MINE_MIN_SPACING | 400.0px | Minimum distance between mines |

### mine_regen_system
- Runs every frame, advances mine gold based on elapsed game time
- Only regenerates when mine has no occupant (`BuildingOccupancy.is_occupied()` returns false)
- Rate: `MINE_REGEN_RATE` (2.0 gold/hour), capped at `MINE_MAX_GOLD` (200.0) per mine
- Uses `MineStates` resource — parallel Vecs of gold, max_gold, and positions per mine

### job_reassign_system
- Runs every frame in `Step::Behavior`, after `decision_system`
- Reads `MinerTarget` resource (per-town desired miner count)
- Counts current miners per town, compares to target
- **diff > 0** (need more miners): converts idle/resting farmers → miners. Removes `Farmer`/`WorkPosition`/`AssignedFarm`, inserts `Miner`, updates sprite to `SPRITE_MINER`, updates `NpcMetaCache.job`
- **diff < 0** (need fewer miners): converts idle/resting miners → farmers. Removes `Miner`, inserts `Farmer` + `WorkPosition` (nearest free farm), updates sprite to `SPRITE_FARMER`, updates `NpcMetaCache.job`
- Only touches NPCs in `Activity::Idle` or `Activity::Resting` — never interrupts working/mining/fighting NPCs
- Updates `BuildingOccupancy` when releasing mine positions or assigning farm positions

### squad_cleanup_system
- Runs every frame in `Step::Behavior`
- Iterates all squads, retains only members whose slot is still in `NpcEntityMap` (alive)
- Lightweight scan — no allocation, just `Vec::retain`

## Known Issues

None currently.

## Rating: 8/10

Farm growth cycle creates meaningful gameplay loop — farmers tend crops, raiders steal harvests, camps forage passively. Group raid coordination prevents solo suicide runs. Starvation adds survival pressure to both factions. Game time system is clean with single `hour_ticked` flag. FarmYield upgrade scales per-town via `TownUpgrades`. Starvation uses resolved `CachedStats.speed` instead of hardcoded constants. Unified spawner system handles all three NPC types (farmer/guard/raider) through a single `spawner_respawn_system`. Weaknesses: no visual feedback for farm state, population helpers use raw `(job, town)` tuple keys.
