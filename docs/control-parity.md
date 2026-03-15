# Control Surface Parity

Current state of player vs AI capability across all four control surfaces.
Last updated: 2026-03-15.

## Control Surfaces

| Surface | Description | Source |
|---------|-------------|--------|
| **Player UI** | egui panels (Policies, Squads, building click, upgrade tree) | `ui/left_panel/mod.rs`, `ui/game_hud.rs` |
| **BRP API** | HTTP JSON-RPC endpoints for external LLM agents | `systems/remote.rs`, registered in `lib.rs` |
| **LLM Player** | In-game LLM that parses structured actions inline | `systems/llm_player.rs` |
| **Built-in AI** | Autonomous `ai_decision_system` + `ai_squad_commander_system` | `systems/ai_player/decision.rs`, `systems/ai_player/squad_commander.rs` |

## Building / Economy

| Capability | Player UI | BRP API | LLM Player | Built-in AI |
|---|---|---|---|---|
| Place building | click map | `endless/build` | `build` | `try_build_*` |
| Destroy building | click building | `endless/destroy` | `destroy` | -- |
| Purchase upgrade | upgrade panel | `endless/upgrade` | `upgrade` | `AiAction::Upgrade` |
| Wall upgrade | click wall | -- | -- | -- |
| Build roads | manual place | `endless/build` road | -- | `AiAction::BuildRoads` |
| Build waypoints | manual place | `endless/build` waypoint | -- | `AiAction::BuildWaypoint` |

## Policy

| Capability | Player UI | BRP API | LLM Player | Built-in AI |
|---|---|---|---|---|
| eat_food | checkbox | `endless/policy` | `policy` | personality hysteresis |
| archer_aggressive | checkbox | `endless/policy` | `policy` | personality default |
| archer_leash | checkbox | `endless/policy` | `policy` | personality default |
| farmer_fight_back | checkbox | `endless/policy` | `policy` | personality default |
| prioritize_healing | checkbox | `endless/policy` | `policy` | personality default |
| farmer_flee_hp | slider | `endless/policy` | `policy` | personality default |
| archer_flee_hp | slider | `endless/policy` | `policy` | personality default |
| recovery_hp | slider | `endless/policy` | `policy` | personality default |
| mining_radius | slider | `endless/policy` | `policy` | `ExpandMiningRadius` |
| reserve_food | slider | `endless/policy` | `policy` | reads internally |
| reserve_gold | slider | `endless/policy` | `policy` | reads internally |
| archer_schedule | dropdown | `endless/policy` | `policy` | -- |
| archer_off_duty | dropdown | `endless/policy` | `policy` | -- |
| farmer_schedule | dropdown | `endless/policy` | `policy` | -- |
| farmer_off_duty | dropdown | `endless/policy` | `policy` | -- |
| loot_threshold (per-squad) | slider (squad tab) | `endless/squad` | `squad` | personality default |

## Squad

| Capability | Player UI | BRP API | LLM Player | Built-in AI |
|---|---|---|---|---|
| Set squad target | click map | `endless/squad_target` | `squad_target` | `ai_squad_commander` |
| Clear squad target | button | `endless/squad_target` (omit x/y) | `squad_target` (omit x/y) | `ai_squad_commander` |
| patrol_enabled | checkbox | `endless/squad` | `squad` | sets internally |
| rest_when_tired | checkbox | `endless/squad` | `squad` | sets internally |
| hold_fire | checkbox | `endless/squad` | `squad` | intentionally off |
| loot_threshold | slider | `endless/squad` | `squad` | personality default |
| Recruit to squad | per-job buttons | `endless/squad_recruit` | -- | auto-assigns |
| Dismiss from squad | button | `endless/squad_dismiss` | -- | -- |
| Direct control (box-select) | mouse select | -- | -- | -- |

## Meta / AI Manager

| Capability | Player UI | BRP API | LLM Player | Built-in AI |
|---|---|---|---|---|
| Pause / speed | buttons | `endless/time` | -- | -- |
| AI manager toggle | checkbox | `endless/ai_manager` | -- | N/A |
| AI personality | dropdown | `endless/ai_manager` | -- | N/A |
| AI road style | dropdown | `endless/ai_manager` | -- | N/A |
| Chat between towns | -- | `endless/chat` | `chat` | -- |

## Gaps

Capabilities the player has that AI surfaces are missing.

### BRP / LLM Player

1. ~~**Schedule controls**~~ -- closed: `endless/policy` + LLM `policy` action
2. ~~**Reserve food/gold**~~ -- closed: `endless/policy` + LLM `policy` action
3. ~~**Squad settings**~~ -- closed: `endless/squad` + LLM `squad` action
4. ~~**Squad recruit/dismiss**~~ -- closed: `endless/squad_recruit` + `endless/squad_dismiss` (BRP only)
5. ~~**Clear squad target**~~ -- closed: omit x/y in `endless/squad_target` or LLM `squad_target`
6. **Wall upgrades** -- no endpoint to upgrade walls
7. **Mine enable/disable** -- no endpoint to toggle individual mines

### Built-in AI

1. **Destroy buildings** -- cannot demolish own buildings
2. **Schedule/off-duty tuning** -- does not change archer/farmer schedules or off-duty behavior
3. **Wall upgrades** -- does not upgrade walls

### Remaining Gaps

Lower priority:

- Wall upgrades (marginal defensive value vs effort)
- Mine enable/disable (niche optimization)
- Direct control (intentionally player-only)
- Squad recruit/dismiss for LLM player (BRP-only for now)
