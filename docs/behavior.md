# Behavior System

## Overview

NPC decision-making and state transitions. All run in `Step::Behavior` after combat is resolved. For economy systems (farm growth, starvation, camp foraging, raider respawning, game time), see [economy.md](economy.md).

**Unified Decision System**: All NPC decisions are handled by `decision_system` using a priority cascade. NPC state is modeled by two orthogonal enum components (concurrent state machines pattern):

- `Activity` enum: what the NPC is *doing* (Idle, Working, OnDuty, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain, Wandering, Raiding, Returning, Mining, MiningAtMine)
- `CombatState` enum: whether the NPC is *fighting* (None, Fighting, Fleeing)

Activity is preserved through combat — a Raiding NPC stays `Activity::Raiding` while `CombatState::Fighting`. When combat ends, the NPC resumes its previous activity.

The system uses **SystemParam bundles** for farm and economy parameters:
- `FarmParams`: farm states, `BuildingOccupancy` tracking, world data
- `EconomyParams`: food storage, food events, population stats
- `DecisionExtras`: npc logs, combat log, policies, squad state, timings, town upgrades
- `Res<BuildingSpatialGrid>`: CPU-side spatial grid for O(1) building lookups (farms, waypoints, towns, gold mines)

Priority order (first match wins), with three-tier throttling via `NpcDecisionConfig.interval`:

