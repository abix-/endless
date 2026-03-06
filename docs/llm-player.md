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

Action format (LLM → game): one action per line, `method key:value key:value ...`
```
build kind:Farm row:-5 col:0
policy eat_food:true recovery_hp:0.5
squad_target squad:0 x:5000 y:8000
```

Serialized via `serde_toon2` crate in Rust (derives `Serialize`).

## Built-in Player (recommended)

The game has a built-in LLM player that spawns `claude --print` directly — no HTTP server, no Python scripts.

### Setup

1. In the main menu, add AI player slots (Builder or Raider)
2. Check the **LLM** checkbox on the slot you want the model to control
3. Click **Play**

The built-in player reads ECS resources directly and sends game state as TOON to Claude via stdin piping. It runs every 20 seconds.

### Architecture

`systems/llm_player.rs`:
- `LlmPlayerState` resource: timer, async receiver, subscriptions, pending queries
- `LlmReadState` SystemParam: read-only ECS access (WorldData, GameTime, FactionStats, PopulationStats, TownUpgrades, Reputation)
- `LlmWriteState` SystemParam: write access (SquadState, FoodStorage, GoldStorage)
- `build_state_json()`: builds `serde_json::Value` from ECS, serialized to TOON via `serde_toon2::to_string()` — own town gets full building list, enemy towns get counts
- `parse_actions()`: parses TOON action lines (`method key:value ...`) with auto-typed values (bool/int/float/string)
- `execute_actions()`: routes parsed actions to ECS mutations

Process spawn uses `.env_remove("CLAUDECODE")` to avoid nested-session detection, `Stdio::piped()` for stdin, and `CREATE_NO_WINDOW` on Windows to prevent console focus stealing.

### Prompt

`llm-player/prompt_builtin.md` is the system prompt. It documents:
- TOON state format (towns with distance, reputation, buildings, squads)
- TOON action format: `method key:value ...` one per line, `NONE` for no-op
- Data topics (npcs, combat_log, upgrades, policies) via `subscribe topics:npcs,upgrades`
- Strategy phases (expand → upgrades → attack)

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
