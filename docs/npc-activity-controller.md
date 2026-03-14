# NPC Activity Controller

Target-state spec for refactoring NPC behavior into a deterministic `Def -> Instance -> Controller` model aligned with [k8s.md](k8s.md).

## Goal

Make NPC behavior deterministic, inspectable, and phase-driven.

The current system already has the right ingredients:
- `ActivityDef` as a registry-backed definition
- `Activity` as runtime state
- `CombatState` as an orthogonal state machine
- controller-style systems (`decision_system`, `resolve_work_targets`, movement, combat)

The problem is that behavior progress is still encoded implicitly in branch order and `NpcFlags::at_destination`. The redesign makes lifecycle progress explicit so behavior is driven by state, not by the order of `if` statements.

## Core model

### K8s alignment

Map behavior to the same three-layer pattern as [k8s.md](k8s.md):

| Layer | K8s analogue | Behavior equivalent |
|---|---|---|
| CRD / Def | schema | `ActivityDef` metadata in `ACTIVITY_REGISTRY` |
| CR / Instance | spec + status | `Activity { kind, phase, ... }` on each NPC |
| Controller | reconcile loop | behavior systems that derive facts, choose transitions, and apply side effects |

Behavior uses:
- `Activity.kind` = intent/spec. What the NPC is trying to do.
- `Activity.phase` = status. Where the NPC is in that lifecycle right now.
- `CombatState` = orthogonal overlay. Fighting does not replace `Activity`.

### Data model

Keep `ActivityKind` as the top-level behavior intent:
- `Idle`
- `Work`
- `Patrol`
- `SquadAttack`
- `Rest`
- `Heal`
- `Wander`
- `Raid`
- `ReturnLoot`
- `Mine`

`Eat` is intentionally not an `ActivityKind`. It remains an inline idle action that consumes food immediately without entering a new activity lifecycle.

Add a small generic `ActivityPhase` enum:
- `Ready`
- `Transit`
- `Active`
- `Holding`

Do not encode destination identity into the phase type. Adding a new location should usually add or extend target data, not add a new phase variant.

Add an explicit target field:

```rust
pub enum ActivityTarget {
    None,
    Home,
    Fountain,
    PatrolPost { route: u16, index: u16 },
    SquadPoint(Vec2),
    Worksite,
    RaidPoint(Vec2),
    Dropoff,
    WanderPoint(Vec2),
}
```

Expected `Activity` shape:

```rust
pub struct Activity {
    pub kind: ActivityKind,
    pub phase: ActivityPhase,
    pub target: ActivityTarget,
    pub recover_until: f32,
    pub ticks_waiting: u32,
}
```

Field meaning:
- `kind` selects the lifecycle.
- `phase` is the authoritative progress marker.
- `target` identifies where or what this activity is operating on.
- `recover_until` remains payload data for healing or recovery thresholds.
- `ticks_waiting` remains phase-local timing for guard posts and similar hold phases.

`ActivityTarget::Worksite` is intentionally semantic, not an identity payload. It means "the NPC's currently assigned claimed worksite."

Do not store a raw Bevy `Entity`, slot index, or cached worksite position inside `Activity.target`:
- actual claim ownership remains on `NpcWorkState.worksite`
- worksite validation and queue order remain on `EntityMap`
- movement destination continues to come from the authoritative claimed worksite
- save/load must not depend on unstable runtime entity ids embedded in `Activity`

`ticks_waiting` reset rule:
- reset to `0` whenever an NPC enters a new phase
- reset to `0` whenever the activity target changes
- in practice, `Patrol + PatrolPost{...} + Holding` starts with `ticks_waiting = 0` on entry and counts upward only while the NPC remains in that same holding state
- no other phase should carry stale guard-wait time

`NpcWorkState.worksite` remains the sole authority for claimed worksite ownership. Do not duplicate worksite truth inside `Activity`.

`Activity.worksite` in the current code is already vestigial and should be removed early in the refactor. It is not a compatibility field worth preserving. Remove it in Slice 1 once the new `kind + phase + target` model is in place.

`Activity.target_pos` is not dead code today. It is an active legacy payload used by pre-phase target-driven activities such as `Mine` and `Raid`, including the mine drift/requeue path.

