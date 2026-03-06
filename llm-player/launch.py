#!/usr/bin/env python3
"""Launch Claude Code as the LLM player."""

import os
import subprocess
import sys

script_dir = os.path.dirname(os.path.abspath(__file__))
prompt_path = os.path.join(script_dir, "prompt.md")

with open(prompt_path) as f:
    prompt = f.read()

subprocess.run([
    "claude",
    "--model", "claude-haiku-4-5-20251001",
    "--system-prompt", prompt,
    "--allowedTools", "Bash(python*) Read",
], cwd=script_dir)
