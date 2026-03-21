# Endless

Real-time kingdom builder: pure Bevy 0.18 ECS with GPU compute for 50K NPCs via instanced rendering.

- When writing Rust for Endless, read `~/.claude/skills/bevy.md` first.
- When writing WGSL shaders, read `~/.claude/skills/wgsl.md` first.
- [docs/README.md](docs/README.md) - architecture, file map, patterns
- [docs/roadmap.md](docs/roadmap.md) - feature tracking with `[x]`/`[ ]` checkboxes
- [CHANGELOG.md](CHANGELOG.md) - dated entries describing changes

## Build & Run

`k3sc cargo-lock` auto-detects `Cargo.toml` from the working directory. Never use `cd`, never use `--manifest-path`, never use raw `cargo`.

- Run: `k3sc cargo-lock run --release 2>&1`
- Check: `k3sc cargo-lock check 2>&1`
- Clippy: `k3sc cargo-lock clippy --release -- -D warnings 2>&1`
- Test: `k3sc cargo-lock test 2>&1`
- Bench check: `k3sc cargo-lock check --bench system_bench 2>&1`

## Rules

- **NEVER use the Agent tool or Task tool.** Do all work manually with direct tool calls (Read, Edit, Grep, Glob, Bash). No subagents, no Explore agents, no Plan agents — nothing. If you think an agent would help, ask first — the answer will be no.

## Rust LSP (rust-analyzer)

LSP tool is available for Rust. Use it for type-aware queries instead of grep when you need compiler understanding.

- **Use LSP for**: type info (`hover`), jump to definition (`goToDefinition`), finding all callers (`incomingCalls`), impact analysis (`findReferences`), file structure (`documentSymbol`)
- **Use Grep/Glob for**: finding files, text search, locating symbols by name when you don't need type info
- **Paths**: Use Windows paths (`C:\code\endless\rust\src\foo.rs`) for LSP filePath, not bash paths
- **Indexing delay**: rust-analyzer needs time to index after startup. If `goToDefinition` returns nothing, try `hover` on the definition site directly, or retry later.
- **Column positions**: 1-based. Match the column to the start of the symbol name, not the line start.

## Lessons Learned

- **PowerShell error suppression**: Don't use `2>$null` - it causes parse errors. Use `-ErrorAction SilentlyContinue` instead.
- **Bash paths on Windows**: Use `/c/code/claude-4` not `C:\code\claude-4` in bash commands. Windows backslash paths fail in the bash shell.
- **Working directory**: This is `C:\code\claude-4` -- the claude-4 agent's own repo copy. Never cd to or reference `C:\code\endless` (that's the main copy for other agents/human).
