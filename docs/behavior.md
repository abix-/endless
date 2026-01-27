# Behavior System

## Overview

Behavior systems manage NPC lifecycle outside of combat: energy drain/recovery, rest transitions, patrol cycling for guards, and work transitions for farmers. All run in `Step::Behavior` after combat is resolved.

## State Machine

```
                    ┌──────────────┐
         spawn ───▶│  GoingToWork  │ (farmers)
                    │  Patrolling   │ (guards)
                    └──────┬───────┘
                           │ arrival
                           ▼
                    ┌──────────────┐
                    │   Working    │ (farmers)
                    │   OnDuty     │ (guards: 60 ticks then next post)
                    └──────┬───────┘
                           │ energy < 50
                           ▼
                    ┌──────────────┐
                    │ GoingToRest  │
                    │ (walk home)  │
                    └──────┬───────┘
                           │ arrival
                           ▼
                    ┌──────────────┐
                    │   Resting    │
                    │ (recover)    │
                    └──────┬───────┘
                           │ energy >= 80
                           ▼
                    back to Patrolling / GoingToWork

    At any point:
    ┌──────────────┐
    │   InCombat   │ ── blocks all behavior transitions
    └──────────────┘    (set by attack_system, cleared when no target)
```

## Components

| Component | Type | Purpose |
|-----------|------|---------|
| Energy | `f32` | 0-100, drains while active, recovers while resting |
| Resting | marker | NPC is at home, recovering energy |
| GoingToRest | marker | NPC is walking home to rest |
| Patrolling | marker | Guard is walking to next patrol post |
| OnDuty | `{ ticks: u32 }` | Guard is stationed at a post |
| Working | marker | Farmer is at work position |
| GoingToWork | marker | Farmer is walking to work |
| Home | `{ x, y }` | NPC's home/bed position |
| WorkPosition | `{ x, y }` | Farmer's field position |
| PatrolRoute | `{ posts: Vec<Vec2>, current: usize }` | Guard's ordered patrol posts |
| HasTarget | marker | NPC has an active movement target |
| InCombat | marker | Blocks behavior transitions |

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
- Query: Energy < 50, not `Resting`, not `GoingToRest`, not `InCombat`
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

## Energy Model

| Constant | Value | Purpose |
|----------|-------|---------|
| ENERGY_DRAIN_RATE | 0.1 | Per-second drain while active |
| ENERGY_RECOVER_RATE | 0.2 | Per-second recovery while resting |
| Tired threshold | 50 | Below this, NPC goes to rest |
| Rested threshold | 80 | Above this, NPC returns to duty |

At drain rate 0.1/s: ~500 seconds (8.3 min) from full to tired threshold.
At recover rate 0.2/s: ~150 seconds (2.5 min) from 50 to 80.

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

## Rating: 7/10

Functional state machine with clean transitions. Energy model creates natural work/rest cycles. Main gaps: no priority/urgency system, no pathfinding, InCombat can get stuck. These are design decisions more than bugs — the system works as intended, it just needs more sophistication for deeper gameplay.