Implementation rule:
- replace `Activity.target_pos` deliberately with `ActivityTarget` or equivalent explicit target data
- if `ActivityTarget` lands globally up front, remove `target_pos` immediately
- otherwise keep `target_pos` only as a temporary legacy payload for activities not yet migrated and delete it no later than Slice 3
- do not describe `target_pos` as vestigial until those remaining activity paths are actually migrated

### Scaling rule

The phase machine must stay small.

Rule:
- lifecycle progress belongs in `phase`
- destination identity belongs in `target`
- behavior meaning belongs in `kind`

This keeps the state machine stable even as more location types are added.

Phase semantics:
- `Ready` = no active route or on-target sustain step; eligible for idle choice
- `Transit` = moving toward the current target
- `Active` = performing sustained work or recovery at the target
- `Holding` = at the target, but waiting on an external condition such as patrol wait time, squad hold, or mine harvest turn

Energy rule:
- all non-restful activities continue to drain energy while in `Transit`, `Active`, or `Holding`
- `Rest + Active` and `Heal + Active` are the recovery states
- work and mine phases must have an explicit tired exit; they may not remain in-place indefinitely once energy is below the tired threshold

### Valid combinations

Not every `kind` may pair with every `phase`. Invalid combinations are bugs.

| Kind | Allowed phases |
|---|---|
| `Idle` | `Ready` |
| `Rest` | `Transit`, `Active` |
| `Heal` | `Transit`, `Active` |
| `Patrol` | `Transit`, `Holding` |
| `SquadAttack` | `Transit`, `Holding` |
| `Work` | `Transit`, `Active` |
| `Mine` | `Transit`, `Holding`, `Active` |
| `Raid` | `Transit`, `Active` |
| `ReturnLoot` | `Transit` |
| `Wander` | `Transit` |

Implementation rule:
- transitions may only move to a valid `(kind, phase)` pair
- debug output should make invalid pairs obvious if they ever occur

### Sensor vs state

`NpcFlags::at_destination` stays in the codebase, but changes role:
- It is a movement sensor produced by `gpu_position_readback`.
- It is not a behavior state.
- Only the behavior controller may translate it into `Activity.phase` transitions.

This is the key design rule. `at_destination` may say "movement target reached", but only `Activity.phase` may say "sleeping", "guarding", or "working".

### Player overrides and effect flags

These must stay separate from the activity lifecycle:

- `NpcFlags.direct_control` is a player-authority override, not an `Activity` state
- while `direct_control` is set, autonomous behavior reconcile, idle choice, and squad-driven retargeting are suspended
- during direct control, behavior may still perform narrow invariant maintenance such as releasing a mine queue spot if the unit is dragged out of range
- `ManualTarget` is not `ActivityTarget`
- `ManualTarget` is owned by player input / combat targeting and must remain a separate overlay
- the behavior controller must not translate `ManualTarget` into a new autonomous activity lifecycle
- `NpcFlags.healing` is an effect/status flag owned by `healing_system`, not by the behavior controller
- `Heal + ...` activity means the NPC is seeking or holding at a healing destination
- `flags.healing` means the NPC is currently inside a healing zone and below its healing cap
- render/debug healing visuals should continue to follow `flags.healing`, not merely `ActivityKind::Heal`

## Lifecycle model

Expected canonical flows:

| Intent | Phase flow |
|---|---|
| `Idle` | `Ready` |
| `Rest` | `Transit -> Active -> Idle/Ready` |
| `Heal` | `Transit -> Active -> Idle/Ready` |
| `Patrol` | `Transit -> Holding -> Transit` |
| `SquadAttack` | `Transit -> Holding` |
| `Work` | `Transit -> Active -> ReturnLoot/Transit`, with tired interrupt -> `Rest/Home/Transit` if home valid, otherwise `Rest/Fountain/Transit` if town center exists, otherwise `Idle/Ready` |
| `Mine` | `Transit -> Holding -> ReturnLoot/Transit`, with tired interrupt -> `Rest/Home/Transit` if home valid, otherwise `Rest/Fountain/Transit` if town center exists, otherwise `Idle/Ready` |
| `Raid` | `Transit -> Active -> ReturnLoot/Transit` |
| `ReturnLoot` | `Transit -> Idle/Ready`, with wrong-place arrival -> `Transit` redirect back to home |
| `Wander` | `Transit -> Idle/Ready` |

