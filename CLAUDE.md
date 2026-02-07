# Endless

Colony sim: Godot 4.6 frontend, Rust/Bevy ECS backend, GPU compute for 10K NPCs @ 140fps.

Read `~/.claude/commands/endless.md` and `~/.claude/commands/test.md` for build/test commands.

- [docs/README.md](docs/README.md) - architecture, file map, patterns
- [docs/roadmap.md](docs/roadmap.md) - feature tracking with `[x]`/`[ ]` checkboxes
- [CHANGELOG.md](CHANGELOG.md) - dated entries describing changes

## Godot

- **Path**: `C:\Games\godot\Godot_v4.6-stable_win64.exe`
- **Before building Rust**: Kill Godot first or the DLL will be locked. Use: `taskkill //F //IM Godot_v4.6-stable_win64.exe`

## Lessons Learned

When a mistake is made during development, document it here so we don't repeat it:

- **PowerShell error suppression**: Don't use `2>$null` - it causes parse errors. Use `-ErrorAction SilentlyContinue` instead.
- **Godot version mismatch**: Always check `tasklist | grep -i godot` to find the actual process name. Update CLAUDE.md when upgrading Godot.
