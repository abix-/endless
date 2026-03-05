$prompt = Get-Content -Path "$PSScriptRoot\prompt.md" -Raw
claude --model claude-haiku-4-5-20251001 --system-prompt $prompt --allowedTools "Bash(curl*localhost:15702*),Bash(python*actions.py*)"
