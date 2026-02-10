# Endless

Colony sim: pure Bevy 0.18 ECS with GPU compute for 16K NPCs via instanced rendering.

- When writing Rust for Endless, read `~/.claude/skills/bevy.md` first.
- When writing WGSL shaders, read `~/.claude/skills/wgsl.md` first.
- [docs/README.md](docs/README.md) - architecture, file map, patterns
- [docs/roadmap.md](docs/roadmap.md) - feature tracking with `[x]`/`[ ]` checkboxes
- [CHANGELOG.md](CHANGELOG.md) - dated entries describing changes

## Build & Run

- Build: `cd /c/code/endless/rust && cargo build --release 2>&1`
- Run: `cd /c/code/endless/rust && cargo run --release 2>&1`
- Check: `cd /c/code/endless/rust && cargo check 2>&1`

## Rules

- **NEVER use the Task tool to launch agents.** Do all work manually with direct tool calls (Read, Edit, Grep, Glob, Bash). If you think an agent would help, ask first — the answer will be no.

## Lessons Learned

- **PowerShell error suppression**: Don't use `2>$null` - it causes parse errors. Use `-ErrorAction SilentlyContinue` instead.
- **Bash paths on Windows**: Use `/c/code/endless` not `C:\code\endless` in bash commands. Windows backslash paths fail in the bash shell.
