# LLM Player Guide

How to run an AI model (Claude, ChatGPT, etc.) as a player in Endless.

## Overview

The LLM player is just another opponent — it reads game state and takes actions through the same controls a human uses. The game's built-in AI Manager handles building placement, road layout, and combat pathing. The LLM makes high-level strategic decisions: personality, policies, squad targets, upgrade priorities.

## Setup

### 1. Configure the Game

In the main menu, set up AI player slots:

1. Click **+ Builder** to add an AI builder town
2. Check the **LLM** checkbox on that slot
3. Click **Play**

The LLM checkbox tells the game server to allow BRP write access for that town. Without it, the model can read game state but can't take any actions.

### 2. Find Your Town Index

After the game starts, call the summary endpoint to find which town is yours:

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/summary","params":{},"id":1}'
```

The response includes all towns with their index, name, faction, food, gold, and buildings. Your town is the one marked LLM in the lobby — typically index 1 (index 0 is the human player).

### 3. Run the Model

#### Option A: Claude Code (recommended for prototyping)

Run Claude Code with a system prompt that tells it to play the game. The model uses `curl` to interact.

```bash
claude --system-prompt "$(cat docs/llm-player-prompt.md)" \
  --allowedTools "Bash(curl*localhost:15702*)"
```

Or just start Claude Code in the repo directory and paste the prompt manually.

#### Option B: Anthropic API with tool use

```python
import anthropic
import json

client = anthropic.Anthropic()

tools = [{
    "name": "game_action",
    "description": "Send a JSON-RPC request to the Endless game server",
    "input_schema": {
        "type": "object",
        "properties": {
            "method": {"type": "string", "description": "JSON-RPC method name"},
            "params": {"type": "object", "description": "Method parameters"}
        },
        "required": ["method", "params"]
    }
}]

# System prompt with game rules + your town index
system = open("docs/llm-player-prompt.md").read()
system += "\n\nYou control town index 1."

messages = [{"role": "user", "content": "The game has started. Begin playing."}]

# Game loop
while True:
    response = client.messages.create(
        model="claude-haiku-4-5-20251001",  # cheap model is fine
        max_tokens=1024,
        system=system,
        tools=tools,
        messages=messages,
    )
    # Process tool calls → curl to localhost:15702 → feed results back
    # Sleep 10-30 seconds between think cycles
```

#### Option C: Any HTTP-capable AI framework

The game doesn't care what's calling the endpoints. OpenAI function calling, LangChain, a custom Python loop — anything that can POST JSON to `http://localhost:15702` works.

## System Prompt

The file `docs/llm-player-prompt.md` contains the full system prompt to give the model. Key sections:

1. **Role**: You are an AI opponent in a real-time kingdom builder
2. **Your town**: Which town index you control
3. **Game loop**: Poll → Assess → Decide → Act → Wait
4. **Available actions**: All BRP endpoints with parameters
5. **Strategy guide**: What to do in early/mid/late game

## Game Loop

The model should follow this cycle:

```
1. POLL    — Call endless/summary to get current game state
2. ASSESS  — Check food, gold, army size, enemy status
3. DECIDE  — Pick 0-3 actions based on the situation
4. ACT     — Execute actions (policy changes, upgrades, squad moves)
5. WAIT    — Sleep 10-30 seconds before next cycle
```

Most cycles result in zero actions — the AI Manager is handling routine building and upgrading. The model only intervenes when something strategic changes:
- Economy is struggling → change personality to Economic
- Under attack → redirect squads, adjust flee thresholds
- Stockpile of resources → enable specific upgrades
- Enemy is weak → send squads to attack

## Token Budget

At Haiku pricing (~$0.25/MTok input, ~$1.25/MTok output as of 2025):
- Each `endless/summary` response: ~500-2000 tokens depending on town count
- Each think cycle: ~200-500 output tokens
- At 1 cycle per 15 seconds: ~4 cycles/min × ~2500 tokens = ~10K tokens/min
- **Cost: roughly $0.01-0.05 per hour of gameplay**

A frontier model (Opus/GPT-4) would cost 10-50x more with no meaningful gameplay improvement.

## Actions Reference

### Read (unrestricted)

| Endpoint | Use |
|----------|-----|
| `endless/summary` | Full game overview (towns, NPCs, food, gold, factions) |
| `world.query` | Query specific ECS components (NPCs in combat, buildings, etc.) |
| `world.get_resources` | Read a single resource (GameTime, TownPolicies, etc.) |

### Write (LLM-marked towns only)

| Endpoint | Use |
|----------|-----|
| `endless/ai_manager` | Set personality, enable/disable AI Manager, toggle auto-build/upgrade |
| `endless/policy` | Set flee thresholds, archer aggression, healing priority |
| `endless/upgrade` | Queue a specific upgrade purchase |
| `endless/build` | Queue a specific building placement |
| `endless/squad_target` | Send a squad to attack a position |
| `endless/time` | Pause/unpause, set game speed (unrestricted) |

See [brp.md](brp.md) for full parameter documentation and curl examples.

## Strategy Tips for the Model

**Early game** (day 1-3):
- Enable AI Manager with Economic personality — let it build farms and homes
- Don't touch upgrades yet, let food stockpile grow
- Set `eat_food: true` so NPCs don't starve

**Mid game** (day 3-10):
- Switch to Balanced personality when you have 5+ farms
- Start buying Move Speed and Stamina upgrades
- Monitor enemy faction stats — if they're weak, switch to Aggressive

**Late game** (day 10+):
- Buy military upgrades (Damage, HP, Range for archers)
- Send squads to attack enemy towns
- React to raids — redirect squads defensively when under attack

**Common mistakes:**
- Acting too often — the AI Manager handles 90% of gameplay, let it work
- Ignoring food — starving NPCs can't fight or work
- Not upgrading — upgrades compound over time, start early
- Micromanaging buildings — the AI Manager's placement algorithm is good enough
