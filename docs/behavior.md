# Behavior System

## Overview

Behavior systems manage NPC lifecycle outside of combat: energy drain/recovery, rest transitions, patrol cycling for guards, work transitions for farmers, and stealing/fleeing for raiders. All run in `Step::Behavior` after combat is resolved.

All systems are **component-driven, not job-driven**. A system like `flee_system` operates on any NPC with `FleeThreshold` + `InCombat`, regardless of whether it's a guard or raider.

## Utility AI (Weighted Random Decisions)

NPCs use utility AI for idle decisions. Instead of priority cascades (if tired→rest, else work), actions are scored and selected via weighted random.

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
| Eat | `(100 - energy) * 1.5` | food available |
| Rest | `100 - energy` | home valid |
| Work | `40.0` | has job |
| Wander | `10.0` | always |

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

```
    Guard:                Farmer:               Stealer (Raider):
    ┌──────────┐         ┌──────────┐          ┌──────────┐
    │Patrolling│         │GoingToWork│         │  (idle)  │ spawns stateless
    └────┬─────┘         └────┬─────┘          └────┬─────┘
         │ arrival            │ arrival              │ npc_decision_system
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
                  │ npc_decision_system             ▼
                  ▼ (weighted random)          deliver food, re-enter
             ┌──────────┐                     npc_decision_system
             │GoingToRest│
             └────┬─────┘
                  │ arrival
                  ▼
             ┌──────────┐
             │ Resting  │
             └────┬─────┘
                  │ npc_decision_system
                  ▼ (weighted random)
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
| TownId | `i32` | Town/camp identifier — every NPC belongs to one |
| Energy | `f32` | 0-100, drains while active, recovers while resting |
| Personality | `{ trait1, trait2 }` | 0-2 traits with magnitude affecting stats and decisions |
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

### npc_decision_system (Utility AI)
- Query: NPCs without active state (no Patrolling, OnDuty, Working, GoingToWork, Resting, GoingToRest, Raiding, Returning, InCombat, Recovering, Dead)
- Score actions: Eat, Rest, Work, Wander (with personality multipliers)
- Select via weighted random
- Execute: set state marker, push GPU target
- Replaces: tired_system, resume_patrol_system, resume_work_system, raider_idle_system

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

### patrol_system
- Query: `OnDuty` guards
- Increment `ticks` each frame
- After 60 ticks: advance `PatrolRoute.current` (wrapping), remove `OnDuty`, add `Patrolling`
- Set target to next post via `GpuUpdate::SetTarget`
- Add `HasTarget`

### raider_arrival_system
- Reads `ArrivalMsg` for NPCs with `Stealer`
- `Raiding` arrival (at farm): add `CarryingFood`, remove `Raiding`, add `Returning`, set color yellow, target home
- `Returning` arrival (at camp): if `CarryingFood` { remove, deliver food to `FOOD_STORAGE`, push `FoodDelivered`, reset color to red }. NPC has no active state → falls through to `npc_decision_system` next tick.

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
- **Food production**: counts `Working` farmers per `TownId`, adds food to `FOOD_STORAGE`
- **Respawn check**: compares `PopulationStats` alive counts vs `GameConfig` caps, spawns replacements

## Energy Model

| Constant | Value | Purpose |
|----------|-------|---------|
| ENERGY_DRAIN_RATE | 0.02/tick | Drain while active |
| ENERGY_RECOVER_RATE | 0.2/tick | Recovery while resting (10x drain) |

With utility AI, there are no fixed thresholds. Low energy increases Rest/Eat scores, but NPCs might still choose Work if their Focused trait outweighs tiredness.

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
- **No pathfinding**: NPCs walk in a straight line to target. They rely on separation physics to avoid each other, but can't navigate around buildings.
- **Linear arrival scan**: handle_arrival_system and raider_arrival_system iterate all entities per arrival event — O(events * entities). A HashMap lookup would be more efficient at scale.
- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).
- **Single camp index hardcoded**: raider_arrival_system uses `camp_food[0]` — multi-camp food delivery needs camp_idx from a component.
- **No HP regen in Bevy**: recovery_system checks health threshold but there's no Bevy system that regenerates HP over time. Recovery currently depends on external healing.
- **All raiders target same farm**: npc_decision_system picks nearest farm per raider. If all raiders spawn at the same camp, they all converge on the same farm.
- **Deterministic pseudo-random**: npc_decision_system uses slot index as random seed, so same NPC makes same choices each run.

## Rating: 8/10

Utility AI with weighted random decisions replaces deterministic priority cascades. Personality traits (Brave, Tough, Swift, Focused) affect both stats and decision weights. Same situation can produce different outcomes based on trait magnitudes and random roll. Combat escape (flee + leash) and recovery remain deterministic for reliability. Main gaps: deterministic pseudo-random (not true random), linear arrival scans, single-camp hardcoding.
