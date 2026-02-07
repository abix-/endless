# Endless

Colony sim: Godot 4.4 frontend, Rust/Bevy ECS backend, GPU compute for 10K NPCs @ 140fps.

Read `~/.claude/commands/endless.md` and `~/.claude/commands/test.md` for build/test commands.

- [docs/README.md](docs/README.md) - architecture, file map, patterns
- [docs/roadmap.md](docs/roadmap.md) - feature tracking with `[x]`/`[ ]` checkboxes
- [CHANGELOG.md](CHANGELOG.md) - dated entries describing changes

## Lessons Learned

When a mistake is made during development, document it here so we don't repeat it:

- **PowerShell error suppression**: Don't use `2>$null` - it causes parse errors. Use `-ErrorAction SilentlyContinue` instead. Example: `Get-Process *godot* -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue`
