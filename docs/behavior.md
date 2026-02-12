# Behavior System

## Overview

NPC decision-making and state transitions. All run in `Step::Behavior` after combat is resolved. For economy systems (farm growth, starvation, camp foraging, raider respawning, game time), see [economy.md](economy.md).

**Unified Decision System**: All NPC decisions are handled by `decision_system` using a priority cascade. NPC state is modeled by two orthogonal enum components (concurrent state machines pattern):

- `Activity` enum: what the NPC is *doing* (Idle, Working, OnDuty, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain, Wandering, Raiding, Returning)
- `CombatState` enum: whether the NPC is *fighting* (None, Fighting, Fleeing)

Activity is preserved through combat — a Raiding NPC stays `Activity::Raiding` while `CombatState::Fighting`. When combat ends, the NPC resumes its previous activity.

The system uses **SystemParam bundles** for farm and economy parameters:
- `FarmParams`: farm states, `BuildingOccupancy` tracking, world data
- `EconomyParams`: food storage, food events, population stats

Priority order (first match wins):
0. AtDestination → Handle arrival transitions (match on Activity variant)
1. CombatState::Fighting + should_flee? → Flee
2. CombatState::Fighting + should_leash? → Leash
3. CombatState::Fighting → Skip (attack_system handles)
4a. HealingAtFountain + HP >= threshold → Wake (HP-only check)
4b. Resting + energy >= 90% → Wake (energy-only check)
5. Working + tired? → Stop work
6. OnDuty + time_to_patrol? → Patrol
7. Idle → Score Eat/Rest/Work/Wander (wounded → fountain, tired → home)

All checks are **policy-driven per town**. Flee thresholds come from `TownPolicies` resource (indexed by `TownId`), not per-entity `FleeThreshold` components. Raiders use a hardcoded 0.50 threshold. `guard_aggressive` and `farmer_fight_back` policies disable flee entirely for their respective jobs.

## Utility AI (Weighted Random Decisions)

**Idle NPCs** use utility AI for decisions. Instead of rigid rules (if tired→rest, else work), actions are scored and selected via weighted random. This creates lifelike, emergent behavior — a tired farmer with Focused trait might still choose Work over Rest sometimes.

The priority cascade (flee > leash > recovery > tired > patrol > wake > raid) handles **state checks** — deterministic "what state am I in" logic. Utility AI only kicks in at the end for NPCs with no active state.

### Personality Component

Each NPC has a `Personality` with 0-2 traits, each with a magnitude (0.5-1.5):

| Trait | Stat Effect | Behavior Effect |
|-------|-------------|-----------------|
| Brave(m) | +25% × m damage | Fight ×(1+m), Flee ×(1/(1+m)) |
| Tough(m) | +25% × m HP | Rest ×(1/(1+m)), Eat ×(1/(1+m)) |
| Swift(m) | +25% × m speed | Wander ×(1+m) |
| Focused(m) | +25% × m yield | Work ×(1+m), Wander ×(1/(1+m)) |

### Action Scoring

| Action | Base Score | Condition |
|--------|-----------|-----------|
| Eat | `(ENERGY_EAT_THRESHOLD - energy) * 1.5` | town has food AND energy < 10 |
| Rest | `(ENERGY_HUNGRY - energy) * 1.0` | home valid AND energy < ENERGY_HUNGRY |
| Work | `40.0 * hp_mult` | has job, HP > 30% |
| Wander | `10.0` | always |

**Eat action**: Instantly consumes 1 food from town storage and restores energy to 100. No travel required — NPCs eat at current location. Only available as emergency option when energy < `ENERGY_EAT_THRESHOLD` (10) — NPCs prefer resting over eating.

**HP-based work score**: `hp_mult = 0` if HP < 30%, otherwise `(hp_pct - 0.3) / 0.7`. This prevents critically wounded NPCs from working/raiding while still allowing starving NPCs (HP capped at 50%) to join raid queues at reduced priority.

**Note**: The code defines `Action::Fight` and `Action::Flee` in the enum, but these are not scored in decision_system. Fight/flee behavior is handled by combat systems (attack_system, flee_system) instead.

Scores are multiplied by personality multipliers, then weighted random selects an action:

