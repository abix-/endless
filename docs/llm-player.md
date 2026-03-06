# LLM Player Guide

How to run an AI model as a player in Endless.

## Built-in Player (recommended)

The game has a built-in LLM player that spawns `claude --print` directly — no HTTP server, no Python scripts.

### Setup

1. In the main menu, add AI player slots (Builder or Raider)
2. Check the **LLM** checkbox on the slot you want the model to control
3. Click **Play**

The built-in player reads ECS resources directly and sends game state as a JSON user message to Claude via stdin piping. It runs every 20 seconds.

### Architecture

`systems/llm_player.rs`:
- `LlmPlayerState` resource: timer, async receiver, subscriptions, pending queries
- `LlmReadState` SystemParam: read-only ECS access (WorldData, GameTime, FactionStats, PopulationStats, TownUpgrades, Reputation)
- `LlmWriteState` SystemParam: write access (SquadState, FoodStorage, GoldStorage)
- `build_state_json()`: serializes game state to JSON — own town gets full building list `[{kind, row, col}]`, enemy towns get counts `{"Farm": 3}`
- `execute_actions()`: parses LLM JSON response into build/destroy/upgrade/policy/squad_target/query/subscribe/unsubscribe actions

Process spawn uses `.env_remove("CLAUDECODE")` to avoid nested-session detection, `Stdio::piped()` for stdin, and `CREATE_NO_WINDOW` on Windows to prevent console focus stealing.

### Prompt

`llm-player/prompt_builtin.md` is the system prompt. It documents:
- Base state format (towns with distance, reputation, buildings, squads)
- All actions (policy, build, destroy, upgrade, squad_target, query, subscribe, unsubscribe)
- Data topics (npcs, combat_log, upgrades, policies)
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

Reads `llm-player/prompt.md` as system prompt. Uses BRP HTTP endpoints on localhost:15702.

### Anthropic API

For unattended play. Requires API key from [console.anthropic.com](https://console.anthropic.com).

```python
import anthropic, json, time, subprocess

system = open("llm-player/prompt.md").read()
client = anthropic.Anthropic()
messages = [{"role": "user", "content": "The game has started. Begin playing."}]

while True:
    response = client.messages.create(
        model="claude-haiku-4-5-20251001",
        max_tokens=1024,
        system=system,
        tools=[{
            "name": "curl",
            "description": "POST JSON-RPC to the game server",
            "input_schema": {
                "type": "object",
                "properties": {
                    "method": {"type": "string"},
                    "params": {"type": "object"}
                },
                "required": ["method", "params"]
            }
        }],
        messages=messages,
    )
    # Process tool calls → curl to localhost:15702 → feed results back
    time.sleep(15)
```

### Any other model

Anything that can POST JSON to `http://localhost:15702` works. See [brp.md](brp.md) for endpoint docs.

## Token Budget

**Built-in**: Uses Claude Code subscription. Haiku burns through it slowly.

**API**: ~$0.01-0.05 per hour at Haiku pricing.