Examples:
- `Rest + Home + Transit` + arrived-home event -> `Rest + Home + Active`
- `Rest + Home + Active` + energy recovered -> `Idle + None + Ready`
- `Heal + Fountain + Transit` + arrived-fountain event -> `Heal + Fountain + Active`
- `Patrol + PatrolPost{...} + Transit` + arrived-post event -> `Patrol + PatrolPost{...} + Holding`
- `Patrol + PatrolPost{...} + Holding` + wait elapsed -> `Patrol + PatrolPost{next} + Transit`
- `Work + Worksite + Transit` + arrived-worksite event -> `Work + Worksite + Active`
- `Work + Worksite + Active` + tired event -> release worksite, then `Rest + Home + Transit` if home valid, otherwise `Rest + Fountain + Transit` if town center exists, otherwise `Idle + None + Ready`
- `Work + Worksite + Transit` + claimed farm invalid/taken event -> remain `Work + Worksite + Transit` with a new claimed farm target via resolver; if no replacement exists -> `Idle + None + Ready`
- `Mine + Worksite + Transit` + arrived-mine event -> `Mine + Worksite + Holding`
- `Mine + Worksite + Holding` + ready-and-my-turn event -> `ReturnLoot + Dropoff + Transit`
- `ReturnLoot + Dropoff + Transit` + arrived-wrong-place event -> remain `ReturnLoot + Dropoff + Transit` and re-submit path to home

Idle rendezvous rule:
- `Idle` is not a hidden transit state anymore
- `Idle + None + Ready` is the only state from which the idle chooser may run
- any activity that completes and wants a fresh choice must transition through `Idle + None + Ready` first
- direct activity-to-activity transitions are only for explicit interrupts or deterministic handoffs such as `Work -> ReturnLoot` or `Work -> Rest`

## Single controller

Keep one authoritative behavior controller.

The goal is not three separate Bevy systems. The goal is one decision system that is easier to reason about because it has explicit internal stages instead of ad hoc branch-order side effects.

Preferred shape:

1. `sense_facts(...)`
   - Derive transient facts from ECS and GPU readback.
   - Examples: `arrived_goal`, `left_goal_radius`, `energy_recovered`, `energy_tired`, `hp_recovered`, `worksite_invalid`, `worksite_ready`, `loot_delivered`, `patrol_wait_elapsed`.

2. `reconcile_activity(...)`
   - Pure transition logic.
   - Input: current `Activity`, `CombatState`, transient facts, policies, squad state, job, home, worksite state.
   - Output: next `Activity` and transition reason.
   - This stage chooses state. It does not own GPU writes or worksite claims directly.

3. `apply_activity_actions(...)`
   - Side effects on transition or sustain.
   - Submit movement intents.
   - Emit `WorkIntentMsg`.
   - Mark visuals dirty.
   - Push NPC log entries.

This keeps the controller explicit: observe, reconcile, act, while still preserving a single decision owner.

Implementation rule:
- prefer one bucket-gated `decision_system`
- implement the three stages as helper functions or tightly-scoped blocks inside that system
- do not split them into separate ECS systems unless profiling later shows a clear need
- the hard requirement is separation of logic and authority, not more schedules or more queries

### How the current priority ladder maps into the new controller

The current `decision_system` is a long priority-ordered branch chain. The refactor should not preserve that exact structure.

Instead, collapse it into three ordered layers inside `reconcile_activity(...)`:

1. global interrupts and overrides
   - ordered checks that can preempt normal activity progression
   - examples: combat flee/leash handling, forced `ReturnLoot`, invalid worksite, tired exit from work if treated as an interrupt
   - this is the only place where coarse "priority ordering" should remain

2. state-local lifecycle transitions
   - a `match (activity.kind, activity.phase, activity.target)` dispatch
   - each valid state owns its own arrival, wake, hold, and completion transitions
   - this replaces the generic arrival-first branch ordering that currently traps states like `Rest`

3. idle selection
   - only runs for `Idle + Ready`
   - performs weighted-random choice among valid idle actions

