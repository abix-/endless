# Behavior System

## Overview

NPC decision-making and state transitions. All run in `Step::Behavior` after combat is resolved. Movement targets are submitted via `MovementIntents` resource with priority-based arbitration — `resolve_movement_system` (after Step::Behavior) is the sole emitter of `GpuUpdate::SetTarget`. For economy systems (farm growth, starvation, raider foraging, raider respawning, game time), see [economy.md](economy.md).

**Unified Decision System**: All NPC decisions are handled by `decision_system` using a priority cascade. NPC state is modeled by two orthogonal enum components (concurrent state machines pattern):

- `Activity` enum: what the NPC is *doing* (Idle, Working, OnDuty, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain, Wandering, Raiding, Returning, Mining, MiningAtMine)
- `CombatState` enum: whether the NPC is *fighting* (None, Fighting, Fleeing)

Activity is preserved through combat — a Raiding NPC stays `Activity::Raiding` while `CombatState::Fighting`. When combat ends, the NPC resumes its previous activity.

The system uses **SystemParam bundles** for farm and economy parameters:
- `FarmParams`: `EntityMap` (occupancy tracked via `BuildingInstance.occupants` field)
- `EconomyParams`: food storage, food events, population stats
- `DecisionExtras`: npc logs, combat log, policies, squad state, town upgrades
- `Res<EntityMap>`: sole source of truth for all building instance lookups (farms, waypoints, towns, gold mines)

