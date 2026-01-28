# Behavior System

## Overview

Behavior systems manage NPC lifecycle outside of combat: energy drain/recovery, rest transitions, patrol cycling for guards, work transitions for farmers, and stealing/fleeing for raiders. All run in `Step::Behavior` after combat is resolved.

All systems are **component-driven, not job-driven**. A system like `flee_system` operates on any NPC with `FleeThreshold` + `InCombat`, regardless of whether it's a guard or raider.

## State Machine

```
    Guard:                Farmer:               Stealer (Raider):
    ┌──────────┐         ┌──────────┐          ┌──────────┐
    │Patrolling│         │GoingToWork│         │  (idle)  │ spawns stateless
    └────┬─────┘         └────┬─────┘          └────┬─────┘
         │ arrival            │ arrival              │ steal_decision_system
         ▼                    ▼                      ▼
    ┌──────────┐         ┌──────────┐          ┌──────────┐
    │  OnDuty  │         │ Working  │          │  Raiding  │ (walk to farm)
    │ 60 ticks │         │          │          └────┬─────┘
    └────┬─────┘         └────┬─────┘               │ arrival at farm
         ▼                    ▼                      ▼
    ┌──────────┐         ┌──────────┐          ┌──────────┐
    │  OnDuty  │         │ Working  │          │Returning │ (+CarryingFood)
    │ 60 ticks │         │          │          │(to camp) │
    └────┬─────┘         └────┬─────┘          └────┬─────┘
         └────────┬───────────┘                     │ arrival at camp
                  │ energy < 50                     ▼
                  ▼                            deliver food, re-enter
             ┌──────────┐                     steal_decision_system
             │GoingToRest│
             └────┬─────┘
                  │ arrival
                  ▼
             ┌──────────┐
             │ Resting  │
             └────┬─────┘
                  │ energy >= 80
                  ▼
             back to previous cycle

    Combat escape (any NPC with FleeThreshold/LeashRange):
    ┌──────────┐                         ┌────────────┐
    │ InCombat │──health < FleeThreshold─▶│ Returning  │
    │          │──dist > LeashRange──────▶│ (go home)  │
    └──────────┘                          └─────┬──────┘
                                                │ arrival (wounded)
                                                ▼
                                          ┌────────────┐
                                          │ Recovering │ (+Resting)
                                          │ until 75%  │
                                          └────────────┘
```

## Components

| Component | Type | Purpose |
|-----------|------|---------|
| Clan | `i32` | Town/camp identifier — every NPC belongs to one |
| Energy | `f32` | 0-100, drains while active, recovers while resting |
| Resting | marker | NPC is at home, recovering energy |
| GoingToRest | marker | NPC is walking home to rest |
| Patrolling | marker | Guard is walking to next patrol post |
| OnDuty | `{ ticks: u32 }` | Guard is stationed at a post |
| Working | marker | Farmer is at work position |
| GoingToWork | marker | Farmer is walking to work |
| Raiding | marker | NPC is walking to a farm to steal |
| Returning | marker | NPC is walking back to home base |
| CarryingFood | marker | NPC has stolen food |
| Recovering | `{ threshold: f32 }` | NPC is resting until HP >= threshold |
| Home | `{ x, y }` | NPC's home/bed position |
| WorkPosition | `{ x, y }` | Farmer's field position |
| PatrolRoute | `{ posts: Vec<Vec2>, current: usize }` | Guard's ordered patrol posts |
| HasTarget | marker | NPC has an active movement target |
| InCombat | marker | Blocks behavior transitions |
| Stealer | marker | NPC steals from farms (enables steal systems) |
| FleeThreshold | `{ pct: f32 }` | Flee combat below this HP % |
| LeashRange | `{ distance: f32 }` | Disengage combat if this far from home |
| WoundedThreshold | `{ pct: f32 }` | Drop everything and go home below this HP % |

## Systems

### handle_arrival_system
- Reads `ArrivalMsg` events (from GPU arrival detection)
- Transitions based on current state:
  - `Patrolling` → remove `Patrolling`, add `OnDuty { ticks: 0 }`, remove `HasTarget`
  - `GoingToRest` → remove `GoingToRest`, add `Resting`, remove `HasTarget`
  - `GoingToWork` → remove `GoingToWork`, add `Working`, remove `HasTarget`

### energy_system
- All NPCs with Energy (excluding `Resting`): drain `ENERGY_DRAIN_RATE * delta`
- NPCs with `Resting`: recover `ENERGY_RECOVER_RATE * delta`
- Clamp to 0.0-100.0

### tired_system
- Query: Energy < 50, not `Resting`, not `GoingToRest`, not `InCombat`, `Home.is_valid()`
- Remove current state (`OnDuty`, `Working`, `Patrolling`, `GoingToWork`)
- Add `GoingToRest`
- Set target to `Home` position via `GpuUpdate::SetTarget`
- Add `HasTarget`

### resume_patrol_system
- Query: `Resting` guards with Energy >= 80
- Remove `Resting`, add `Patrolling`
- Set target to current patrol post via `GpuUpdate::SetTarget`
- Add `HasTarget`

### resume_work_system
- Query: `Resting` farmers with Energy >= 80
- Remove `Resting`, add `GoingToWork`
- Set target to `WorkPosition` via `GpuUpdate::SetTarget`
- Add `HasTarget`