The key rule is:
- keep ordering only for true cross-cutting interrupts
- do not keep a generic "Priority 0: arrived => handle activity" branch ahead of all state logic
- arrival, recovery, hold-time, and completion rules belong to the specific lifecycle state that cares about them

Expected mapping from the current code:
- old `Priority 0: AtDestination -> Handle arrival transition`
  - remove as a generic top-level branch
  - rewrite as per-state transitions such as:
    - `Rest + Home + Transit` + arrived -> `Rest + Home + Active`
    - `Heal + Fountain + Transit` + arrived -> `Heal + Fountain + Active`
    - `Patrol + PatrolPost + Transit` + arrived -> `Patrol + PatrolPost + Holding`
    - `Work + Worksite + Transit` + arrived -> `Work + Worksite + Active`

- old `Priority 1-3: Combat`
  - stay as coarse interrupt ordering ahead of normal activity progression
  - combat remains orthogonal, but flee/leash/skip logic may still short-circuit the activity reconcile pass

- old `Priority 4a/4b: healing/rest wake`
  - move into state-local transitions:
    - `Heal + Fountain + Active` + hp recovered -> `Idle + None + Ready`
    - `Rest + Home + Active` + energy recovered -> `Idle + None + Ready`

- old `Priority 4c: loot threshold`
  - keep as a coarse interrupt if it is meant to override multiple activities
  - otherwise move it into the specific states that can accumulate loot

- old `Priority 5: work/mine/tired`
  - move into `Work` and `Mine` lifecycle arms
  - if tired exit is intended to preempt all other worksite behavior, it may stay in the global interrupt layer for those states only

- old `Priority 6: patrol wait / on-duty progression`
  - move into the `Patrol + Holding` lifecycle arm

- old `Idle scoring`
  - becomes the `Idle + Ready` chooser, and nowhere else

Implementation guidance:
- `reconcile_activity(...)` should read like a small ordered dispatcher, not a second 2000-line priority chain
- the top-level should first evaluate cross-cutting interrupts, then dispatch to a state-local match, then fall back to idle choice
- if a rule can be expressed as "when in this exact `(kind, phase, target)` state, transition on these facts", it belongs in the state-local match, not in the interrupt ladder

### `ticks_waiting` ownership

`ticks_waiting` is phase-local timing state, not a second behavior controller.

Implementation rule:
- `ticks_waiting` reset remains part of behavior transition helpers
- if a separate `on_duty_tick_system` is kept, it must be a narrow timer feeder only
- that timer system may increment `ticks_waiting` only for `Patrol + PatrolPost{...} + Holding`
- it must never mutate `Activity.kind`, `Activity.phase`, or `Activity.target`
- schedule it before the bucket-gated `decision_system` so a same-frame transition into `Holding` resets to `0` and begins counting on the following frame

This keeps one decision owner while still allowing per-frame patrol wait timing.

## Determinism rules

Determinism is required for tests, debugging, and BRP inspection.

Determinism boundary:
- activity lifecycle and phase transitions must be deterministic once an activity has been chosen
- idle choice may remain weighted-random by design
- randomness belongs only in the chooser, not in lifecycle progression or controller side effects

Rules:
- Keep weighted-random selection for the idle chooser. This is intentional and part of NPC personality.
- Randomness must be limited to choosing among valid idle actions such as `Eat`, `Rest`, `Work`, and `Wander`.
- Once an action is chosen, the resulting lifecycle (`Transit`, `Active`, `Holding`, completion) must be deterministic.
- Use a deterministic pseudo-random source so runs are reproducible from the same game state and tick history.
- Transition guards must depend only on stable inputs for that frame.
- Side effects happen after transition selection, not during scoring.

Weighted-random idle chooser:
- Compute candidate scores for `Eat`, `Rest`, `Work`, `Wander`.
- Use weighted random across those scores so NPCs do not feel mechanically identical.
- Prefer a deterministic pseudo-random roll keyed from stable simulation inputs such as slot plus decision frame, rather than a non-replayable external RNG.
- If all candidate scores are zero, fall back to a stable default action.

`Eat` semantics:
- `Eat` is an idle-side effect, not a behavior lifecycle.
- It does not create `ActivityKind::Eat`.
- It does not create a new phase.
- It immediately consumes food from town storage and restores energy at the NPC's current location.
- After eating, the NPC remains in `Idle + Ready` and the chooser may run again on the next normal decision cadence.