Priority order (first match wins), with two-cadence top-of-loop bucket gating (see [performance.md](performance.md#bucket-gated-decision-system) for formulas and scaling numbers):

**DirectControl skip** (before all priorities): NPCs with `direct_control` flag skip the entire decision system — no autonomous behavior whatsoever. The system clears `AtDestination` if present to prevent stale arrival flags. DC NPCs may accumulate loot in `Activity::Returning` while fighting (via `dc_no_return` toggle) — the Returning activity is inert while DC is active. When a DC right-click move/attack command is issued (`click_to_select_system` in render.rs), resting NPCs (`GoingToRest`/`Resting`) are woken to `Idle` so they respond to the command instead of sliding while asleep.

**Priority 0 — arrivals** (every bucket tick):
0. AtDestination → Handle arrival transitions (transient one-frame flag)
-- Farmer en-route retarget (GoingToWork + target farm occupied by other → `find_farmer_farm_target()` local search with claim/release lifecycle, or idle) --
-- Transit skip (`activity.is_transit()` → continue, with GoingToHeal proximity check) --

**Priority 1-3 — combat decisions** (every bucket tick, fighting NPCs on COMBAT_BUCKET cadence):
1. CombatState::Fighting + should_flee? → Flee
2. CombatState::Fighting + should_leash? → Leash
3. CombatState::Fighting → Skip (attack_system handles)

**Priority 4-7 — idle/work decisions** (every bucket tick):
4a. HealingAtFountain + HP >= threshold → Wake (HP-only check)
4b. Resting + energy >= 90% → Wake (energy-only check)
5. Working + tired? → Stop work
6. OnDuty + time_to_patrol? → Patrol
7. Idle → Score Eat/Rest/Work/Wander (wounded → fountain, tired → home)

All checks are **policy-driven per town**. Flee thresholds come from `TownPolicies` resource (indexed by `TownId`), not per-entity `FleeThreshold` components. Raiders use a hardcoded 0.50 threshold. `archer_aggressive` and `farmer_fight_back` policies disable flee entirely for their respective jobs.

## Utility AI (Weighted Random Decisions)

**Idle NPCs** use utility AI for decisions. Instead of rigid rules (if tired→rest, else work), actions are scored and selected via weighted random. This creates lifelike, emergent behavior — a tired farmer with the Efficient trait might still choose Work over Rest sometimes.

The priority cascade (flee > leash > recovery > tired > patrol > wake > raid) handles **state checks** — deterministic "what state am I in" logic. Utility AI only kicks in at the end for NPCs with no active state.

### Personality Component

Each NPC has a `Personality` with 0-2 spectrum traits. Each trait is an axis with signed magnitude (±0.5 to ±1.5); sign determines the pole (e.g. +Brave/-Coward):

| Axis | + / - | Stat Effect | Behavior Effect |
|------|-------|-------------|-----------------|
| Courage | Brave / Coward | — | +: never flees / -: flee threshold +20%×\|m\| |
| Diligence | Efficient / Lazy | +25%×m yield, -25%×m cooldown | +: work↑ / -: work↓ wander↑ |
| Vitality | Hardy / Frail | +25%×m HP | +: rest↓ eat↓ / -: rest↑ eat↑ |
| Power | Strong / Weak | +25%×m damage | +: fight↑ / -: fight↓ |
| Agility | Swift / Slow | +25%×m speed | +: wander↑ / -: wander↓ |
| Precision | Sharpshot / Myopic | +25%×m range | — |
| Ferocity | Berserker / Timid | +50%×m damage when <50% HP | +: fight↑ flee↓ / -: fight↓ flee↑ |

### Action Scoring

| Action | Base Score | Condition |
|--------|-----------|-----------|
| Eat | `(ENERGY_EAT_THRESHOLD - energy) * 1.5` | town has food AND energy < 10 |
| Rest | `(ENERGY_HUNGRY - energy) * 1.0` | home valid AND energy < ENERGY_HUNGRY |
| Work | `40.0 * hp_mult * energy_factor` | has job, HP > 30% |
| Wander | `10.0` | always |

**Eat action**: Instantly consumes 1 food from town storage and restores energy to 100. No travel required — NPCs eat at current location. Only available as emergency option when energy < `ENERGY_EAT_THRESHOLD` (10) — NPCs prefer resting over eating.

**HP-based work score**: `hp_mult = 0` if HP < 30%, otherwise `(hp_pct - 0.3) / 0.7`. This prevents critically wounded NPCs from working/raiding while still allowing starving NPCs (HP capped at 50%) to join raid queues at reduced priority.

**Energy-based work score**: `energy_factor = energy / ENERGY_TIRED_THRESHOLD` when energy < 30, otherwise 1.0. This scales work desire down linearly as energy drops, so rest naturally outcompetes work around energy ~24. Prevents the starvation death spiral where NPCs repeatedly choose work over rest, burn energy in farm-retarget loops, and hit energy 0.

**Note**: The code defines `Action::Fight` and `Action::Flee` in the enum, but these are not scored in decision_system. Fight/flee behavior is handled by combat systems (attack_system, flee_system) instead.

Scores are multiplied by personality multipliers, then weighted random selects an action:

```
Example: Energy=40, Hardy(1.0) + Efficient(1.0)
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
    Archer:               Farmer:               Miner:                Stealer (Raider):
    ┌──────────┐         ┌──────────┐         ┌──────────┐          ┌──────────┐
    │  OnDuty  │ spawn   │GoingToWork│ spawn  │  Idle    │ spawn   │  Idle    │ spawns idle
    │{ticks: 0}│         └────┬─────┘         └────┬─────┘          └────┬─────┘
    └────┬─────┘              │ arrival             │ decision          │ decision_system
         │ decision_system    ▼                     ▼                   ▼
         ▼                ┌──────────┐         ┌──────────┐       ┌──────────────────┐
    ┌──────────┐         │ Working  │         │Mining{pos}│       │ Raiding{target}  │
    │Patrolling│         └────┬─────┘         └────┬─────┘       └────┬─────────────┘
    └────┬─────┘              │ farm Ready         │ arrival          │ arrival at farm
         │ arrival            ▼                     ▼                  ▼
         ▼              ┌──────────────┐       ┌──────────┐       ┌──────────────────┐
    ┌──────────┐        │Returning     │       │MiningAt  │       │Returning{food:T} │
    │  OnDuty  │        │{food: yield} │       │Mine      │       └────┬─────────────┘
    │{ticks: 0}│        └────┬─────────┘       │(4h cycle)│            │ proximity delivery
    └────┬─────┘              │ delivery        └────┬─────┘            ▼
         │                    ▼                     │ full/tired  deliver food, re-enter
         │               GoingToWork           Returning{gold}     decision_system
         └────────┬───────────┴─────────────────────┘
                  │ decision_system
                  ▼ (weighted random)
             ┌──────────┐                  ┌──────────┐
             │GoingToRest│ (tired→home)    │GoingToHeal│ (wounded→fountain)
             └────┬─────┘                  └────┬─────┘
                  │ arrival                      │ within 100px (early)
                  ▼                              ▼
             ┌──────────┐                  ┌────────────────────────┐
             │ Resting  │ (energy recovery)│ HealingAtFountain      │
             └────┬─────┘                  │ {recover_until: 0.75}  │
                  │ energy >= 90%          │ drift check: re-target │
                  ▼                        │ if pushed > 100px      │
             back to previous cycle        └────┬───────────────────┘
                                                │ HP >= threshold
                                                ▼
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

| ECS Component | Variants | Purpose |
|-----------|----------|---------|
| Activity | `Idle, Working, OnDuty{ticks_waiting}, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain{recover_until}, Wandering, Raiding{target}, Returning{loot: Vec<(ItemKind, i32)>}, Mining{mine_pos}, MiningAtMine` | What the NPC is *doing* — mutually exclusive |
| CombatState | `None, Fighting{origin}, Fleeing` | Whether the NPC is *fighting* — orthogonal to Activity |

`Activity::is_transit()` returns true for Patrolling, GoingToWork, GoingToRest, GoingToHeal, Wandering, Raiding, Returning, Mining. Used by `gpu_position_readback` for arrival detection.

`Resting` is a unit variant — energy recovery only. NPCs go home (spawner) to rest.

`HealingAtFountain { recover_until: 0.75 }` — HP recovery at town fountain. NPC waits until HP >= threshold, then resumes. Separate from energy rest.

`Returning { loot: Vec<(ItemKind, i32)> }` — carried resources are part of the activity. Loot accumulates from NPC kills (`npc_def.loot_drop`), building destruction (`BuildingDef::loot_drop()`), farm stealing, and mine extraction. Multiple loot types can be carried simultaneously.

`Mining { mine_pos: Vec2 }` — miner walking to a gold mine. `MiningAtMine` — miner actively extracting gold (claims occupancy, progress-based 4-hour work cycle with gold progress bar overhead).

### NPC ECS Components

All NPC gameplay state lives in ECS components on entities. `EntityMap` provides slot→entity index only (via `NpcEntry`).

| Component | Type | Purpose |
|-----------|------|---------|
| Energy | `f32` | 0-100, drains while active, recovers while resting |
| Personality | `{ trait1, trait2 }` | 0-2 spectrum traits (7 axes, signed magnitude) affecting stats and decisions |
| NpcWorkState | `{ worksite: Option<EntityUid> }` | Always-present — single claimed worksite (if any). UID-based for ABA safety. All mutations (claim/release/retarget) go through `WorkIntentMsg` → `resolve_work_targets` system. |
| NpcFlags | `{ healing, starving, direct_control, migrating, at_destination }` | High-churn booleans bundled to avoid archetype moves |
| CachedStats | `{ max_health, damage, range, ... }` | Resolved combat stats |
| Home | `Vec2` | NPC's spawner building position — rest destination. Set to (-1,-1) when building destroyed (orphaned/homeless). |
| PatrolRoute | `{ posts, current }` | Optional — patrol unit's ordered patrol posts |
| Stealer | marker | Optional — NPC steals from farms (raiders) |
| LeashRange | `f32` | Optional — disengage combat if chased this far from origin |
| SquadId | `i32` | Optional — squad assignment, military units follow squad target |
| CarriedGold | `i32` | Gold being carried by miner/raider |

## Systems

### decision_system (Unified Priority Cascade)
- Iterates a focused ECS query `(Entity, &GpuSlot, &Job, &TownId, &Faction)` with `Without<Building>, Without<Dead>` filters for the outer NPC loop. Reads/writes mutable NPC state via `DecisionNpcState` + `NpcDataQueries` SystemParam bundles (`get_mut(entity)` per NPC). Skips `direct_control` NPCs entirely, skips NPCs in transit (`activity.is_transit()`). Work state managed via always-present `NpcWorkState` component (no `Commands` needed — no archetype churn). Patrol route data read inline at usage sites (no per-NPC Vec clone). **Conditional writeback**: captures original values at loop top, compares at end — only calls `get_mut()` for changed fields (most NPCs exit early via `break 'decide`). On transition to `Idle`, submits a self-position movement intent to clear stale GPU targets (prevents oscillation with nearby NPCs). `EntityMap` retained for building instance lookups (farms, waypoints, mines, occupancy)
- Uses **SystemParam bundles** for farm and economy parameters (see Overview)
- `DecisionExtras` includes `work_intents: MessageWriter<WorkIntentMsg>` — all worksite claim/release/retarget delegated to `resolve_work_targets` via fire-and-forget messages (no inline `entity_map.release()` or `try_claim_worksite()` calls)
- `worksite_deferred` flag per NPC: set when WorkIntentMsg sent, skips NpcWorkState write-back (resolver owns the component that frame)
- Bucket-gated: two-cadence top-of-loop gate (see [performance.md](performance.md#bucket-gated-decision-system))
- Matches on Activity and CombatState enums in priority order:

**Squad policy hard gate** (before combat, after arrivals):
- Any NPC with `SquadId` and squad `rest_when_tired` enabled: if energy < `ENERGY_TIRED_THRESHOLD` (30) OR (energy < `ENERGY_WAKE_THRESHOLD` (90) AND already `GoingToRest`/`Resting`), set `GoingToRest` targeting home. Hysteresis prevents oscillation — once resting, stays resting until 90% energy.
- Clears `CombatState::Fighting` if active.

**Priority 0: Arrival transitions**
- If `AtDestination`: match on Activity variant
  - `Patrolling` → check squad rest first (tired squad members → `GoingToRest` targeting home instead of `OnDuty`); otherwise `Activity::OnDuty { ticks_waiting: 0 }`
  - `GoingToRest` → `Activity::Resting` (sleep icon derived by `sync_visual_sprites`)
  - `GoingToHeal` → `Activity::HealingAtFountain { recover_until: policy.recovery_hp }` (healing aura handles HP recovery)
  - `GoingToWork` → Farmer: checks `worksite` slot. If occupied by another, sends `WorkIntent::Retarget` message (or idles if none free). If not occupied, checks Ready: if Ready, `harvest()` + sends `WorkIntent::Release` + `Returning { loot: Food }`. If not Ready, `Working` (tending).
  - `Raiding { .. }` → steal if farm ready, else find a different farm (excludes current position, skips tombstoned); if no other farm exists, return home
  - `Mining { mine_pos }` → find mine at position, check gold > 0, send `WorkIntent::Claim` message, insert `MiningProgress(0.0)`, set `Activity::MiningAtMine`
  - `Returning { .. }` → if home is valid, redirect to home (may have arrived at wrong place after DC removal); otherwise transition to Idle
  - `Wandering` → `Activity::Idle` (wander completes, re-enters decision scoring)
- Removes `AtDestination` after handling

**Priority 1-3: Combat decisions**
- If `CombatState::Fighting` + should flee: policy-driven flee thresholds per job — archers use `archer_flee_hp`, farmers and miners use `farmer_flee_hp`, raiders hardcoded 0.50. Threshold compared against `health.0 / max_hp` (from `CachedStats.max_health` via separate query). `archer_aggressive` disables archer flee, `farmer_fight_back` disables farmer/miner flee. Dynamic threat assessment via GPU spatial grid (enemies vs allies within 200px, computed in npc_compute.wgsl Mode 2, packed u32 readback via `GpuReadState.threat_counts`, throttled every 30 frames on CPU). Preserves existing `Activity::Returning` loot when fleeing.
- If `CombatState::Fighting` + should leash: archers check `archer_leash` policy (if disabled, archers chase freely), raiders use per-entity `LeashRange` component. Preserves existing `Activity::Returning` loot when leashing.
- If `CombatState::Fighting`: skip (attack_system handles targeting)

**Stuck-transit redirect** (bucket-gated, before GoingToHeal check):
- `Wandering`: re-scatter from current position (128px random offset, clamped within 200px of home). Prevents NPCs stuck behind obstacles from idling forever.
- `Patrolling`: re-submit scatter to current patrol post (128px scatter). Unsticks patrol units blocked by walls or congestion.
- Other transit activities: fall through to GoingToHeal check.

**Early arrival: GoingToHeal proximity check** (before transit skip)
- If `GoingToHeal` + within `HEAL_DRIFT_RADIUS` (100px) of town center: transition to `HealingAtFountain` immediately — NPC stops walking as soon as it enters healing range, doesn't need to reach the exact center

**Priority 4a: HealingAtFountain wake**
- If `HealingAtFountain { recover_until }` + HP / max_hp >= recover_until: set `Activity::Idle`
- **Drift check**: if not recovered and NPC is >100px from town center, re-target fountain (separation physics can push NPCs out of healing range)

**Priority 4b: Resting wake**
- If `Activity::Resting` + energy >= `ENERGY_WAKE_THRESHOLD` (90%): set `Activity::Idle`, proceed to scoring

**Priority 5: Unified worksite occupancy (farm + mine)**
- Single merged block handles both `Activity::Working` and `Activity::MiningAtMine` using config from `BuildingDef.worksite` (`WorksiteDef` in `BUILDING_REGISTRY`). Config fields: `max_occupants` (Farm=1, GoldMine=5), `drift_radius` (Farm=20, Mine=MINE_WORK_RADIUS=40), `upgrade_job` ("Farmer"/"Miner"), `harvest_item` (Food/Gold), `town_scoped` (Farm=true, GoldMine=false — mines are usable by any faction).
- **Worksite safety invariant** (validated before energy check, gated on `!worksite_deferred`): (1) no `worksite` → Idle, (2) worksite destroyed or wrong town (town-scoped only) → `WorkIntent::Release` + Idle, (3) contention: `occupant_count > ws.max_occupants` → `WorkIntent::Release` + Idle. Self-heals invalid state from older saves or edge cases.
- **Drift check**: if NPC distance > `ws.drift_radius` from worksite position, submit intent back (stay claimed, no release). Applies to both farm and mine.
- **Harvest check**: if `growth_ready`, `inst.harvest()` → yield multiplied by `UPGRADES.stat_mult(ws.upgrade_job, Yield)` → `WorkIntent::Release` → `Activity::Returning { loot: [(ws.harvest_item, final_yield)] }` targeting home.
- **Tired check**: energy < `ENERGY_TIRED_THRESHOLD` → `WorkIntent::Release` → Idle.

**Priority 6: Patrol**
- If `Activity::OnDuty { ticks_waiting }` + energy < `ENERGY_TIRED_THRESHOLD`: drop to `Idle` (falls through to scoring where Rest wins). **Squad exception**: archers in a squad with `rest_when_tired == false` stay on duty — they never leave post for energy reasons.
- If `Activity::OnDuty { ticks_waiting }` + ticks >= `GUARD_PATROL_WAIT` (60): advance `PatrolRoute`, set `Activity::Patrolling`

**Priority 7: Idle scoring (Utility AI)**
- **Squad override**: NPCs with a `SquadId` component check `SquadState.squads[id].target` before normal patrol logic. If squad has a target, unit walks to squad target instead of patrol posts. Falls through to normal behavior if no target is set.
- **Fighters**: Patrol waypoints like archers/crossbows, respond to squad targets. Work-allowed check uses `patrol_query` (needs `PatrolRoute`).
- **Raiders**: Squad-driven only, not idle-scored — raiders without a squad wander near town.
- **Healing priority**: if `prioritize_healing` policy enabled, energy > 0, HP < `recovery_hp`, and town center known → `GoingToHeal` targeting fountain. Applies to all jobs (including raiders — they heal at their town center). Skipped when starving (energy=0) because HP is capped at 50% by starvation — NPC must rest for energy first.
- **Work schedule gate**: Work only scored if the per-job schedule allows it — farmers and miners use `farmer_schedule`, archers use `archer_schedule` (`Both` = always, `DayOnly` = hours 6-20, `NightOnly` = hours 20-6)
- **Off-duty behavior**: when work is gated out by schedule, off-duty policy applies: `GoToBed` boosts Rest to 80, `StayAtFountain` targets town center, `WanderTown` boosts Wander to 80
- Score Eat/Rest/Work/Wander with personality multipliers and HP modifier
- Select via weighted random, execute action
- **Food check**: Eat only scored if town has food in storage
- **Farmer work branch**: Farmers send `WorkIntent::Claim { kind: Farm, ... }` message + set `Activity::GoingToWork`. The resolver (`resolve_work_targets`) performs spatial search via `find_farm_target()` (delegates to `EntityMap.find_nearest_worksite()` with cell-ring expansion, `BuildingKind::Farm` + town-scoped, `WorksiteFallback::TownOnly`, scoring: `(not_ready, inverted_growth_bits, dist2_bits)`), claims via `try_claim_worksite()`, updates `NpcWorkState.worksite`, and submits movement. On claim failure, resolver sets `Activity::Idle`. While en-route (`GoingToWork`), farmers re-check occupancy at Tier 3 cadence — if another farmer claimed the target farm first, the en-route farmer sends `WorkIntent::Retarget` from current position (or idles if none).
- **Miner work branch**: Miners send `WorkIntent::Claim { kind: GoldMine, ... }` message. If the miner's `MinerHome` has `assigned_mine` set (via building inspector UI), that mine is used directly. Otherwise, the resolver uses `find_mine_target()` (`EntityMap.find_nearest_worksite()` with `WorksiteFallback::AnyTown`). Scoring: `(priority: u8, dist2_bits: u32)` — ready(0) > unoccupied(1) > occupied(2), then nearest. Walks to mine (`Activity::Mining { mine_pos }`). Miners share farmer schedule/flee/off-duty policies.
- **Decision logging**: Each decision logged to `NpcLogCache`

### on_duty_tick_system
- Query-first: `(&mut Activity, &CombatState)` with `Without<Building>, Without<Dead>` — no `EntityMap` dependency
- Increments `ticks_waiting` each frame for NPCs with `Activity::OnDuty` where `CombatState` is not Fighting

### arrival_system (Proximity Checks)
- **Proximity-based delivery** for Returning NPCs: matches `Activity::Returning { .. }`, checks distance to home, delivers food and/or gold within DELIVERY_RADIUS (50px). All NPCs (including farmers) go `Idle` after delivery — the decision system re-evaluates the best target. Gold delivered to `GoldStorage` per town.
- **Worksite harvest + drift** handled entirely by `decision_system` Priority 5 unified worksite block (not arrival_system)
- **Healing drift check** in decision_system: `HealingAtFountain` NPCs pushed >100px from town center by separation physics get re-targeted to fountain (prevents deadlock where NPC is outside healing range but stuck in healing state)
- **GoingToHeal early arrival** in decision_system: NPCs transition to `HealingAtFountain` as soon as they're within 100px of town center, before reaching the exact pixel
- Arrival detection (`is_transit()` → `AtDestination`) is handled by `gpu_position_readback` in movement.rs
- All state transitions handled by decision_system Priority 0 (central brain model)

### energy_system
- NPCs with `Activity::Resting` or `Activity::HealingAtFountain`: recover `ENERGY_RECOVER_RATE` per tick
- All other NPCs: drain `ENERGY_DRAIN_RATE` per tick
- Clamp to 0.0-100.0
- **Note**: HealingAtFountain also recovers energy to prevent ping-pong (NPC leaves fountain tired → goes home → not healed → returns to fountain)
- All state transitions (wake-up, stop working) are handled in decision_system to keep decisions centralized

### healing_system
Candidate-driven pipeline (see [performance.md](performance.md#candidate-driven-healing) for scaling details). Two-loop approach with `ActiveHealingSlots` resource:
- **Sustain-check (every frame)**: iterates only active slots, rechecks position with hysteresis radii, applies healing with starving HP cap, clears stale/dead slots
- **Enter-check (cadenced)**: bucket-filtered per town, collects candidates then activates in one mutation pass with dedup
- **Building healing**: gated behind `BuildingHealState.needs_healing`, iterates only damaged buildings

**Zone lookup**: Precomputes `HashMap<i32, Vec<&HealingZone>>` once per frame from `HealingZoneCache.by_faction` for safe negative/sparse faction indexing.

**Starvation HP cap**: Moved to `starvation_system` (economy.rs) — always clamps HP for all starving NPCs each hour tick. `healing_system` still caps healing at 50% for starving NPCs in zones.

- Sets/clears `NpcFlags.healing` with `MarkVisualDirty` on transitions
- Debug: `healing_active_count`, `healing_enter_checks`, `healing_exits` + legacy fields

*Economy systems (game_time, farm_growth, raider_forage, raider_respawn, starvation) documented in [economy.md](economy.md).*

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

Patrol units (archers, crossbows, and fighters, identified by `Job::is_patrol_unit()`) have a `PatrolRoute` with ordered posts (built from `EntityMap` at spawn via `build_patrol_route`). The cycle:

1. Spawn → walk to post 0 (`Patrolling`)
2. Arrive → stand at post (`OnDuty`, ticks counting)
3. 60 ticks → advance to post 1 (`Patrolling`)
4. Arrive → `OnDuty` again
5. After last post, wrap to post 0

Each town has 4 waypoints at corners. Patrol units cycle clockwise. Patrol routes are rebuilt by `rebuild_patrol_routes_system` (runs in `Step::Behavior`) only when `MessageReader<PatrolsDirtyMsg>` has messages — i.e. when waypoints are built, destroyed, or reordered via the Patrols tab. The system applies any pending `PatrolSwapMsg` from the UI, then builds routes once per town (cached) and assigns to all patrol units in that town. Current patrol index is clamped to the new route length. The system also inserts `PatrolRoute` for patrol units that spawned before waypoints existed (queries `Without<PatrolRoute>` and inserts when town has waypoints).

## Squads

Military unit groups for both player and AI. 10 player-reserved squads + AI squads appended after. All military NPCs (determined by `Job::is_military()`: archers, crossbows, fighters, raiders) can be squad members. `SquadId(i32)` is an optional ECS component — inserted on recruitment, removed on dismiss.

**Behavior override**: In `decision_system`'s squad sync block, any NPC with `squad_id` checks `SquadState.squads[id].target`. If a target exists, the unit walks there (`Activity::Patrolling` with squad target). On arrival, `Activity::OnDuty` (same as waypoint). If no target is set and patrol disabled, unit stops (`Activity::Idle`). Squad sync also handles `Activity::Raiding` (raiders redirect to squad target).

**Manual micro override**: NPCs with a `manual_target` field skip the squad sync block entirely — player-assigned attack targets take priority over squad auto-redirect. The combat system handles `ManualTarget` directly (see [combat.md](combat.md#attack-system)).

**Squad sync**: The squad sync block always submits a movement intent to the squad target at `MovementPriority::Squad` (2). The movement system deduplicates unchanged targets, so redundant writes are cheap. Non-transit, non-OnDuty activities transition to `Patrolling`. `OnDuty` scatter targets the squad target (not patrol post) when a squad target is active. Patrol cycling is suppressed when the squad has an active target, preventing archers from walking back to their patrol waypoints.

**Rest-when-tired**: Squad members respect `rest_when_tired` flag via four gates: (1) arrival handler catches tired members before `OnDuty`, (2) hard gate before combat priorities forces `GoingToRest`, (3) squad sync block skips resting members, (4) Priority 6 OnDuty+tired check skips leave-post when `rest_when_tired == false`. Gates 1-3 use hysteresis (enter at energy < 30, stay until energy ≥ 90). Gate 4 is the inverse — it prevents units from leaving post when the flag is off. `attack_system` skips `GoingToRest` NPCs to prevent GPU target override.

**Wounded→fountain**: After the rest-when-tired check, the squad sync block checks `prioritize_healing` policy. If enabled and HP / max_hp < `recovery_hp` (and energy > 0), the NPC is sent to fountain (`GoingToHeal`) instead of the squad target — but NPCs already in `GoingToHeal` or `HealingAtFountain` are left alone (prevents oscillation between healing states). This prevents flee-engage oscillation where low-HP squad members repeatedly flee combat, arrive home, get redirected by squad sync back to the enemy, and flee again. The check runs at the same priority level as the rest check so it can't be overridden by the squad target redirect.

**All survival behavior preserved**: Squad members still flee (policy-driven), rest when tired, heal at fountain when wounded, fight enemies they encounter, and leash back. The squad override only affects the *work decision*, not combat or energy priorities. Loot delivery takes priority over squad orders — NPCs with `Activity::Returning` carrying loot are not redirected until they deliver.

**Raider behavior**: Raiders are squad-driven — if assigned to a squad with a target, the squad sync block redirects them. Raiders without a squad wander near their town. Raider attacks run through the AI squad commander wave cycle.

**Recruitment**: `squad_cleanup_system` uses a focused ECS query `(&GpuSlot, &Job, &TownId, Option<&SquadId>)` with `Without<Building>, Without<Dead>` for recruit pool discovery. Player squads recruit from player-town units; AI squads recruit from their owner town's units. "Dismiss All" clears `squad_id` from all squad members — units resume normal behavior.

**Death cleanup**: `squad_cleanup_system` (Step::Behavior) removes dead NPC slots from `Squad.members` by checking `EntityMap`.

## Worksite Reservation Lifecycle

All worksite occupancy mutations are centralized in `resolve_work_targets` (work_targeting.rs) — the sole caller of `entity_map.release()` and `try_claim_worksite()` for NPC work slots. Systems send fire-and-forget `WorkIntentMsg` messages; the resolver processes them after `decision_system` in `Step::Behavior`. `NpcWorkState` has a single `worksite: Option<EntityUid>` field (merged from the previous two-field design that enabled desync bugs).

- **Claim**: `WorkIntent::Claim { entity, kind, town_idx, from }` — resolver searches for best worksite via `find_farm_target()`/`find_mine_target()`, calls `try_claim_worksite()`, updates `NpcWorkState.worksite`, submits movement via `PathRequestQueue`. On failure, sets `Activity::Idle`.
- **Release**: `WorkIntent::Release { entity, uid }` — resolver releases by carried UID (sender captures UID before write-back may clear it), clears `NpcWorkState.worksite`.
- **Retarget**: `WorkIntent::Retarget` — atomic release + re-claim at a new worksite.
- **Deferred write-back**: `decision_system` sets `worksite_deferred = true` when sending WorkIntentMsg, skipping NpcWorkState write-back that frame (resolver owns the component). The stale worksite invariant is also gated on `!worksite_deferred`.
- **No pre-claim at spawn**: `spawner_respawn_system` does not claim worksite slots — workers self-claim via behavior system on first work decision.

## Known Issues / Limitations

- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).