```
Example: Energy=40, Tough(1.0) + Focused(1.0)
  Eat:    60 × 0.5 = 30
  Rest:   60 × 0.5 = 30
  Work:   40 × 2.0 = 80
  Wander: 10 × 0.5 = 5
  → 21% eat, 21% rest, 55% work, 3% wander
```

Same situation, different outcomes. That's emergent behavior.

## State Machine

Two concurrent state machines: `Activity` (what NPC is doing) and `CombatState` (fighting status). Activity is preserved through combat.

```
    Guard:                Farmer:               Stealer (Raider):
    ┌──────────┐         ┌──────────┐          ┌──────────┐
    │  OnDuty  │ spawn   │GoingToWork│ spawn   │  Idle    │ spawns idle
    │{ticks: 0}│         └────┬─────┘          └────┬─────┘
    └────┬─────┘              │ arrival              │ decision_system
         │ decision_system    ▼                      ▼
         ▼                ┌──────────┐          ┌──────────────────┐
    ┌──────────┐         │ Working  │          │ Raiding{target}  │
    │Patrolling│         └────┬─────┘          └────┬─────────────┘
    └────┬─────┘              │                     │ arrival at farm
         │ arrival            │                     ▼
         ▼                    │                ┌──────────────────┐
    ┌──────────┐              │                │Returning{food:T} │
    │  OnDuty  │              │                └────┬─────────────┘
    │{ticks: 0}│              │                     │ proximity delivery
    └────┬─────┘              │                     ▼
         └────────┬───────────┘                deliver food, re-enter
                  │ decision_system            decision_system
                  ▼ (weighted random)
             ┌──────────┐                  ┌──────────┐
             │GoingToRest│ (tired→home)    │GoingToHeal│ (wounded→fountain)
             └────┬─────┘                  └────┬─────┘
                  │ arrival                      │ arrival
                  ▼                              ▼
             ┌──────────┐                  ┌────────────────────────┐
             │ Resting  │ (energy recovery)│ HealingAtFountain      │
             └────┬─────┘                  │ {recover_until: 0.75}  │
                  │ energy >= 90%          └────┬───────────────────┘
                  ▼                              │ HP >= threshold
             back to previous cycle              ▼
                                           back to previous cycle

    Combat (orthogonal CombatState, Activity preserved):
    ┌─────────────────────┐                    ┌───────────────────┐
    │ CombatState::       │                    │ CombatState::None │
    │ Fighting{origin}    │──flee/leash───────▶│ Activity unchanged│
    │ Activity: preserved │                    │ (or Returning)    │
    └─────────────────────┘                    └───────────────────┘
```

## Components

### State Enums (Concurrent State Machines)

| Component | Variants | Purpose |
|-----------|----------|---------|
| Activity | `Idle, Working, OnDuty{ticks_waiting}, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain{recover_until}, Wandering, Raiding{target}, Returning{has_food}` | What the NPC is *doing* — mutually exclusive |
| CombatState | `None, Fighting{origin}, Fleeing` | Whether the NPC is *fighting* — orthogonal to Activity |

`Activity::is_transit()` returns true for Patrolling, GoingToWork, GoingToRest, GoingToHeal, Wandering, Raiding, Returning. Used by `gpu_position_readback` for arrival detection.

`Resting` is a unit variant — energy recovery only. NPCs go home (spawner) to rest.

`HealingAtFountain { recover_until: 0.75 }` — HP recovery at town fountain. NPC waits until HP >= threshold, then resumes. Separate from energy rest.

`Returning { has_food: true }` replaces old `CarryingFood` marker — food carried state is part of the activity.

### Data Components

| Component | Type | Purpose |
|-----------|------|---------|
| TownId | `i32` | Town/camp identifier — every NPC belongs to one |
| Energy | `f32` | 0-100, drains while active, recovers while resting |
| Personality | `{ trait1, trait2 }` | 0-2 traits with magnitude affecting stats and decisions |
| AssignedFarm | `Vec2` | Farm position farmer is working at (for occupancy tracking) |
| Starving | marker | NPC energy at zero (50% HP cap, 50% speed) |
| Healing | marker | NPC is inside healing aura (visual feedback) |
| MaxHealth | `f32` | NPC's maximum health (for healing cap) |
| Home | `{ x, y }` | NPC's spawner building position (house/barracks/tent) — rest destination |
| WorkPosition | `{ x, y }` | Farmer's field position |
| PatrolRoute | `{ posts: Vec<Vec2>, current: usize }` | Guard's ordered patrol posts |
| AtDestination | marker | NPC arrived at destination (transient frame flag from gpu_position_readback) |
| Stealer | marker | NPC steals from farms (enables steal systems) |
| LeashRange | `{ distance: f32 }` | Disengage combat if chased this far from combat origin (raiders only) |

