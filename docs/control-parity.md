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
| reserve_food | slider | -- | -- | reads internally |
| reserve_gold | slider | -- | -- | reads internally |
| archer_schedule | dropdown | -- | -- | -- |
| archer_off_duty | dropdown | -- | -- | -- |
| farmer_schedule | dropdown | -- | -- | -- |
| farmer_off_duty | dropdown | -- | -- | -- |
| loot_threshold (per-squad) | slider (squad tab) | readable via `endless/debug` | -- | personality default |

## Squad

| Capability | Player UI | BRP API | LLM Player | Built-in AI |
|---|---|---|---|---|
| Set squad target | click map | `endless/squad_target` | `squad_target` | `ai_squad_commander` |
| Clear squad target | button | -- | -- | `ai_squad_commander` |
| patrol_enabled | checkbox | -- | -- | sets internally |
| rest_when_tired | checkbox | -- | -- | sets internally |
| hold_fire | checkbox | -- | -- | intentionally off |
| loot_threshold | slider | -- | -- | personality default |
| Recruit to squad | per-job buttons | -- | -- | auto-assigns |
| Dismiss from squad | button | -- | -- | -- |
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

1. **Schedule controls** -- `archer_schedule`, `farmer_schedule`, `archer_off_duty`, `farmer_off_duty` not exposed
2. **Reserve food/gold** -- `reserve_food`, `reserve_gold` not exposed
3. **Squad settings** -- `patrol_enabled`, `rest_when_tired`, `hold_fire`, `loot_threshold` not settable (only readable via `endless/debug`)
4. **Squad recruit/dismiss** -- no action to move NPCs between squads
5. **Clear squad target** -- `endless/squad_target` can set but not clear (would need null target support)
6. **Wall upgrades** -- no endpoint to upgrade walls
7. **Mine enable/disable** -- no endpoint to toggle individual mines

### Built-in AI

1. **Destroy buildings** -- cannot demolish own buildings
2. **Schedule/off-duty tuning** -- does not change archer/farmer schedules or off-duty behavior
3. **Wall upgrades** -- does not upgrade walls

### By Impact

Highest impact for competitive AI parity:

- **Squad settings via BRP** (patrol, rest, hold_fire, loot_threshold, recruit/dismiss) -- military effectiveness
- **Schedule/reserve via BRP** (schedules, off-duty, reserve_food/gold) -- economic tuning
- **Schedule/off-duty tuning for built-in AI** -- still fixed at default policy values
- **Clear squad target via BRP** -- needed for retreat/disengage

Lower impact:

- Wall upgrades (marginal defensive value vs effort)
- Mine enable/disable (niche optimization)
- Direct control (intentionally player-only)
