# Behavior System

## Overview

Behavior systems manage NPC lifecycle outside of combat: energy drain/recovery, rest transitions, patrol cycling for guards, work transitions for farmers, and stealing/fleeing for raiders. All run in `Step::Behavior` after combat is resolved.

**Unified Decision System**: All NPC decisions are handled by ONE system (`decision_system`) using a priority cascade. This replaced 5 separate systems (flee, leash, patrol, recovery, old decision) to avoid scattered decision-making and Bevy command sync race conditions.

Priority cascade (first match wins):
1. InCombat + should_flee? → Flee
2. InCombat + should_leash? → Leash
3. InCombat → Skip (attack_system handles)
4. Recovering + healed? → Resume
5. Working + tired? → Stop work
6. OnDuty + time_to_patrol? → Patrol
7. Resting + rested? → Wake up
8. Raiding (post-combat) → Re-target farm
9. Idle → Score Eat/Rest/Work/Wander

All checks are **component-driven, not job-driven**. Flee logic operates on any NPC with `FleeThreshold` + `InCombat`, regardless of whether it's a guard or raider.

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
| Eat | `(100 - energy) * 1.5` | town has food in storage |
| Rest | `100 - energy` | home valid |
| Work | `40.0 * hp_mult` | has job, HP > 50% |
| Wander | `10.0` | always |

**Eat action**: Instantly consumes 1 food from town storage and restores `ENERGY_FROM_EATING` (30) energy. No travel required — NPCs eat at current location.

**HP-based work score**: `hp_mult = 0` if HP < 50%, otherwise `(hp_pct - 0.5) * 2`. This prevents wounded NPCs (especially raiders) from working/raiding when they should rest.

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
| AssignedFarm | `usize` | Farm index farmer is working at (for occupancy tracking) |
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

### decision_system (Unified Priority Cascade)
- Query: NPCs not in transit states (no Patrolling, GoingToWork, GoingToRest, Returning, Wandering, Dead)
- Checks states via `Option<&Component>` and handles in priority order:

**Priority 1-3: Combat decisions**
- If `InCombat` + has `FleeThreshold`: dynamic threat assessment (enemies vs allies within 200px, throttled every 30 frames), flee if HP < effective threshold
- If `InCombat` + has `LeashRange`: check distance from `CombatOrigin`, disengage if > leash distance
- If `InCombat`: skip (attack_system handles targeting)

**Priority 4: Recovery**
- If `Recovering` + HP >= threshold: remove `Recovering` and `Resting`

**Priority 5: Tired workers**
- If `Working` + energy < `ENERGY_TIRED_THRESHOLD` (30%): remove `Working`, release `AssignedFarm`

**Priority 6: Patrol**
- If `OnDuty` + ticks >= `GUARD_PATROL_WAIT` (60): advance `PatrolRoute`, transition to `Patrolling`

**Priority 7: Wake up**
- If `Resting` + energy >= `ENERGY_WAKE_THRESHOLD` (90%): remove `Resting`, proceed to idle scoring

**Priority 8: Raid continuation**
- If `Raiding` (post-combat): re-target nearest farm

**Priority 9: Idle scoring (Utility AI)**
- Score Eat/Rest/Work/Wander with personality multipliers and HP modifier
- Select via weighted random, execute action
- **Food check**: Eat only scored if town has food in storage
- **Decision logging**: Each decision logged to `NpcLogCache`

### on_duty_tick_system
- Query: NPCs with `OnDuty` (excluding `InCombat`)
- Increments `ticks_waiting` each frame
- Separated from decision_system to allow mutable `OnDuty` access while main query has immutable view

### arrival_system (Generic)
- Reads `ArrivalMsg` events (from GPU arrival detection) for most states
- **Proximity-based arrival** for Returning and GoingToRest: checks distance to home instead of waiting for exact GPU arrival. Uses DELIVERY_RADIUS (150px, same as healing aura). This fixes raiders and resting NPCs getting stuck when exact arrival doesn't trigger.
- **Food delivery is proximity-only**: Only the proximity check (within 150px of home) delivers food. Event-based Returning arrival just re-targets home — this prevents delivering food at wrong location after combat chase.
- **Working farmer drift check** (throttled every 30 frames): re-targets farmers who drifted >20px from their assigned farm
- **Farm lookup uses WorkPosition**: When farmer arrives at work, finds farm near their `WorkPosition` (not current position) to avoid "no farm" errors when pushed by separation forces
- Transitions based on current state marker (component-driven, not job-driven):
  - `Patrolling` → `OnDuty { ticks: 0 }`
  - `GoingToRest` → `Resting` (via proximity check)
  - `GoingToWork` → `Working` + `AssignedFarm` + reserve farm + **harvest if farm ready** (see Farm Growth below)
  - `Raiding` → `CarryingFood` + `Returning` **only if farm is Ready** (see Farm Growth below)
  - `Returning` event arrival → re-target home (actual delivery via proximity check)
  - `Wandering` → clear state (back to decision_system)
