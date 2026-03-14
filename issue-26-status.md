# Issue 26 Status

Current ai-collab state:

- Issue `#26` is claimed by `codex-2` for a review pass.
- GitHub claim comment was posted.
- The turn was interrupted during verification, before `cargo test` and the targeted benchmark completed.
- User later ran `cargo bench --bench system_bench death_system/25000 -- --noplot`, but the benchmark output/result is not captured in this note yet.

What was found:

- Claude's code change in `rust/src/systems/health.rs` moves NPC death marking into `damage_system` by inserting `Dead` immediately on lethal hits.
- That removes the old full-NPC `health_q.iter()` scan from `death_system`, which is the main intended perf fix.
- The benchmark harness in `rust/benches/system_bench.rs` was stale: it still set NPC HP to `0` and called `death_system` directly, which no longer matches the live `damage_system -> death_system` path.

What was changed locally:

- `rust/benches/system_bench.rs`
  - The death benchmark setup was updated so it zeroes HP and inserts `Dead` on the selected NPCs before running `death_system`.
  - This makes the benchmark measure the post-`damage_system` cleanup path that the game actually pays for.

Open follow-up:

- `docs/combat.md` still appears to describe the old full-scan death detection path.
- I attempted to patch that doc too, but targeted edits were fighting the file's existing encoding/text issues.
- The benchmark fix is the important code-side correction; the doc still needs cleanup if we want the writeup to match the implementation.

Verification that still needs to run:

1. `cargo test`
2. `cargo bench --bench system_bench death_system/25000 -- --noplot`
3. Compare the new 25K result to the old baseline from the issue:
   - old `death_system/25000`: about `57.9 ms`
4. If tests pass and the benchmark shows a significant improvement, leave the Codex handoff comment and close or hand back per workflow.

Relevant files:

- `rust/src/systems/health.rs`
- `rust/benches/system_bench.rs`
- `docs/combat.md`

Workspace caution:

- There are unrelated local changes in:
  - `rust/src/resources.rs`
  - `rust/src/ui/left_panel/mod.rs`
  - `rust/src/ui/mod.rs`
  - `rust/src/ui/armory.rs`
  - `docs/armory-ui.md`
- Those were not part of issue `#26` and should be left alone.