## Ownership rules

These are hard invariants.

- Only the behavior controller mutates `Activity.kind` or `Activity.phase`.
- Spawn and save/load code may initialize `Activity`, but normal runtime transitions still belong to the behavior controller.
- Movement systems own pathing and target arrival sensing only.
- `resolve_movement_system` remains the sole writer of `GpuUpdate::SetTarget`.
- `resolve_work_targets` remains the sole mutator of worksite claim/release state.
- `CombatState` remains orthogonal to `Activity`.
- `Activity.phase` replaces `at_destination` as the source of truth for "moving vs arrived vs active".

### Population accounting

`PopulationStats` is incrementally updated and must stay correct through the refactor.

Implementation rules:
- preserve `working` count semantics for the HUD and economy systems
- treat `working` as membership in a working lifecycle, not merely "currently standing on a worksite"
- transitions from a non-working activity into a working activity must increment `pop_stats.working` exactly once
- transitions from a working activity into a non-working activity must decrement `pop_stats.working` exactly once
- death cleanup must still decrement `working` if the NPC dies while in a working activity
- implement this in central transition/apply helpers using the old-state/new-state `is_working` flag, not with scattered ad hoc calls

### Explicit ownership exceptions

Not all behavior-adjacent state belongs to `Activity`.

Ownership rules:
- `Activity.kind`, `Activity.phase`, and `Activity.target` belong to the behavior controller
- `NpcFlags.healing` belongs to `healing_system`
- `NpcFlags.direct_control` and `ManualTarget` belong to player input / command systems
- `CombatState` belongs to combat systems
- `NpcFlags.at_destination` belongs to movement sensing

The refactor must keep those boundaries explicit instead of absorbing every flag into `Activity`.

### Enforcement strategy

This refactor needs active enforcement, not just good intentions.

Implementation rules:
- funnel runtime activity transitions through a small set of controller helpers such as `transition_to(...)` / `set_activity(...)`
- do not allow ad hoc `activity.kind = ...`, `activity.phase = ...`, or `activity.target = ...` writes to spread across unrelated systems
- keep a short whitelist of allowed writers:
  - behavior controller for activity transitions
  - spawn/load initialization for initial state
  - player input/combat systems for `ManualTarget` and `direct_control`
  - healing system for `flags.healing`
  - movement system for `flags.at_destination`
- add debug validation after controller execution so invalid `(kind, phase, target)` combinations fail loudly in debug builds
- use code search during implementation/review to catch new direct writes outside the allowed systems

## Refactor rules

The refactor must preserve controller boundaries, but it does not need to preserve old saves.

### Save/load

Old saves may break. That is intentional.

Implementation rules:
- bump the save version when the new `Activity { kind, phase, target, ... }` model lands
- reject or invalidate old saves instead of adding compatibility shims
- delete legacy `ActivitySave` mapping code rather than teaching it how to approximate the new phase model
- serialize the new activity shape directly once the refactor lands
- prefer a clean load path for the new model over a dual-format loader
- remove `Activity.worksite` instead of carrying it as migration baggage
- remove `Activity.target_pos` when its active `Mine`/`Raid` paths have been replaced by `ActivityTarget`, not before
- do not keep the old branch-order machine alive beside the new phase model
- do not build adapter layers whose only purpose is to let old and new behavior representations coexist

The goal is to make the new behavior model correct and simple, not to preserve a lossy translation layer from the old branch-order state machine.

For work and mine activities specifically:
- `Activity.target = Worksite` only means "this lifecycle is bound to the claimed worksite path"
- the actual worksite entity and position are resolved from `NpcWorkState.worksite`
- if no claim exists, behavior must request one through `resolve_work_targets` rather than inventing target identity inside `Activity`

### Existing authority boundaries

These do not change:
- movement still owns pathfinding and arrival sensing
- work targeting still owns worksite claim/release
- combat still owns `CombatState`
- behavior still owns activity transitions

The refactor changes how behavior state is represented, not who owns adjacent systems.

### Rendering and debug

Rendering and debug must not reintroduce implicit behavior state.

