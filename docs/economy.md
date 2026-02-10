# Economy System

## Overview

Economy systems handle time progression, food production, starvation, camp foraging, and raider respawning. All run in `Step::Behavior` and use `GameTime.hour_ticked` for hourly event gating. Defined in `rust/src/systems/economy.rs`.

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
    ├─ raider_respawn_system (hourly)
    │   └─ Camps with food + population room → spawn raider
    │
    ├─ starvation_system (hourly)
    │   └─ NPCs with zero energy → Starving marker
    │
    └─ farm_visual_system (every frame)
        └─ FarmStates Growing→Ready: spawn FarmReadyMarker; Ready→Growing: despawn
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
- Checks `FarmOccupancy` to determine if a farmer is tending (position key lookup)
- **FarmYield upgrade**: growth rate multiplied by `1.0 + level * 0.15` per-town (reads `TownUpgrades` via `farm.town_idx`)

### camp_forage_system
- Runs when `game_time.hour_ticked` is true
- Each raider camp (faction > 0) gains `CAMP_FORAGE_RATE` (1) food per hour
- Passive income ensures raiders can survive even if they never steal

### raider_respawn_system
- Runs when `game_time.hour_ticked` is true
- Camps with food >= `RAIDER_SPAWN_COST` (5) and population < `CAMP_MAX_POP` (10) spawn a new raider
- Spawns at camp center via `SpawnNpcMsg`
- Subtracts food cost after spawning

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
- If farm not ready: re-target another farm via `find_nearest_location()`
- Logs "Stole food → Returning" vs "Farm not ready, seeking another"

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
- **Rest**: walk home to bed, recover energy slowly (6 hours 0→100). Works even when starving.

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
| FarmOccupancy | `HashMap<pos, count>` — workers per farm | decision_system, death_cleanup |
| CampState | max_pop, respawn_timers, forage_timers | camp_forage_system, raider_respawn |
| RaidQueue | `HashMap<faction, Vec<(Entity, slot)>>` | decision_system, death_cleanup |
| PopulationStats | alive/working/dead per (job, town) | spawn, death, state transitions |

## Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| FARM_BASE_GROWTH_RATE | 0.08/hour | Passive growth (~12h to harvest) |
| FARM_TENDED_GROWTH_RATE | 0.25/hour | Tended growth (~4h to harvest) |
| CAMP_FORAGE_RATE | 1 food/hour | Passive raider food income |
| RAIDER_SPAWN_COST | 5 food | Cost to respawn a raider |
| CAMP_MAX_POP | 10 | Max raiders per camp |
| STARVING_HP_CAP | 0.5 | 50% MaxHealth cap while starving |
| STARVING_SPEED_MULT | 0.5 | 50% speed while starving |
| RAID_GROUP_SIZE | 5 | Min raiders to form a raid group |

## Known Issues

- **No carried item rendering**: CarriedItem component exists but nothing draws the food icon.

## Rating: 8/10

Farm growth cycle creates meaningful gameplay loop — farmers tend crops, raiders steal harvests, camps forage passively. Group raid coordination prevents solo suicide runs. Starvation adds survival pressure to both factions. Game time system is clean with single `hour_ticked` flag. FarmYield upgrade scales per-town via `TownUpgrades`. Starvation uses resolved `CachedStats.speed` instead of hardcoded constants. Weaknesses: no visual feedback for farm state, population helpers use raw `(job, town)` tuple keys.
