# LLM Player Guide

How to run an AI model as a player in Endless.

## 1. Configure the Game

In the main menu, set up AI player slots:

1. Click **+ Builder** or **+ Raider** to add AI towns
2. Check the **LLM** checkbox on slots you want the model to control
3. Click **Play**

The LLM checkbox grants BRP write access for that town. Without it, the model can read game state but can't take actions.

## 2. Launch the Model

### Claude Code (recommended)

```bash
claude --model claude-haiku-4-5-20251001 \
  --system-prompt "$(cat docs/llm-player-prompt.md)" \
  --allowedTools "Bash(curl*localhost:15702*)"
```

- **`--model`** — Haiku is cheap and more than capable. Swap in `claude-sonnet-4-6` for stronger play.
- **`--system-prompt`** — Injects the game rules, endpoints, and strategy from `docs/llm-player-prompt.md`.
- **`--allowedTools`** — Restricts the model to only `curl` commands against the game server.

Uses your Claude Code subscription. No separate API account needed.

### Anthropic API (alternative)

For a fully autonomous, unattended player. Requires a separate API key from [console.anthropic.com](https://console.anthropic.com).

```python
import anthropic, json, time, subprocess

system = open("docs/llm-player-prompt.md").read()
system += "\n\nYou control town index 1."

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

The game doesn't care what's calling the endpoints. OpenAI, LangChain, a bash script — anything that can POST JSON to `http://localhost:15702` works. See [brp.md](brp.md) for endpoint docs.

## Token Budget

**Claude Code**: Uses your subscription budget. Haiku burns through it very slowly — a game session uses a fraction of what a coding session would.

**API**: ~$0.01-0.05 per hour at Haiku pricing. A frontier model costs 10-50x more with no meaningful gameplay improvement.