Implementation rules:
- visuals that currently depend on `at_destination` for behavior meaning must be rewritten to depend on `Activity.kind + Activity.phase`
- BRP/debug output must report explicit behavior state from `kind + phase + target`, not infer it from movement flags
- `NpcFlags::at_destination` may remain a movement sensor, but render/debug code must not treat it as "sleeping", "working", or "guarding"

## Migration order

Do this in small slices. Do not try to migrate every activity in one patch.

Implementation rule:
- keep the first slice minimal and proving-oriented
- add only the helpers, target variants, and transition structure needed to make `Rest` and `Heal` correct
- do not overbuild abstractions for future activities before Slice 1 is stable

### Slice 1: Rest and Heal

Reason: smallest lifecycle, already exposes a real bug.

Target end state:
- `Rest + Home + Transit -> Rest + Home + Active -> Idle + None + Ready`
- `Heal + Fountain + Transit -> Heal + Fountain + Active -> Idle + None + Ready`

Required outcomes:
- Resting NPCs never get trapped by generic arrival logic.
- Healing NPCs never get trapped by generic arrival logic.
- Wake-up checks run from explicit `Active` recovery phases, not from `at_destination`.
- Remove `Activity.worksite` as dead state; keep worksite authority solely on `NpcWorkState.worksite`.
- Keep Slice 1 small enough that `Rest` and `Heal` are fixed before broader activity migration begins.

### Slice 2: Patrol and SquadAttack

Target end state:
- `Patrol + PatrolPost{...} + Transit -> Patrol + PatrolPost{...} + Holding`
- `Patrol + PatrolPost{...} + Holding -> Patrol + PatrolPost{next} + Transit`
- `SquadAttack + SquadPoint(...) + Transit -> SquadAttack + SquadPoint(...) + Holding`

Required outcomes:
- Guard-post waiting is explicit.
- Squad target holding is explicit.
- Patrol progression no longer depends on `Patrol + at_destination`.

### Slice 3: Work, Mine, ReturnLoot, Raid

Target end state:
- work uses explicit `Transit/Active` phases with target data
- mine uses explicit `Transit/Holding` phases for the current queued-at-mine / tending state (`MiningAtMine`)
- reserve `Active` for mine only if mining later grows a true sustained on-target work step
- raid uses explicit `Transit/Active` phases with target data
- `ReturnLoot` always means "moving to delivery target", never "maybe already delivered"

Required outcomes:
- worksite validation becomes phase-aware
- harvest and delivery are explicit transitions
- drift, queue loss, or invalid worksite become explicit phase changes
- tired work and mine exits are explicit transitions, not implicit fallthrough
- `ReturnLoot` wrong-place arrival re-enters the same transit phase and re-targets home instead of incorrectly completing
- replace remaining live `Activity.target_pos` paths for `Mine` and `Raid`, then delete `target_pos`

### Slice 4: Idle chooser and debug

Keep weighted-random idle action choice, but isolate it to the idle chooser only.

Do not convert `Eat` into an activity state during this refactor. Keep it as an inline idle action with immediate effect.

Expose in BRP:
- `activity_kind`
- `activity_phase`
- `activity_target`
- `transition_reason`
- `last_transition_tick`

## File-level impact

Primary files:
- `rust/src/components.rs`
- `rust/src/constants/npcs.rs`
- `rust/src/systems/behavior.rs`
- `rust/src/systems/movement.rs`
- `rust/src/systems/work_targeting.rs`
- `rust/src/entity_map.rs`
- `rust/src/systems/health.rs`
- `rust/src/render.rs`
- `rust/src/systems/spawn.rs`
- `rust/src/save.rs`
- `rust/src/systems/remote.rs`
- `rust/src/tests/archer_patrol.rs`
- `rust/src/tests/farmer_cycle.rs`
- `rust/src/tests/movement.rs`
- `rust/src/tests/miner_cycle.rs`
- `rust/src/tests/healing.rs`
- `docs/behavior.md`

Expected code changes:
- add `ActivityPhase`
- add `ActivityTarget`
- refactor transition logic to match on `(kind, phase, target)` instead of `kind + at_destination`
- move ad hoc `activity.kind = ...` writes behind transition helpers
- audit all non-behavior direct `Activity` writes and either convert them to transition helpers or leave them as explicit initialization-only code paths
- keep weighted-random idle action selection, but make the random source explicit and isolated to the chooser
- keep `Eat` as an inline chooser action, not a new `ActivityKind`
- expose phase-aware debug state

