# Endless

Colony sim: pure Bevy 0.18 ECS with GPU compute for 16K NPCs via instanced rendering.

- When writing Rust for Endless, read `~/.claude/skills/bevy.md` first.
- When writing WGSL shaders, read `~/.claude/skills/wgsl.md` first.
- Read `~/.claude/commands/endless.md` and `~/.claude/commands/test.md` for build/test commands.
- [docs/README.md](docs/README.md) - architecture, file map, patterns
- [docs/roadmap.md](docs/roadmap.md) - feature tracking with `[x]`/`[ ]` checkboxes
- [CHANGELOG.md](CHANGELOG.md) - dated entries describing changes

## Lessons Learned

- **PowerShell error suppression**: Don't use `2>$null` - it causes parse errors. Use `-ErrorAction SilentlyContinue` instead.
- **Bash paths on Windows**: Use `/c/code/endless` not `C:\code\endless` in bash commands. Windows backslash paths fail in the bash shell.