**Tier 1 — every frame:**
0. AtDestination → Handle arrival transitions (transient one-frame flag, can't miss)
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

| Component | Variants | Purpose |
|-----------|----------|---------|
| Activity | `Idle, Working, OnDuty{ticks_waiting}, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain{recover_until}, Wandering, Raiding{target}, Returning{loot: Vec<(ItemKind, i32)>}, Mining{mine_pos}, MiningAtMine` | What the NPC is *doing* — mutually exclusive |
| CombatState | `None, Fighting{origin}, Fleeing` | Whether the NPC is *fighting* — orthogonal to Activity |

`Activity::is_transit()` returns true for Patrolling, GoingToWork, GoingToRest, GoingToHeal, Wandering, Raiding, Returning, Mining. Used by `gpu_position_readback` for arrival detection.

`Resting` is a unit variant — energy recovery only. NPCs go home (spawner) to rest.

`HealingAtFountain { recover_until: 0.75 }` — HP recovery at town fountain. NPC waits until HP >= threshold, then resumes. Separate from energy rest.

`Returning { loot: Vec<(ItemKind, i32)> }` — carried resources are part of the activity. Loot accumulates from NPC kills (`npc_def.loot_drop`), building destruction (`BuildingDef::loot_drop()`), farm stealing, and mine extraction. Multiple loot types can be carried simultaneously.

`Mining { mine_pos: Vec2 }` — miner walking to a gold mine. `MiningAtMine` — miner actively extracting gold (claims occupancy, progress-based 4-hour work cycle with gold progress bar overhead).

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
| Home | `{ x, y }` | NPC's spawner building position (FarmerHome/ArcherHome/Tent) — rest destination |
| WorkPosition | `{ x, y }` | Farmer's field / miner's mine position |
| MiningProgress | `f32` | Mining work progress 0.0–1.0, inserted when miner starts at mine, removed on extraction or interruption |
| PatrolRoute | `{ posts: Vec<Vec2>, current: usize }` | Patrol unit's ordered patrol posts (archers, crossbows, fighters) |
| AtDestination | marker | NPC arrived at destination (transient frame flag from gpu_position_readback) |
| Stealer | marker | NPC steals from farms (enables steal systems) |
| LeashRange | `{ distance: f32 }` | Disengage combat if chased this far from combat origin (raiders only) |
| SquadId | `i32` (0-9) | Squad assignment — military units (archers/crossbows/fighters/raiders) with this follow squad target instead of patrolling |

## Systems

### decision_system (Unified Priority Cascade)
- Query: NPCs with `&mut Activity`, `&mut CombatState`, skips NPCs in transit (`activity.is_transit()`)
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
  - `GoingToWork` → check `BuildingOccupancy`: if farm occupied, redirect to nearest free farm in own town (or idle if none); else check if farm is Ready — if so, harvest and immediately enter `Returning { loot: Food }` (carry home without claiming); if not Ready, claim farm via `BuildingOccupancy.claim()` + `AssignedFarm` + `Working`
  - `Raiding { .. }` → steal if farm ready, else find a different farm (excludes current position, skips tombstoned); if no other farm exists, return home
  - `Mining { mine_pos }` → find mine at position, check gold > 0 and occupancy < `MAX_MINE_OCCUPANCY`, claim occupancy via `BuildingOccupancy`, insert `MiningProgress(0.0)`, set `Activity::MiningAtMine`
  - `Wandering` → `Activity::Idle` (wander targets are offset from home position, not current position, preventing unbounded drift)
- Removes `AtDestination` after handling

**Priority 1-3: Combat decisions**
- If `CombatState::Fighting` + should flee: policy-driven flee thresholds per job — archers use `archer_flee_hp`, farmers and miners use `farmer_flee_hp`, raiders hardcoded 0.50. `archer_aggressive` disables archer flee, `farmer_fight_back` disables farmer/miner flee. Dynamic threat assessment via GPU spatial grid (enemies vs allies within 200px, computed in npc_compute.wgsl Mode 2, packed u32 readback via `GpuReadState.threat_counts`, throttled every 30 frames on CPU). Preserves existing `Activity::Returning` loot when fleeing.
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
- If `Activity::Working` + energy < `ENERGY_TIRED_THRESHOLD` (30%): set `Activity::Idle`, release `AssignedFarm`
- If `Activity::MiningAtMine`: tick `MiningProgress` by `delta_hours / MINE_WORK_HOURS` (4h cycle). When progress >= 1.0 OR energy < tired threshold: extract gold scaled by progress fraction × `MINE_EXTRACT_PER_CYCLE` × GoldYield upgrade, release occupancy, remove `MiningProgress` + `WorkPosition`, set `Activity::Returning { loot: [(Gold, extracted)] }`. Gold progress bar rendered overhead via `MinerProgressRender` (atlas_id=6.0, gold color).

**Priority 6: Patrol**
- If `Activity::OnDuty { ticks_waiting }` + energy < `ENERGY_TIRED_THRESHOLD`: drop to `Idle` (falls through to scoring where Rest wins). **Squad exception**: archers in a squad with `rest_when_tired == false` stay on duty — they never leave post for energy reasons.
- If `Activity::OnDuty { ticks_waiting }` + ticks >= `GUARD_PATROL_WAIT` (60): advance `PatrolRoute`, set `Activity::Patrolling`

**Priority 7: Idle scoring (Utility AI)**
- **Squad override**: NPCs with a `SquadId` component check `SquadState.squads[id].target` before normal patrol logic. If squad has a target, unit walks to squad target instead of patrol posts. Falls through to normal behavior if no target is set.
- **Fighters**: Patrol waypoints like archers/crossbows, respond to squad targets. Work-allowed check uses `patrol_query` (needs `PatrolRoute`).
- **Raiders**: Squad-driven only, not idle-scored — raiders without a squad wander near camp.
- **Healing priority**: if `prioritize_healing` policy enabled, energy > 0, HP < `recovery_hp`, and town center known → `GoingToHeal` targeting fountain. Applies to all jobs (including raiders — they heal at their camp center). Skipped when starving (energy=0) because HP is capped at 50% by starvation — NPC must rest for energy first.
- **Work schedule gate**: Work only scored if the per-job schedule allows it — farmers and miners use `farmer_schedule`, archers use `archer_schedule` (`Both` = always, `DayOnly` = hours 6-20, `NightOnly` = hours 20-6)
- **Off-duty behavior**: when work is gated out by schedule, off-duty policy applies: `GoToBed` boosts Rest to 80, `StayAtFountain` targets town center, `WanderTown` boosts Wander to 80
- Score Eat/Rest/Work/Wander with personality multipliers and HP modifier
- Select via weighted random, execute action
- **Food check**: Eat only scored if town has food in storage
- **Miner work branch**: Miners have a separate `Action::Work` → `Job::Miner` branch. If the miner's `MinerHome` has `assigned_mine` set (via building inspector UI), that mine is used directly. Otherwise, finds the nearest unoccupied mine and walks there (`Activity::Mining { mine_pos }`). Completely independent of farmer logic — no `mining_pct` roll. Miners share farmer schedule/flee/off-duty policies.
- **Decision logging**: Each decision logged to `NpcLogCache`

### on_duty_tick_system
- Query: NPCs with `Activity::OnDuty { ticks_waiting }` where `CombatState` is not Fighting
- Increments `ticks_waiting` each frame
- Separated from decision_system to allow mutable Activity access while main query has immutable view

### arrival_system (Proximity Checks)
- **Proximity-based delivery** for Returning NPCs: matches `Activity::Returning { .. }`, checks distance to home, delivers food and/or gold within DELIVERY_RADIUS (50px). Farmers return to `GoingToWork` (targeting their `WorkPosition`) after delivery; all other NPCs go `Idle`. Gold delivered to `GoldStorage` per town.
- **Working farmer harvest → carry home** (throttled every 30 frames): re-targets farmers who drifted >20px from their assigned farm; when farm becomes Ready, calls `harvest()` (resets farm, returns yield), releases occupancy, removes `AssignedFarm`, enters `Returning { loot: [(Food, yield)] }` targeting home — farmer visibly carries food home for delivery
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

Patrol units (archers, crossbows, and fighters, identified by `Job::is_patrol_unit()`) have a `PatrolRoute` with ordered posts (built from WorldData at spawn). The cycle:

1. Spawn → walk to post 0 (`Patrolling`)
2. Arrive → stand at post (`OnDuty`, ticks counting)
3. 60 ticks → advance to post 1 (`Patrolling`)
4. Arrive → `OnDuty` again
5. After last post, wrap to post 0

Each town has 4 waypoints at corners. Patrol units cycle clockwise. Patrol routes are rebuilt by `rebuild_patrol_routes_system` (runs in `Step::Behavior`) only when `DirtyFlags.patrols` is set — i.e. when waypoints are built, destroyed, or reordered via the Patrols tab. The system applies any pending `DirtyFlags.patrol_swap` from the UI, then builds routes once per town (cached) and assigns to all patrol units in that town. Current patrol index is clamped to the new route length. The system also inserts `PatrolRoute` for patrol units that spawned before waypoints existed (queries `Without<PatrolRoute>` and inserts when town has waypoints).

## Squads

Military unit groups for both player and AI. 10 player-reserved squads + AI squads appended after. All military NPCs (`SquadUnit` component: archers, crossbows, fighters, raiders) can be squad members.

**Behavior override**: In `decision_system`'s squad sync block, any NPC with `SquadId` checks `SquadState.squads[id].target`. If a target exists, the unit walks there (`Activity::Patrolling` with squad target). On arrival, `Activity::OnDuty` (same as waypoint). If no target is set and patrol disabled, unit stops (`Activity::Idle`). Squad sync also handles `Activity::Raiding` (raiders redirect to squad target).

**Manual micro override**: NPCs with a `ManualTarget` component skip the squad sync block entirely — player-assigned attack targets take priority over squad auto-redirect. The combat system handles `ManualTarget` directly (see [combat.md](combat.md#attack-system)).

**Squad sync optimization**: The squad sync block only writes GPU targets when needed — not every frame. `OnDuty` units are redirected only when the squad target moves >100px from the unit's position. `Patrolling`, `Raiding`, `GoingToRest`, `Resting`, and `Returning` units are left alone (already heading to target, resting, or carrying loot home). Other activities (`Idle`, `Wandering`) get redirected immediately.

**Rest-when-tired**: Squad members respect `rest_when_tired` flag via four gates: (1) arrival handler catches tired members before `OnDuty`, (2) hard gate before combat priorities forces `GoingToRest`, (3) squad sync block skips resting members, (4) Priority 6 OnDuty+tired check skips leave-post when `rest_when_tired == false`. Gates 1-3 use hysteresis (enter at energy < 30, stay until energy ≥ 90). Gate 4 is the inverse — it prevents units from leaving post when the flag is off. `attack_system` skips `GoingToRest` NPCs to prevent GPU target override.

**All survival behavior preserved**: Squad members still flee (policy-driven), rest when tired, heal at fountain when wounded, fight enemies they encounter, and leash back. The squad override only affects the *work decision*, not combat or energy priorities. Loot delivery takes priority over squad orders — NPCs with `Activity::Returning` carrying loot are not redirected until they deliver.

**Raider behavior**: Raiders are squad-driven — if assigned to a squad with a target, the squad sync block redirects them. Raiders without a squad wander near their camp. The old `RaidQueue` (group formation + dispatch to farms) has been replaced by AI squad commander wave-based attacks.

**Recruitment**: `squad_cleanup_system` queries alive `SquadUnit` NPCs without `SquadId`. Player squads recruit from player-town units; AI squads recruit from their owner town's units. "Dismiss All" removes `SquadId` from all squad members — units resume normal behavior.

**Death cleanup**: `squad_cleanup_system` (Step::Behavior) removes dead NPC slots from `Squad.members` by checking `NpcEntityMap`.

## Known Issues / Limitations

- **No pathfinding**: NPCs walk in a straight line to target. They rely on separation physics to avoid each other, but can't navigate around buildings.
- **Energy drains during transit**: NPCs lose energy while walking home to rest. Distant homes could drain to 0 before arrival (clamped, but NPC arrives empty).
