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
| Work | `40.0 * hp_mult` | has job, HP > 50% |
| Wander | `10.0` | always |

**HP-based work score**: `hp_mult = 0` if HP < 50%, otherwise `(hp_pct - 0.5) * 2`. This prevents wounded NPCs (especially raiders) from working/raiding when they should rest.

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
    │  OnDuty  │ spawn   │GoingToWork│ spawn   │  (idle)  │ spawns stateless
    │ 60 ticks │         └────┬─────┘          └────┬─────┘
    └────┬─────┘              │ arrival              │ decision_system
         │ patrol_system      ▼                      ▼
         ▼                ┌──────────┐          ┌──────────┐
    ┌──────────┐         │ Working  │          │  Raiding │ (walk to farm)
    │Patrolling│         └────┬─────┘          └────┬─────┘
    └────┬─────┘              │                     │ arrival at farm
         │ arrival            │                     ▼
         ▼                    │                ┌──────────┐
    ┌──────────┐              │                │Returning │ (+CarryingFood)
    │  OnDuty  │              │                │(to camp) │
    │ 60 ticks │              │                └────┬─────┘
    └────┬─────┘              │                     │ arrival at camp
         └────────┬───────────┘                     ▼
                  │ decision_system            deliver food, re-enter
                  ▼ (weighted random)          decision_system
             ┌──────────┐
             │GoingToRest│
             └────┬─────┘
                  │ arrival
                  ▼
             ┌──────────┐
             │ Resting  │
             └────┬─────┘
                  │ decision_system
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
| CarriedItem | `u8` | Item NPC is carrying (0=none, 1=food). Rendered as separate MultiMesh layer above head. |
| Recovering | `{ threshold: f32 }` | NPC is resting until HP >= threshold |
| Wandering | marker | NPC is walking to a random nearby position |
| Healing | marker | NPC is inside healing aura (visual feedback) |
| MaxHealth | `f32` | NPC's maximum health (for healing cap) |
| Home | `{ x, y }` | NPC's home/bed position |
| WorkPosition | `{ x, y }` | Farmer's field position |
| PatrolRoute | `{ posts: Vec<Vec2>, current: usize }` | Guard's ordered patrol posts |
| HasTarget | marker | NPC has an active movement target |
| InCombat | marker | Blocks behavior transitions |
| CombatOrigin | `{ x, y }` | Position where combat started; leash measures from here |
| Stealer | marker | NPC steals from farms (enables steal systems) |
| FleeThreshold | `{ pct: f32 }` | Flee combat below this HP % |
| LeashRange | `{ distance: f32 }` | Disengage combat if chased this far from combat origin |
| WoundedThreshold | `{ pct: f32 }` | Drop everything and go home below this HP % |

## Systems

### decision_system (Utility AI)
- Query: NPCs without active state (no Patrolling, OnDuty, Working, GoingToWork, Resting, GoingToRest, Returning, Wandering, InCombat, Recovering, Dead), OR raiders with `Raiding` needing re-target
- **Raid continuation**: If raider has `Raiding` marker (e.g., after combat ends), skip scoring and re-target nearest farm via `find_nearest_location`
- Score actions: Eat, Rest, Work, Wander (with personality multipliers and HP modifier)
- Select via weighted random
- Execute: set state marker, push GPU target
- **Decision logging**: Each decision is logged to `NpcLogCache` with timestamp and format `"{action} (e:{energy} h:{health})"`

### arrival_system (Generic)
- Reads `ArrivalMsg` events (from GPU arrival detection) for most states
- **Proximity-based arrival** for Returning and GoingToRest: checks distance to home instead of waiting for exact GPU arrival. Uses DELIVERY_RADIUS (150px, same as healing aura). This fixes raiders and resting NPCs getting stuck when exact arrival doesn't trigger.
- Transitions based on current state marker (component-driven, not job-driven):
  - `Patrolling` → `OnDuty { ticks: 0 }`
  - `GoingToRest` → `Resting` (via proximity check)
  - `GoingToWork` → `Working`
  - `Raiding` → `CarryingFood` + `Returning` (if near farm)
  - `Returning` → deliver food if carrying, clear state (via proximity check)
  - `Wandering` → clear state (back to decision_system)