### patrol_system
- Query: `OnDuty` guards
- Increment `ticks` each frame
- After 60 ticks: advance `PatrolRoute.current` (wrapping), remove `OnDuty`, add `Patrolling`
- Set target to next post via `GpuUpdate::SetTarget`
- Add `HasTarget`

### steal_arrival_system
- Reads `ArrivalMsg` for NPCs with `Stealer`
- `Raiding` arrival (at farm): add `CarryingFood`, remove `Raiding`, add `Returning`, set color yellow, target home
- `Returning` arrival (at camp): if `CarryingFood` { remove, deliver food to `FOOD_STORAGE`, push `FoodDelivered`, reset color to red }. NPC has no active state → falls through to `steal_decision_system` next tick.

### steal_decision_system
- Query: `Stealer` NPCs with no active state (no `Raiding`, `Returning`, `Resting`, `InCombat`, `Recovering`, `GoingToRest`, `Dead`)
- Priority 1: Health < `WoundedThreshold` (+ valid home) → drop food, add `Returning`, target home
- Priority 2: Has `CarryingFood` (+ valid home) → add `Returning`, target home
- Priority 3: Energy < 50 (+ valid home) → add `Returning`, target home
- Priority 4: Find nearest farm from `WORLD_DATA` (reads position from `GPU_READ_STATE`), add `Raiding`, target farm

### flee_system
- Query: `InCombat` + `FleeThreshold` + `Home`
- If health < `FleeThreshold.pct`: remove `InCombat`, drop `CarryingFood` if present, add `Returning`, target home

### leash_system
- Query: `InCombat` + `LeashRange` + `Home`
- Read position from `GPU_READ_STATE`
- If distance to home > `LeashRange.distance`: remove `InCombat`, add `Returning`, target home

### wounded_rest_system
- On `ArrivalMsg` for NPCs with `WoundedThreshold`
- If health < `WoundedThreshold.pct`: add `Recovering { threshold: 0.75 }` + `Resting`

### recovery_system
- Query: `Recovering` + `Resting` + `Health`
- If health >= `Recovering.threshold`: remove both, NPC re-enters decision system next tick

### economy_tick_system
- Reads `Res<PhysicsDelta>` (godot-bevy's Godot-synced delta time)
- Accumulates elapsed time, triggers on hour boundaries
- **Food production**: counts `Working` farmers per `Clan`, adds food to `FOOD_STORAGE`
- **Respawn check**: compares `PopulationStats` alive counts vs `GameConfig` caps, spawns replacements

## Energy Model

| Constant | Value | Purpose |
|----------|-------|---------|
| ENERGY_DRAIN_RATE | 0.02/tick | Drain while active |
| ENERGY_RECOVER_RATE | 0.2/tick | Recovery while resting (10x drain) |
| Tired threshold | 50 | Below this, NPC goes to rest |
| Rested threshold | 80 | Above this, NPC returns to duty |

The 50-80 hysteresis band prevents oscillation. An NPC stops working at 50 and doesn't resume until 80.

At 60fps: ~2500 ticks (42s) from 100 to 50. ~150 ticks (2.5s) resting from 50 to 80. Then ~1500 ticks (25s) from 80 back to 50.

## Patrol Cycle

Guards have a `PatrolRoute` with ordered posts (built from WorldData at spawn). The cycle:

1. Spawn → walk to post 0 (`Patrolling`)
2. Arrive → stand at post (`OnDuty`, ticks counting)
3. 60 ticks → advance to post 1 (`Patrolling`)
4. Arrive → `OnDuty` again
5. After last post, wrap to post 0

Each town has 4 guard posts at corners. Guards cycle clockwise.

## Known Issues / Limitations

- **InCombat is sticky**: If a target dies out of detection range, the NPC may stay `InCombat` until attack_system clears it. No timeout.
- **No priority system**: A farmer at 51% energy ignores a raid happening nearby. Combat only engages via GPU targeting, not behavior decisions.
- **Fixed patrol timing**: 60 ticks at every post, regardless of threat level or distance.
- **No pathfinding**: NPCs walk in a straight line to target. They rely on separation physics to avoid each other, but can't navigate around buildings.
- **Energy doesn't affect combat**: A nearly exhausted guard fights at full strength.
- **Linear arrival scan**: handle_arrival_system and steal_arrival_system iterate all entities per arrival event — O(events * entities). A HashMap lookup would be more efficient at scale.
- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).
- **Single camp index hardcoded**: steal_arrival_system uses `camp_food[0]` — multi-camp food delivery needs camp_idx from a component.
- **No HP regen in Bevy**: recovery_system checks health threshold but there's no Bevy system that regenerates HP over time. Recovery currently depends on external healing.
- **All raiders target same farm**: steal_decision_system picks nearest farm per raider. If all raiders spawn at the same camp, they all converge on the same farm.

## Rating: 8/10

Full behavior state machine with guard patrol, farmer work, and raider steal/flee/recover cycles. All systems are component-driven (not job-specific) — any NPC given `Stealer` + `FleeThreshold` would behave as a raider. Energy hysteresis prevents oscillation. Combat escape (flee + leash) and recovery are generic. Main gaps: single-camp hardcoding, no HP regen system, linear arrival scans.
