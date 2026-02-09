# Behavior System

## Overview

NPC decision-making and state transitions. All run in `Step::Behavior` after combat is resolved. For economy systems (farm growth, starvation, camp foraging, raider respawning, game time), see [economy.md](economy.md).

**Unified Decision System**: All NPC decisions are handled by `decision_system` using a priority cascade. The system uses **SystemParam bundles** to organize related parameters and stay within Bevy's 16-parameter limit:

- `FarmParams`: farm states, occupancy tracking, world data
- `EconomyParams`: food storage, food events, population stats
- `CombatParams`: flee data, leash data queries
- `NpcStateParams`: transit state queries, work positions, assigned farms, patrol routes

Priority order (first match wins):
0. AtDestination → Handle arrival transitions
1. InCombat + should_flee? → Flee
2. InCombat + should_leash? → Leash
3. InCombat → Skip (attack_system handles)
4. Recovering + healed? → Resume
5. Working + tired? → Stop work
6. OnDuty + time_to_patrol? → Patrol
7. Resting + rested? → Wake up
8. Idle → Score Eat/Rest/Work/Wander

All checks are **component-driven, not job-driven**. Flee logic operates on any NPC with `FleeThreshold` + `InCombat`, regardless of whether it's a guard or raider.

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
         │ decision_system    ▼                      ▼
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
| GoingToRest | marker | NPC is walking home to rest (`#[require(HasTarget)]`) |
| Patrolling | marker | Guard is walking to next patrol post (`#[require(HasTarget)]`) |
| OnDuty | `{ ticks: u32 }` | Guard is stationed at a post |
| Working | marker | Farmer is at work position |
| AssignedFarm | `Vec2` | Farm position farmer is working at (for occupancy tracking) |
| GoingToWork | marker | Farmer is walking to work (`#[require(HasTarget)]`) |
| Raiding | marker | NPC is walking to a farm to steal (`#[require(HasTarget)]`) |
| Returning | marker | NPC is walking back to home base (`#[require(HasTarget)]`) |
| CarryingFood | marker | NPC has stolen food |
| *(removed)* | | CarriedItem replaced by `SetEquipSprite(Item, ...)` for visual rendering |
| Recovering | `{ threshold: f32 }` | NPC is resting until HP >= threshold |
| Wandering | marker | NPC is walking to a random nearby position (`#[require(HasTarget)]`) |
| Starving | marker | NPC hasn't eaten in 24+ hours (50% HP cap, 75% speed) |
| LastAteHour | `i32` | Game hour when NPC last ate (for starvation tracking) |
| Healing | marker | NPC is inside healing aura (visual feedback) |
| MaxHealth | `f32` | NPC's maximum health (for healing cap) |
| Home | `{ x, y }` | NPC's home/bed position |
| WorkPosition | `{ x, y }` | Farmer's field position |
| PatrolRoute | `{ posts: Vec<Vec2>, current: usize }` | Guard's ordered patrol posts |
| AtDestination | marker | NPC arrived at destination, needs transition handling |
| HasTarget | marker | NPC has an active movement target (auto-inserted via `#[require]` on transit components) |
| InCombat | marker | Blocks behavior transitions |
| CombatOrigin | `{ x, y }` | Position where combat started; leash measures from here |
| Stealer | marker | NPC steals from farms (enables steal systems) |
| FleeThreshold | `{ pct: f32 }` | Flee combat below this HP % |
| LeashRange | `{ distance: f32 }` | Disengage combat if chased this far from combat origin |
| WoundedThreshold | `{ pct: f32 }` | Drop everything and go home below this HP % |

## Systems

### decision_system (Unified Priority Cascade)
- Query: NPCs not in transit states (no Patrolling, GoingToWork, GoingToRest, Returning, Wandering, Dead)
- Uses **SystemParam bundles** to organize 19+ parameters into 4 logical groups (see Overview)
- Checks states via `Option<&Component>` and handles in priority order:

**Priority 0: Arrival transitions**
- If `AtDestination`: handle state-specific transitions
  - `Patrolling` → `OnDuty { ticks: 0 }`
  - `GoingToRest` → `Resting`
  - `GoingToWork` → `Working` + `AssignedFarm` + harvest if farm ready
  - `Raiding` → steal if farm ready, else re-target another farm; `CarryingFood` + `Returning`
  - `Wandering` → clear state
  - Check `WoundedThreshold` for recovery mode
- Removes `AtDestination` after handling

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

**Priority 8: Idle scoring (Utility AI)**
- Score Eat/Rest/Work/Wander with personality multipliers and HP modifier
- Select via weighted random, execute action
- **Food check**: Eat only scored if town has food in storage
- **Decision logging**: Each decision logged to `NpcLogCache`

### on_duty_tick_system
- Query: NPCs with `OnDuty` (excluding `InCombat`)
- Increments `ticks_waiting` each frame
- Separated from decision_system to allow mutable `OnDuty` access while main query has immutable view

### arrival_system (Proximity Checks)
- **Proximity-based delivery** for Returning raiders: checks distance to home, delivers food within DELIVERY_RADIUS (150px)
- **Working farmer drift check** (throttled every 30 frames): re-targets farmers who drifted >20px from their assigned farm
- Arrival detection (`HasTarget` → `AtDestination`) is handled by `gpu_position_readback` in movement.rs
- All state transitions handled by decision_system Priority 0 (central brain model)

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
- **Starvation HP cap**: Starving NPCs have HP capped at 50% of MaxHealth (can't heal above this)
- Adds/removes `Healing` marker component for visual feedback
- Sends `GpuUpdate::SetHealing` for shader halo effect
- Debug: `get_health_debug()` returns healing_in_zone_count and healing_healed_count

*Economy systems (game_time, farm_growth, camp_forage, raider_respawn, starvation) documented in [economy.md](economy.md).*

*Farm growth, starvation, and group raid systems documented in [economy.md](economy.md).*

## Energy Model

Energy uses game time (respects time_scale and pause):

| Constant | Value | Purpose |
|----------|-------|---------|
| ENERGY_RECOVER_PER_HOUR | 100/6 (~16.7) | Recovery while resting (6 hours to full) |
| ENERGY_DRAIN_PER_HOUR | 100/24 (~4.2) | Drain while active (24 hours to empty) |
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

Each town has 4 guard posts at corners. Guards cycle clockwise. Patrol routes are rebuilt when guard posts are added or removed via `add_location()`/`remove_location()`.

## Known Issues / Limitations

- **No pathfinding**: NPCs walk in a straight line to target. They rely on separation physics to avoid each other, but can't navigate around buildings.
- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).
- **Healing halo visual not working**: healing_system heals NPCs but the shader halo effect isn't rendering correctly yet.
- **Deterministic pseudo-random**: decision_system uses slot index as random seed, so same NPC makes same choices each run.

## Rating: 8/10

Central brain architecture: `decision_system` handles all NPC decisions with clear priority cascade. SystemParam bundles organize parameters into logical groups, allowing the system to scale as more features are added. Utility AI for idle decisions creates lifelike behavior.

Gaps: no pathfinding, healing halo visual broken, deterministic pseudo-random.
