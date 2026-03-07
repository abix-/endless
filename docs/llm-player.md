# LLM Player Guide

How to run an AI model as a player in Endless.

## Wire Format

**State (game → LLM)**: TOON (Token-Oriented Object Notation) via `serde_toon2` — compact line-oriented format, 30-60% fewer tokens than JSON.

**Actions (LLM → game)**: CSV lines — one action per line: `method, key:value, key:value, ...`
```
build, kind:Farm, row:1, col:0
policy, eat_food:true, prioritize_healing:true
subscribe, topics:npcs,upgrades
squad_target, squad:0, x:5000, y:8000
chat, to:2, message:good luck neighbor
```

Parsed by `parse_actions()` — splits on `, ` then `split_once(':')` for named key:value params. Special handling for `message:` (captures everything after, preserving commas).

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
- `build_state_json()`: builds `serde_json::Value` from ECS, serialized to TOON via `serde_toon2::to_string()` — flat per-town fields (compact building strings, squad counts, distance, reputation) for tabular TOON format + top-level own-town extras (your_squads, open_slots, inbox). Inbox uses flag-based delivery: only unsent messages (`!sent_to_llm`) are included, then marked `sent_to_llm = true`.
- `parse_actions()`: parses CSV lines — splits on `, ` then `split_once(':')` for named key:value params
- `execute_actions()`: routes parsed actions to ECS mutations (build, destroy, upgrade, policy, squad_target, chat, query, subscribe, unsubscribe). Combat log entries use `push_at` with world positions for camera-pan buttons. Chat action writes to `ChatInbox` only (not combat log) and marks the original player message as `has_reply = true`.

Process spawn uses `.env_remove("CLAUDECODE")` to avoid nested-session detection, `Stdio::piped()` for stdin, and `CREATE_NO_WINDOW` on Windows to prevent console focus stealing.

### Prompt

`llm-player/prompt_builtin.md` is the system prompt. It documents:
- TOON state format (flat per-town fields: i, name, faction, cx, cy, dist, rep, food, gold, buildings, squads + top-level your_squads, open_slots, inbox)
- CSV action format: `method, key:value, key:value, ...` one per line, `NONE` for no-op
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

### Chat System

`ChatInbox` (resources.rs) is the single source of truth for player ↔ LLM messaging. `VecDeque<ChatMessage>` ring buffer (200 cap, oldest evicted). Each message has flags:
- `sent_to_llm`: set to `true` after message is included in an LLM state payload (prevents re-sending)
- `has_reply`: set to `true` when the LLM responds to a player message

Player sends chat via the combat log input box → pushed to `ChatInbox` only (no combat log write). LLM sends chat via `chat` action → pushed to `ChatInbox` only. The combat log UI reads `ChatInbox` directly and renders entries as `[chat to Town]` / `[chat from Town]` under the Chat filter.

### Reputation

Per-town `rep` field in base state shows how your faction feels about each town's faction. -50 means they killed ~50 of your NPCs. Backed by the 2D `Reputation` matrix in `resources.rs`.

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