- Also checks `WoundedThreshold` for recovery mode on arrival
- **State logging**: All transitions are logged to `NpcLogCache` (e.g., "→ OnDuty", "→ Resting", "Stole food → Returning")

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

### flee_system
- Query: `InCombat` + `FleeThreshold` + `Home`
- If health < `FleeThreshold.pct`: remove `InCombat`, `CombatOrigin`, `Raiding`, drop `CarryingFood` if present, add `Returning`, target home

### leash_system
- Query: `InCombat` + `LeashRange` + `Home` + `CombatOrigin`
- Read position from `GPU_READ_STATE`
- If distance from **combat origin** > `LeashRange.distance`: remove `InCombat`, `CombatOrigin`, `Raiding`, add `Returning`, target home
- Note: Leash is based on distance from where combat started, not from home. This allows NPCs to travel far for objectives (raids) but prevents chasing enemies forever.

### recovery_system
- Query: `Recovering` + `Health` (Resting is optional)
- If health >= `Recovering.threshold`: remove `Recovering` and `Resting` (if present)
- Fixes: NPCs that lost `Resting` but kept `Recovering` were stuck forever

### healing_system
- Query: NPCs with `Health`, `MaxHealth`, `Faction`, `TownId` (without `Dead`)
- Reads NPC position from `GpuReadState`, town centers from `WorldData`
- If NPC within `HEAL_RADIUS` (150px) of same-faction town center: heal `HEAL_RATE` (5 HP/sec)
- Adds/removes `Healing` marker component for visual feedback
- Sends `GpuUpdate::SetHealing` for shader halo effect

### economy_tick_system
- Reads `Res<PhysicsDelta>` (godot-bevy's Godot-synced delta time)
- Accumulates elapsed time, triggers on hour boundaries
- **Food production**: counts `Working` farmers per `TownId`, adds food to `FOOD_STORAGE`
- **Respawn check**: (disabled) code exists to compare `PopulationStats` vs `GameConfig` caps and spawn replacements

## Energy Model

| Constant | Value | Purpose |
|----------|-------|---------|
| ENERGY_DRAIN_RATE | 0.02/tick | Drain while active |
| ENERGY_RECOVER_RATE | 0.2/tick | Recovery while resting (10x drain) |
| ENERGY_WAKE_THRESHOLD | 90.0 | Wake from Resting when energy reaches this |

With utility AI, there are no fixed thresholds. Low energy increases Rest/Eat scores, but NPCs might still choose Work if their Focused trait outweighs tiredness. NPCs automatically wake from `Resting` at 90% energy.

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
- **Linear arrival scan**: arrival_system iterates all entities per arrival event — O(events * entities). A HashMap lookup would be more efficient at scale.
- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).
- **~~Single camp index hardcoded~~**: Fixed. Raiders now deliver to their own faction's town using TownId component.
- **Healing halo visual not working**: healing_system heals NPCs but the shader halo effect isn't rendering correctly yet.
- **All raiders target same farm**: decision_system picks nearest farm per raider. If all raiders spawn at the same camp, they all converge on the same farm.
- **Deterministic pseudo-random**: decision_system uses slot index as random seed, so same NPC makes same choices each run.

## Rating: 8/10

Utility AI with weighted random decisions replaces deterministic priority cascades. Personality traits (Brave, Tough, Swift, Focused) affect both stats and decision weights. Same situation can produce different outcomes based on trait magnitudes and random roll. Combat escape (flee + leash) and recovery remain deterministic for reliability. Main gaps: deterministic pseudo-random (not true random), linear arrival scans, single-camp hardcoding.