## Testing requirements

Unit tests:
- resting NPC at destination wakes correctly
- healing NPC at destination wakes correctly
- arrival sensing does not trap `Rest` or `Heal`
- patrol arrival becomes `Holding`, not generic "arrival handled"

Scenario tests:
- `archer-patrol` asserts `Transit -> Holding -> Transit`
- `movement` asserts explicit move/active phases
- `farmer-cycle` asserts `Transit -> Active -> ReturnLoot/Transit` plus tired exit from work
- `miner-cycle` asserts `Transit -> Holding -> ReturnLoot/Transit`
- `healing` asserts `Transit -> Active -> Ready`

Test runner guidance:
- pure unit tests in `behavior.rs` still run via `cargo test`
- integration behavior tests in `rust/src/tests/` run through the in-app harness, not plain `cargo test <name>`
- use CLI mode for those: `cargo run -- --test archer-patrol`, `cargo run -- --test movement`, `cargo run -- --test healing`

BRP verification:
- inspect one resting NPC and one patrolling NPC
- verify `kind`, `phase`, and `target` are all visible and coherent

## Done when

- `Activity.kind` answers "what is this NPC trying to do?"
- `Activity.phase` answers "where is it in that lifecycle?"
- `Activity.target` answers "where or what is this activity operating on?"
- no behavior depends on branch order to distinguish moving from active
- `Rest` and `Heal` cannot loop forever on arrival
- idle choice remains weighted-random by design, while lifecycle progression remains deterministic
- BRP/debug output shows `kind + phase + target + reason`
- [behavior.md](behavior.md) is updated to document the new model once the refactor lands

## Cargo test expectation

This refactor must be validated by `cargo test` as much as possible, not only by the in-app scenario harness.

Implementation rules:
- extract transition logic into helper functions that can be unit-tested without booting the whole game
- prefer pure or near-pure tests for `sense_facts(...)`, `reconcile_activity(...)`, transition helpers, and validity checks
- use the in-app test harness only for end-to-end behavior that truly depends on full ECS scheduling, movement, or GPU readback
- do not leave core lifecycle correctness validated only by manual playtesting or BRP inspection

Minimum `cargo test` coverage expected from this refactor:
- `Rest + Transit` arrives -> `Rest + Active`
- `Rest + Active` wakes -> `Idle + None + Ready`
- `Heal + Transit` arrives -> `Heal + Active`
- `Heal + Active` wakes -> `Idle + None + Ready`
- generic arrival sensing does not trap `Rest` or `Heal`
- `Patrol + Transit` arrives -> `Patrol + Holding`
- `Patrol + Holding` wait elapsed -> next patrol target + `Transit`
- `Work + Transit` arrives -> `Work + Active`
- farmer retarget on taken/invalid farm remains in `Work + Transit` and re-requests claim, rather than falling through to arbitrary idle behavior
- `Mine + Transit` arrives -> `Mine + Holding`
- `Mine + Holding` ready-and-my-turn -> `ReturnLoot + Dropoff + Transit`
- tired exits from `Work` and `Mine` happen exactly once and preserve worksite/accounting invariants
- `ReturnLoot` wrong-place arrival reissues transit instead of completing
- `Idle + None + Ready` is the only state that runs the idle chooser
- weighted-random chooser remains isolated to idle choice and does not affect lifecycle transitions
- `DirectControl` suppresses autonomous transitions while still allowing narrow invariant maintenance
- `ManualTarget` does not get absorbed into `Activity`
- `NpcFlags.healing` remains owned by healing logic and is not inferred from `ActivityKind::Heal`
- `ticks_waiting` resets on phase/target change and only increments in the allowed patrol holding state
- `pop_stats.working` increments and decrements exactly once across working lifecycle boundaries
- invalid `(kind, phase, target)` combinations fail loudly in debug-oriented tests

Acceptance rule:
- the refactor is not complete until the new unit tests pass under `cargo test`
- scenario harness tests remain required, but they are supplemental proof, not the primary safety net
