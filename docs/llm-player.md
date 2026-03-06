# LLM Player Guide

How to run an AI model as a player in Endless.

## Wire Format: TOON

All data between the game and LLM uses **TOON** (Token-Oriented Object Notation) — a compact format that saves 30-60% tokens vs JSON.

- Flat data: `key: value` pairs, one per line
- Arrays: `field[N]:` header + indented CSV rows
- Maps: `field:` header + indented `key: value` pairs

State example:
```
day: 4
hour: 7
food: 27
factions[2]:
  0,30,0,3
  1,25,1,0
npcs:
  Archer: 8 (Patrolling:5 OnDuty:3)
```

Action format (LLM → game): TOON `actions[N]:` array of objects with `method` field plus params.
```
actions[3]:
  - method: build
    kind: Farm
    row: -5
    col: 0
  - method: policy
    eat_food: true
    recovery_hp: 0.5
  - method: squad_target
    squad: 0
    x: 5000
    y: 8000
```

Serialized via `serde_toon2` crate in Rust — `serde_toon2::from_str` parses responses directly.

## Built-in Player (recommended)

The game has a built-in LLM player that spawns `claude --print` directly — no HTTP server, no Python scripts.

### Setup

1. In the main menu, add AI player slots (Builder or Raider)
2. Check the **LLM** checkbox on the slot you want the model to control
3. Click **Play**

The built-in player reads ECS resources directly and sends game state as TOON to Claude via stdin piping. Default cycle is 20 seconds, adjustable via LLM Player settings tab (5-120s).

### Architecture

`systems/llm_player.rs`:
- `LlmPlayerState` resource: timer, async receiver, subscriptions, pending queries, `last_command`/`last_payload`/`last_response` for settings panel inspector, `LlmStatus` enum (Idle/Sending/Thinking/Done)
- `LlmReadState` SystemParam: read-only ECS access (WorldData, GameTime, FactionStats, PopulationStats, TownUpgrades, Reputation)
- `LlmWriteState` SystemParam: write access (SquadState, FoodStorage, GoldStorage, ChatInbox)
- `build_state_json()`: builds `serde_json::Value` from ECS, serialized to TOON via `serde_toon2::to_string()` — own town gets full building list, enemy towns get counts
- `parse_actions()`: parses TOON `actions[N]:` array via `serde_toon2::from_str` — each action object has `method` field plus params
- `execute_actions()`: routes parsed actions to ECS mutations (build, destroy, upgrade, policy, squad_target, chat, query, subscribe, unsubscribe)

Process spawn uses `.env_remove("CLAUDECODE")` to avoid nested-session detection, `Stdio::piped()` for stdin, and `CREATE_NO_WINDOW` on Windows to prevent console focus stealing.

### Prompt

`llm-player/prompt_builtin.md` is the system prompt. It documents:
- TOON state format (towns with distance, reputation, buildings, squads)
- TOON action format: `actions[N]:` array with `- method: X` objects, `NONE` for no-op
- Actions: build, destroy, upgrade, policy, squad_target, chat, query, subscribe, unsubscribe
- Data topics (npcs, combat_log, upgrades, policies) via subscribe
- Strategy phases (expand → upgrades → attack → diplomacy)

### Settings Panel

The LLM Player tab in the pause menu settings provides:
- **Cycle interval slider** (5-120s, step 5) — synced live to timer duration
- **Last Command** — the `claude --print` CLI invocation (collapsible, copy button)
- **Last Payload** — full TOON state sent as stdin (collapsible, copy button)
- **Last Response** — raw LLM output (collapsible, copy button)

### HUD Status Indicator

A colored circle in the top bar shows LLM status:
- Gray: idle (timer counting down)
- Blue: sending state to Claude
- Yellow: waiting for response
- Green: executed actions (or no actions)

### Data Model

Three tiers of data delivery:
1. **Base state** (always sent): game_time, towns, factions, reputation
2. **Subscriptions** (persistent): topics included every cycle until unsubscribed
3. **One-shot queries**: topics included in next cycle only, then cleared

### Reputation

Per-town `reputation` field in base state shows how your faction feels about each town's faction. -50 means they killed ~50 of your NPCs. Backed by the 2D `Reputation` matrix in `resources.rs`.

## External Player (alternative)

### Claude Code

```
python llm-player/launch.py
```

Reads `llm-player/prompt.md` as system prompt. Uses BRP HTTP endpoints on localhost:15702. All responses return TOON format. Commands use TOON params:

```
python actions.py endless/summary
python actions.py endless/ai_manager town:1 active:true personality:Aggressive
python actions.py endless/squad_target squad:13 x:6944 y:11488
```

### Anthropic API

For unattended play. Requires API key from [console.anthropic.com](https://console.anthropic.com). Use `actions.py` tool definitions with TOON key:value params.

### Any other model

Anything that can POST JSON-RPC to `http://localhost:15702` works (BRP transport is still JSON-RPC, only the response payloads are TOON strings). See [brp.md](brp.md) for endpoint docs.

## Token Budget

**Built-in**: Uses Claude Code subscription. Haiku burns through it slowly.

**API**: ~$0.01-0.05 per hour at Haiku pricing.
