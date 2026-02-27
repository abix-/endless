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
- `DecisionExtras`: npc logs, combat log, policies, squad state, timings, town upgrades
- `Res<EntityMap>`: sole source of truth for all building instance lookups (farms, waypoints, towns, gold mines)

Priority order (first match wins), with three-tier throttling via `NpcDecisionConfig.interval`:

**DirectControl skip** (before all priorities): NPCs with `direct_control` flag skip the entire decision system — no autonomous behavior whatsoever. The system clears `AtDestination` if present to prevent stale arrival flags. DC NPCs may accumulate loot in `Activity::Returning` while fighting (via `dc_no_return` toggle) — the Returning activity is inert while DC is active. When a DC right-click move/attack command is issued (`click_to_select_system` in render.rs), resting NPCs (`GoingToRest`/`Resting`) are woken to `Idle` so they respond to the command instead of sliding while asleep.

**Tier 1 — every frame:**
0. AtDestination → Handle arrival transitions (transient one-frame flag, can't miss)
-- Farmer en-route retarget (GoingToWork + target farm occupied → find nearest free farm or idle, throttled at Tier 3 cadence) --
-- Transit skip (`activity.is_transit()` → continue, with GoingToHeal proximity check at Tier 2 cadence) --

**Tier 2 — every 8 frames (~133ms):**
1. CombatState::Fighting + should_flee? → Flee
2. CombatState::Fighting + should_leash? → Leash
3. CombatState::Fighting → Skip (attack_system handles)

**Tier 3 — bucketed by `NpcDecisionConfig.interval` (default 2s, configurable 0.5-10s):**
4a. HealingAtFountain + HP >= threshold → Wake (HP-only check)
4b. Resting + energy >= 90% → Wake (energy-only check)
5. Working + tired? → Stop work
6. OnDuty + time_to_patrol? → Patrol
7. Idle → Score Eat/Rest/Work/Wander (wounded → fountain, tired → home)

Bucketing uses `(idx + frame) % bucket_count` where `bucket_count = interval × 60fps`. With 5100 NPCs at 2s default, only ~42 NPCs evaluate Tier 3 per frame.

All checks are **policy-driven per town**. Flee thresholds come from `TownPolicies` resource (indexed by `TownId`), not per-entity `FleeThreshold` components. Raiders use a hardcoded 0.50 threshold. `archer_aggressive` and `farmer_fight_back` policies disable flee entirely for their respective jobs.

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
| Work | `40.0 * hp_mult * energy_factor` | has job, HP > 30% |
| Wander | `10.0` | always |

**Eat action**: Instantly consumes 1 food from town storage and restores energy to 100. No travel required — NPCs eat at current location. Only available as emergency option when energy < `ENERGY_EAT_THRESHOLD` (10) — NPCs prefer resting over eating.

**HP-based work score**: `hp_mult = 0` if HP < 30%, otherwise `(hp_pct - 0.3) / 0.7`. This prevents critically wounded NPCs from working/raiding while still allowing starving NPCs (HP capped at 50%) to join raid queues at reduced priority.

**Energy-based work score**: `energy_factor = energy / ENERGY_TIRED_THRESHOLD` when energy < 30, otherwise 1.0. This scales work desire down linearly as energy drops, so rest naturally outcompetes work around energy ~24. Prevents the starvation death spiral where NPCs repeatedly choose work over rest, burn energy in farm-retarget loops, and hit energy 0.

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

| NpcInstance field | Variants | Purpose |
|-----------|----------|---------|
| activity: Activity | `Idle, Working, OnDuty{ticks_waiting}, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain{recover_until}, Wandering, Raiding{target}, Returning{loot: Vec<(ItemKind, i32)>}, Mining{mine_pos}, MiningAtMine` | What the NPC is *doing* — mutually exclusive |
| combat_state: CombatState | `None, Fighting{origin}, Fleeing` | Whether the NPC is *fighting* — orthogonal to Activity |

`Activity::is_transit()` returns true for Patrolling, GoingToWork, GoingToRest, GoingToHeal, Wandering, Raiding, Returning, Mining. Used by `gpu_position_readback` for arrival detection.

`Resting` is a unit variant — energy recovery only. NPCs go home (spawner) to rest.

`HealingAtFountain { recover_until: 0.75 }` — HP recovery at town fountain. NPC waits until HP >= threshold, then resumes. Separate from energy rest.

`Returning { loot: Vec<(ItemKind, i32)> }` — carried resources are part of the activity. Loot accumulates from NPC kills (`npc_def.loot_drop`), building destruction (`BuildingDef::loot_drop()`), farm stealing, and mine extraction. Multiple loot types can be carried simultaneously.

`Mining { mine_pos: Vec2 }` — miner walking to a gold mine. `MiningAtMine` — miner actively extracting gold (claims occupancy, progress-based 4-hour work cycle with gold progress bar overhead).

### NpcInstance Data Fields

All NPC state lives in `NpcInstance` (stored in `EntityMap.npcs`). No ECS components except `EntitySlot`.

| Field | Type | Purpose |
|-------|------|---------|
| town_idx | `i32` | Town identifier — every NPC belongs to one |
| energy | `f32` | 0-100, drains while active, recovers while resting |
| personality | `Personality` | 0-2 traits with magnitude affecting stats and decisions |
| assigned_farm | `Option<usize>` | Farm building slot farmer is working at (for occupancy tracking) |
| is_starving | `bool` | NPC energy at zero (50% HP cap, 50% speed) |
| healing | `bool` | NPC is inside healing aura (visual feedback) |
| cached_stats.max_health | `f32` | NPC's maximum health (for healing cap) |
| home | `Vec2` | NPC's spawner building position — rest destination |
| work_position | `Option<usize>` | Farmer's field / miner's mine building slot |
| mining_progress | `f32` | Mining work progress 0.0–1.0, set when miner starts at mine, cleared on extraction or interruption |
| patrol_route | `Option<PatrolRoute>` | Patrol unit's ordered patrol posts (archers, crossbows, fighters) |
| at_destination | `bool` | NPC arrived at destination (transient frame flag from gpu_position_readback) |
| is_stealer | `bool` | NPC steals from farms (enables steal systems) |
| leash_range | `Option<f32>` | Disengage combat if chased this far from combat origin (raiders only) |
| squad_id | `Option<i32>` | Squad assignment — military units follow squad target instead of patrolling |

## Systems

### decision_system (Unified Priority Cascade)
- Iterates `EntityMap.iter_npcs()` — all NPC state read/written via `NpcInstance` fields, no ECS queries. Skips `direct_control` NPCs entirely, skips NPCs in transit (`activity.is_transit()`)
- Uses **SystemParam bundles** for farm and economy parameters (see Overview)
- Reads `NpcDecisionConfig.interval` for Tier 3 bucket count (`interval × 60fps`)
- Three-tier throttling: arrivals every frame, combat every 8 frames, decisions bucketed by interval
- Matches on Activity and CombatState enums in priority order:

**Squad policy hard gate** (before combat, after arrivals):
- Any NPC with `SquadId` and squad `rest_when_tired` enabled: if energy < `ENERGY_TIRED_THRESHOLD` (30) OR (energy < `ENERGY_WAKE_THRESHOLD` (90) AND already `GoingToRest`/`Resting`), set `GoingToRest` targeting home. Hysteresis prevents oscillation — once resting, stays resting until 90% energy.
- Clears `CombatState::Fighting` if active.

**Priority 0: Arrival transitions**
- If `AtDestination`: match on Activity variant
  - `Patrolling` → check squad rest first (tired squad members → `GoingToRest` targeting home instead of `OnDuty`); otherwise `Activity::OnDuty { ticks_waiting: 0 }`
  - `GoingToRest` → `Activity::Resting` (sleep icon derived by `sync_visual_sprites`)
  - `GoingToHeal` → `Activity::HealingAtFountain { recover_until: policy.recovery_hp }` (healing aura handles HP recovery)
  - `GoingToWork` → check occupancy via `BuildingInstance.occupants`: if farm occupied, redirect to nearest free farm in own town (or idle if none); else check if farm is Ready via `EntityMap::find_farm_at(pos)` (O(1) spatial lookup) — if so, harvest and immediately enter `Returning { loot: Food }` (carry home without claiming); if not Ready, claim farm via `entity_map.claim(slot)` + `AssignedFarm(slot)` + `Working`. Note: farmer dynamically picked this farm via priority scan (ready > unoccupied, closest) during work decision — no permanent assignment.
  - `Raiding { .. }` → steal if farm ready, else find a different farm (excludes current position, skips tombstoned); if no other farm exists, return home
  - `Mining { mine_pos }` → find mine at position, check gold > 0 and occupancy < `MAX_MINE_OCCUPANCY`, claim occupancy via `entity_map.claim(slot)`, insert `MiningProgress(0.0)`, set `Activity::MiningAtMine`
  - `Returning { .. }` → if home is valid, redirect to home (may have arrived at wrong place after DC removal); otherwise transition to Idle
  - `Wandering` → `Activity::Idle` (wander targets are offset from home position, not current position, preventing unbounded drift)
- Removes `AtDestination` after handling

**Priority 1-3: Combat decisions**
- If `CombatState::Fighting` + should flee: policy-driven flee thresholds per job — archers use `archer_flee_hp`, farmers and miners use `farmer_flee_hp`, raiders hardcoded 0.50. Threshold compared against `health.0 / max_hp` (from `CachedStats.max_health` via separate query). `archer_aggressive` disables archer flee, `farmer_fight_back` disables farmer/miner flee. Dynamic threat assessment via GPU spatial grid (enemies vs allies within 200px, computed in npc_compute.wgsl Mode 2, packed u32 readback via `GpuReadState.threat_counts`, throttled every 30 frames on CPU). Preserves existing `Activity::Returning` loot when fleeing.
- If `CombatState::Fighting` + should leash: archers check `archer_leash` policy (if disabled, archers chase freely), raiders use per-entity `LeashRange` component. Preserves existing `Activity::Returning` loot when leashing.
- If `CombatState::Fighting`: skip (attack_system handles targeting)

**Early arrival: GoingToHeal proximity check** (before transit skip)
- If `GoingToHeal` + within `HEAL_DRIFT_RADIUS` (100px) of town center: transition to `HealingAtFountain` immediately — NPC stops walking as soon as it enters healing range, doesn't need to reach the exact center

**Priority 4a: HealingAtFountain wake**
- If `HealingAtFountain { recover_until }` + HP / max_hp >= recover_until: set `Activity::Idle`
- **Drift check**: if not recovered and NPC is >100px from town center, re-target fountain (separation physics can push NPCs out of healing range)

**Priority 4b: Resting wake**
- If `Activity::Resting` + energy >= `ENERGY_WAKE_THRESHOLD` (90%): set `Activity::Idle`, proceed to scoring

**Priority 5: Working/Mining progress**
- If `Activity::Working` + energy < `ENERGY_TIRED_THRESHOLD` (30%): set `Activity::Idle`, release occupancy via `entity_map.release(slot)`, remove `AssignedFarm`.
- If `Activity::MiningAtMine`: tick `MiningProgress` by `delta_hours / MINE_WORK_HOURS` (4h cycle). When progress >= 1.0 OR energy < tired threshold: extract gold scaled by progress fraction × `MINE_EXTRACT_PER_CYCLE` × GoldYield upgrade, release occupancy, remove `MiningProgress` + `WorkPosition`, set `Activity::Returning { loot: [(Gold, extracted)] }`. Gold progress bar rendered overhead via `MinerProgressRender` (atlas_id=6.0, gold color).

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
- **Farmer work branch**: Farmers dynamically scan same-town farms via `EntityMap::iter_kind_for_town(Farm, town_id)` — O(k) where k = farms in that town. Occupancy checked via `inst.occupants >= 1` (slot-indexed, no hashing). Priority: ready unoccupied > growing unoccupied, closest within each tier. `find_nearest_free` returns `(slot, position)` — inserts `WorkPosition(slot)` and sets `GoingToWork`. While en-route (`GoingToWork`), farmers re-check occupancy at Tier 3 cadence — if another farmer claimed the target farm first, the en-route farmer retargets to the nearest free farm (or idles if none). This prevents dogpiling: closest farmer wins, others keep searching.
- **Miner work branch**: Miners have a separate `Action::Work` → `Job::Miner` branch. If the miner's `MinerHome` has `assigned_mine` set (via building inspector UI), that mine is used directly. Otherwise, finds the nearest unoccupied mine and walks there (`Activity::Mining { mine_pos }`). Completely independent of farmer logic — no `mining_pct` roll. Miners share farmer schedule/flee/off-duty policies.
- **Decision logging**: Each decision logged to `NpcLogCache`

### on_duty_tick_system
- Query: NPCs with `Activity::OnDuty { ticks_waiting }` where `CombatState` is not Fighting
- Increments `ticks_waiting` each frame
- Separated from decision_system to allow mutable Activity access while main query has immutable view

### arrival_system (Proximity Checks)
- **Proximity-based delivery** for Returning NPCs: matches `Activity::Returning { .. }`, checks distance to home, delivers food and/or gold within DELIVERY_RADIUS (50px). All NPCs (including farmers) go `Idle` after delivery — the decision system re-evaluates the best target. Gold delivered to `GoldStorage` per town.
- **Working farmer harvest → carry home** (throttled every 30 frames): re-targets farmers who drifted >20px from their assigned farm; when farm becomes Ready, uses `EntityMap::find_farm_at_mut(pos)` for O(1) spatial lookup, calls `BuildingInstance::harvest()` (resets farm, returns yield), releases occupancy, removes `AssignedFarm`, enters `Returning { loot: [(Food, yield)] }` targeting home — farmer visibly carries food home for delivery
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
- Iterates `EntityMap.iter_npcs_mut()` — reads position, faction, town_idx, health, cached_stats from NpcInstance
- Reads town centers from `WorldData`
- All settlements (villager and raider) are Town entries with faction (unified town model)
- If NPC within `HEAL_RADIUS` (150px) of same-faction town center: heal `HEAL_RATE` (5 HP/sec)
- **Starvation HP cap**: Starving NPCs have HP capped at 50% of max_health (can't heal above this)
- Sets/clears `npc.healing` bool for visual feedback (heal icon derived by `sync_visual_sprites`)
- Debug: `get_health_debug()` returns healing_in_zone_count and healing_healed_count

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

Military unit groups for both player and AI. 10 player-reserved squads + AI squads appended after. All military NPCs (`is_military` flag on NpcInstance: archers, crossbows, fighters, raiders) can be squad members.

**Behavior override**: In `decision_system`'s squad sync block, any NPC with `squad_id` checks `SquadState.squads[id].target`. If a target exists, the unit walks there (`Activity::Patrolling` with squad target). On arrival, `Activity::OnDuty` (same as waypoint). If no target is set and patrol disabled, unit stops (`Activity::Idle`). Squad sync also handles `Activity::Raiding` (raiders redirect to squad target).

**Manual micro override**: NPCs with a `manual_target` field skip the squad sync block entirely — player-assigned attack targets take priority over squad auto-redirect. The combat system handles `ManualTarget` directly (see [combat.md](combat.md#attack-system)).

**Squad sync optimization**: The squad sync block only writes GPU targets when needed — not every frame. `OnDuty` units are redirected only when the squad target moves >100px from the unit's position. `Patrolling`, `Raiding`, `GoingToRest`, `Resting`, `GoingToHeal`, `HealingAtFountain`, and `Returning` units are left alone (already heading to target, resting, healing, or carrying loot home). Other activities (`Idle`, `Wandering`) get redirected immediately.

**Rest-when-tired**: Squad members respect `rest_when_tired` flag via four gates: (1) arrival handler catches tired members before `OnDuty`, (2) hard gate before combat priorities forces `GoingToRest`, (3) squad sync block skips resting members, (4) Priority 6 OnDuty+tired check skips leave-post when `rest_when_tired == false`. Gates 1-3 use hysteresis (enter at energy < 30, stay until energy ≥ 90). Gate 4 is the inverse — it prevents units from leaving post when the flag is off. `attack_system` skips `GoingToRest` NPCs to prevent GPU target override.

**Wounded→fountain**: After the rest-when-tired check, the squad sync block checks `prioritize_healing` policy. If enabled and HP / max_hp < `recovery_hp` (and energy > 0), the NPC is sent to fountain (`GoingToHeal`) instead of the squad target — but NPCs already in `GoingToHeal` or `HealingAtFountain` are left alone (prevents oscillation between healing states). This prevents flee-engage oscillation where low-HP squad members repeatedly flee combat, arrive home, get redirected by squad sync back to the enemy, and flee again. The check runs at the same priority level as the rest check so it can't be overridden by the squad target redirect.

**All survival behavior preserved**: Squad members still flee (policy-driven), rest when tired, heal at fountain when wounded, fight enemies they encounter, and leash back. The squad override only affects the *work decision*, not combat or energy priorities. Loot delivery takes priority over squad orders — NPCs with `Activity::Returning` carrying loot are not redirected until they deliver.

**Raider behavior**: Raiders are squad-driven — if assigned to a squad with a target, the squad sync block redirects them. Raiders without a squad wander near their town. Raider attacks run through the AI squad commander wave cycle.

**Recruitment**: `squad_cleanup_system` iterates `EntityMap.iter_npcs()` filtering alive military NPCs without `squad_id`. Player squads recruit from player-town units; AI squads recruit from their owner town's units. "Dismiss All" clears `squad_id` from all squad members — units resume normal behavior.

**Death cleanup**: `squad_cleanup_system` (Step::Behavior) removes dead NPC slots from `Squad.members` by checking `EntityMap`.

## Profiling

`decision_system` has sub-timers recorded via `SystemTimings.record()` for performance analysis:

| Timer | What it measures |
|-------|-----------------|
| `decision/arrival` | Priority 0 arrival transitions |
| `decision/combat` | Priority 1-3 combat decisions |
| `decision/idle` | Priority 7 idle scoring (utility AI) |
| `decision/squad` | Squad rest gate + squad sync + squad target redirect |
| `decision/work` | Priority 5 Working/MiningAtMine + farmer retarget + OnDuty checks |

| Counter | What it counts |
|---------|---------------|
| `decision/n_arrival` | NPCs entering arrival path |
| `decision/n_combat` | NPCs entering combat path |
| `decision/n_idle` | NPCs entering idle scoring |
| `decision/n_squad` | NPCs entering squad sync path |
| `decision/n_work` | NPCs entering work/mining/onduty check |
| `decision/n_transit_skip` | NPCs skipped by transit gate |
| `decision/n_total` | Total NPCs processed per frame |

All timers use `Instant::now()` guarded by the `profiling` flag (only measured when profiler enabled in settings).

## Known Issues / Limitations

- **No pathfinding**: NPCs walk in a straight line to target. They rely on separation physics to avoid each other, but can't navigate around buildings.
- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).