- Also checks `WoundedThreshold` for recovery mode on arrival
- **State logging**: All transitions are logged to `NpcLogCache` (e.g., "→ OnDuty", "→ Resting", "Stole food → Returning")

### energy_system
- NPCs with `Resting`: recover `ENERGY_RECOVER_RATE` per tick
- NPCs without `Resting`: drain `ENERGY_DRAIN_RATE` per tick
- Clamp to 0.0-100.0
- **Note**: All state transitions (wake-up, stop working) are handled in decision_system to keep decisions centralized and avoid Bevy command sync races

### healing_system
- Query: NPCs with `Health`, `MaxHealth`, `Faction`, `TownId` (without `Dead`)
- Reads NPC position from `GpuReadState`, town centers from `WorldData`
- All settlements (villager and raider) are Town entries with faction (unified town model)
- If NPC within `HEAL_RADIUS` (150px) of same-faction town center: heal `HEAL_RATE` (5 HP/sec)
- Adds/removes `Healing` marker component for visual feedback
- Sends `GpuUpdate::SetHealing` for shader halo effect
- Debug: `get_health_debug()` returns healing_in_zone_count and healing_healed_count

### economy_tick_system
- Reads `Res<PhysicsDelta>` (godot-bevy's Godot-synced delta time)
- Accumulates elapsed time, triggers on hour boundaries
- **Food production**: counts `Working` farmers per `TownId`, adds food to `FOOD_STORAGE`
- **Respawn check**: (disabled) code exists to compare `PopulationStats` vs `GameConfig` caps and spawn replacements

### farm_growth_system
- Runs every frame, advances farm growth based on elapsed game time
- **FarmStates resource**: tracks `Growing` vs `Ready` state and progress (0.0-1.0) per farm
- **Hybrid growth model**:
  - Passive: `FARM_BASE_GROWTH_RATE` (0.08/hour) — ~12 game hours to full growth
  - Tended: `FARM_TENDED_GROWTH_RATE` (0.25/hour) — ~4 game hours with farmer working
- Farm transitions to `Ready` when progress >= 1.0
- Ready farms show yellow food icon (same color as carried food)

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

**Farmer harvest** (arrival_system, GoingToWork → Working):
- Uses `find_location_within_radius()` to find farm within FARM_ARRIVAL_RADIUS (20px) of **WorkPosition** (not current position)
- If farm is Ready: add food to town storage, reset farm to Growing
- Logs "Harvested → Working" vs "→ Working (tending)"

**Raider steal** (arrival_system, Raiding):
- Uses `find_location_within_radius()` to find farm within FARM_ARRIVAL_RADIUS
- Only steals if farm is Ready — reset farm to Growing, set CarryingFood + Returning
- If farm not ready: re-target another farm via `find_nearest_location()`
- Logs "Stole food → Returning" vs "Farm not ready, seeking another"

**Visual feedback** (gpu.rs, build_item_multimesh):
- Item MultiMesh renders food icons on ready farms (same yellow/gold as carried food)
- Farm slots allocated after NPC slots (MAX_NPC_COUNT + MAX_FARMS)

## Energy Model

| Constant | Value | Purpose |
|----------|-------|---------|
| ENERGY_DRAIN_RATE | 0.02/tick | Drain while active |
| ENERGY_RECOVER_RATE | 0.2/tick | Recovery while resting (10x drain) |
| ENERGY_WAKE_THRESHOLD | 90.0 | Wake from Resting when energy reaches this |
| ENERGY_TIRED_THRESHOLD | 30.0 | Stop working and seek rest below this |
| ENERGY_FROM_EATING | 30.0 | Energy restored per food consumed |

With utility AI, there are no fixed thresholds for decisions. Low energy increases Rest/Eat scores, but NPCs might still choose Work if their Focused trait outweighs tiredness. NPCs automatically wake from `Resting` at 90% energy (handled in decision_system for proper command sync).

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
- **~~Farm data fetch performance~~**: Fixed. BevyApp reference is now cached in ready(), eliminating scene tree traversal every frame.

## Rating: 8/10

**Unified decision system** consolidates all NPC decisions into one place with clear priority cascade. Previously scattered across 5 systems (flee, leash, patrol, recovery, decision), now all handled in `decision_system`. This eliminates confusion about where decisions happen and avoids Bevy command sync race conditions.

Utility AI for idle decisions is sound. Farm growth system adds meaningful gameplay loop. Still has gaps: no pathfinding, InCombat sticks forever, all raiders converge on same farm, healing halo visual broken. But architecture is now clean and maintainable.