## Systems

### decision_system (Unified Priority Cascade)
- Query: NPCs with `&mut Activity`, `&mut CombatState`, skips NPCs in transit (`activity.is_transit()`)
- Uses **SystemParam bundles** for farm and economy parameters (see Overview)
- Matches on Activity and CombatState enums in priority order:

**Priority 0: Arrival transitions**
- If `AtDestination`: match on Activity variant
  - `Patrolling` → `Activity::OnDuty { ticks_waiting: 0 }`
  - `GoingToRest` → `Activity::Resting` (sleep icon derived by `sync_visual_sprites`)
  - `GoingToHeal` → `Activity::HealingAtFountain { recover_until: policy.recovery_hp }` (healing aura handles HP recovery)
  - `GoingToWork` → check `BuildingOccupancy`: if farm occupied, redirect to nearest free farm in own town (or idle if none); else claim farm via `BuildingOccupancy.claim()` + `AssignedFarm` + harvest if ready
  - `Raiding { .. }` → steal if farm ready, else find a different farm (excludes current position, skips tombstoned); if no other farm exists, return home
  - `Wandering` → `Activity::Idle` (wander targets are offset from home position, not current position, preventing unbounded drift)
- Removes `AtDestination` after handling

**Priority 1-3: Combat decisions**
- If `CombatState::Fighting` + should flee: policy-driven flee thresholds per job — guards use `guard_flee_hp`, farmers use `farmer_flee_hp`, raiders hardcoded 0.50. `guard_aggressive` disables guard flee, `farmer_fight_back` disables farmer flee. Dynamic threat assessment via `count_nearby_factions()` (enemies vs allies within 200px, throttled every 30 frames) — reads `GpuReadState.factions` and `GpuReadState.health` from GPU readback
- If `CombatState::Fighting` + should leash: guards check `guard_leash` policy (if disabled, guards chase freely), raiders use per-entity `LeashRange` component
- If `CombatState::Fighting`: skip (attack_system handles targeting)

**Priority 4a: HealingAtFountain wake**
- If `HealingAtFountain { recover_until }` + HP / max_hp >= recover_until: set `Activity::Idle`

**Priority 4b: Resting wake**
- If `Activity::Resting` + energy >= `ENERGY_WAKE_THRESHOLD` (90%): set `Activity::Idle`, proceed to scoring

**Priority 5: Tired workers**
- If `Activity::Working` + energy < `ENERGY_TIRED_THRESHOLD` (30%): set `Activity::Idle`, release `AssignedFarm`

**Priority 6: Patrol**
- If `Activity::OnDuty { ticks_waiting }` + ticks >= `GUARD_PATROL_WAIT` (60): advance `PatrolRoute`, set `Activity::Patrolling`

**Priority 7: Idle scoring (Utility AI)**
- **Healing priority**: if `prioritize_healing` policy enabled, energy > 0, HP < `recovery_hp`, and town center known → `GoingToHeal` targeting fountain. Applies to all jobs (including raiders — they heal at their camp center). Skipped when starving (energy=0) because HP is capped at 50% by starvation — NPC must rest for energy first.
- **Work schedule gate**: Work only scored if the per-job schedule allows it — farmers use `farmer_schedule`, guards use `guard_schedule` (`Both` = always, `DayOnly` = hours 6-20, `NightOnly` = hours 20-6)
- **Off-duty behavior**: when work is gated out by schedule, off-duty policy applies: `GoToBed` boosts Rest to 80, `StayAtFountain` targets town center, `WanderTown` boosts Wander to 80
- Score Eat/Rest/Work/Wander with personality multipliers and HP modifier
- Select via weighted random, execute action
- **Food check**: Eat only scored if town has food in storage
- **Decision logging**: Each decision logged to `NpcLogCache`

### on_duty_tick_system
- Query: NPCs with `Activity::OnDuty { ticks_waiting }` where `CombatState` is not Fighting
- Increments `ticks_waiting` each frame
- Separated from decision_system to allow mutable Activity access while main query has immutable view

### arrival_system (Proximity Checks)
- **Proximity-based delivery** for Returning raiders: matches `Activity::Returning { .. }`, checks distance to home, delivers food within DELIVERY_RADIUS (150px), sets `Activity::Idle`
- **Working farmer drift check** (throttled every 30 frames): re-targets farmers who drifted >20px from their assigned farm
- Arrival detection (`is_transit()` → `AtDestination`) is handled by `gpu_position_readback` in movement.rs
- All state transitions handled by decision_system Priority 0 (central brain model)

### energy_system
- NPCs with `Activity::Resting` or `Activity::HealingAtFountain`: recover `ENERGY_RECOVER_RATE` per tick
- All other NPCs: drain `ENERGY_DRAIN_RATE` per tick
- Clamp to 0.0-100.0
- **Note**: HealingAtFountain also recovers energy to prevent ping-pong (NPC leaves fountain tired → goes home → not healed → returns to fountain)
- All state transitions (wake-up, stop working) are handled in decision_system to keep decisions centralized

### healing_system
- Query: NPCs with `Health`, `MaxHealth`, `Faction`, `TownId` (without `Dead`)
- Reads NPC position from `GpuReadState`, town centers from `WorldData`
- All settlements (villager and raider) are Town entries with faction (unified town model)
- If NPC within `HEAL_RADIUS` (150px) of same-faction town center: heal `HEAL_RATE` (5 HP/sec)
- **Starvation HP cap**: Starving NPCs have HP capped at 50% of MaxHealth (can't heal above this)
- Adds/removes `Healing` marker component for visual feedback (heal icon derived by `sync_visual_sprites`)
- Debug: `get_health_debug()` returns healing_in_zone_count and healing_healed_count

*Economy systems (game_time, farm_growth, camp_forage, raider_respawn, starvation) documented in [economy.md](economy.md).*

*Farm growth, starvation, and group raid systems documented in [economy.md](economy.md).*

## Energy Model

Energy uses game time (respects time_scale and pause):

| Constant | Value | Purpose |
|----------|-------|---------|
| ENERGY_RECOVER_PER_HOUR | 100/6 (~16.7) | Recovery while resting (6 hours to full) |
| ENERGY_DRAIN_PER_HOUR | 100/12 (~8.3) | Drain while active (12 hours to empty) |
| ENERGY_WAKE_THRESHOLD | 90.0 | Wake from Resting when energy reaches this |
| ENERGY_TIRED_THRESHOLD | 30.0 | Stop working and seek rest below this |
| ENERGY_EAT_THRESHOLD | 10.0 | Emergency eat threshold — Eat only scored below this |

Rest is scored when energy < `ENERGY_HUNGRY` (50), Eat only when energy < `ENERGY_EAT_THRESHOLD` (10). This means NPCs strongly prefer resting over eating, only consuming food as a last resort. NPCs go home (spawner building) to rest, and wake at 90% energy. Wounded NPCs go to the town fountain to heal (separate `GoingToHeal` / `HealingAtFountain` activity).

## Patrol Cycle

Guards have a `PatrolRoute` with ordered posts (built from WorldData at spawn). The cycle:

1. Spawn → walk to post 0 (`Patrolling`)
2. Arrive → stand at post (`OnDuty`, ticks counting)
3. 60 ticks → advance to post 1 (`Patrolling`)
4. Arrive → `OnDuty` again
5. After last post, wrap to post 0

Each town has 4 guard posts at corners. Guards cycle clockwise. Patrol routes are rebuilt dynamically by `rebuild_patrol_routes_system` (runs in `Step::Behavior`) when `WorldData` changes — e.g. guard posts added, removed, or reordered via the Patrols tab. The system clamps the current patrol index to the new route length.

## Known Issues / Limitations

- **No pathfinding**: NPCs walk in a straight line to target. They rely on separation physics to avoid each other, but can't navigate around buildings.
- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).

## Rating: 8/10

Central brain architecture: `decision_system` handles all NPC decisions with clear priority cascade. Policy-driven flee/leash/schedule system replaces per-entity threshold components. Utility AI for idle decisions creates lifelike behavior with work schedule and off-duty policies.

Gaps: no pathfinding.
