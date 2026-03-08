# Performance

Single source of truth for achieving maximum performance in this codebase. All other docs reference here for perf patterns.

## Core Principles

| Principle | Problem | Solution |
|-----------|---------|----------|
| DOD / Parallel arrays | Object overhead, cache misses | Contiguous `Vec<f32>` arrays (`EntityGpuState`) |
| Spatial grid | O(n²) neighbor search | O(n×k) cell lookup (GPU 256×256 grid, CPU `EntityMap` spatial buckets) |
| GPU compute | CPU bottleneck for movement/targeting | Parallel compute shader for all NPCs every frame |
| Instanced rendering | Draw call overhead (16K entities = 16K draw calls) | 1 `NpcBatch` entity for NPC storage-buffer path; buildings/overlays/projectiles use instance buffers. One `draw_indexed` per layer |
| Pipelined rendering | CPU/GPU sync stalls | Parallel main + render worlds, extract barrier once per frame |
| GPU readback avoidance | PCIe transfer stalls pipeline | Render reads GPU buffers directly; CPU readback async + throttled |
| Dirty-index uploads | Bulk buffer writes waste bandwidth | Per-dirty-index `write_buffer` (typically <1KB vs ~4MB bulk at 30K entities) |
| Coalesced writes | Many small GPU writes | Adjacent dirty indices merged into range writes (strict for GPU-authoritative, gap-based for CPU-authoritative) |
| Cadenced processing | Per-frame CPU spikes at scale | Bucket-gated systems spread NPC processing across frames |
| Budgeted pathfinding | A* at 50K NPCs would spike frames | Priority queue + per-frame budget (50-100 requests), LOS bypass for short distances |
| Event-driven updates | Redundant per-frame rebuilds | Dirty flags + message-driven triggers (visual upload, terrain sync, building grid) |

## Performance Targets

| Metric | 10K NPCs | 30K NPCs | 50K NPCs |
|--------|----------|----------|----------|
| UPS (FixedUpdate) | 60 | 60 | 60 |
| FPS (render) | 60 | 60 | 45-60 |
| Frame budget (FixedUpdate) | <8ms | <12ms | <16ms |
| Decision system | <1ms | <3ms | <5ms |
| GPU compute dispatch | <2ms | <3ms | <4ms |
| Visual upload (steady) | <0.1ms | <0.1ms | <0.1ms |

Reference hardware: mid-range discrete GPU (GTX 1060 / RX 580 class), 4-core CPU.

Benchmark: `cargo bench --bench system_bench` (Criterion, HTML reports in `target/criterion/`). In-game profiler via `SystemTimings` (enable `debug_profiler` in settings).

## Hybrid Data Access Rule

Use a hybrid access pattern by default:

1. Use Bevy `Query` for hot filtered scans (per-frame/per-tick loops).
2. Use `EntityMap` for keyed lookup/index access (`slot -> entity`, grid/cell occupancy, kind/town indexes, spatial helpers).
3. Do not replace keyed `EntityMap` lookups with full ECS scans.
4. Do not replace hot filtered query scans with full `EntityMap` NPC/building scans.

## Canonical Key Model

Treat `slot` as the canonical foreign key between ECS and `EntityMap`.

1. Canonical identity key: `slot` (`GpuSlot` in ECS).
2. Runtime handle: `Entity` (ephemeral; not persistence identity).
3. Required bridge: `slot <-> Entity` mapping stays synchronized.
4. Uniqueness rule: NPCs and buildings share one slot namespace; a slot value cannot be owned by both at the same time.
5. Secondary indexes are allowed for performance (`Entity -> slot`, grid cell, kind+town, spatial buckets), but all must resolve to the same canonical `slot`.
6. Invariants enforced via `debug_assert` in `resources.rs`: UID bijection (`uid_to_slot` / `slot_to_uid` stay synchronized on every register/unregister), slot uniqueness (cannot register two entities to same slot), town index validity (`town_idx != u32::MAX`). Debug builds only — zero cost in release. Invariant failure = slot lifecycle bug.

## Scope

Apply the hybrid rule to any runtime path that is expected to scale with population or map size:

1. `EguiPrimaryContextPass` systems and inspector/overlay rendering code.
2. `FixedUpdate` systems in active gameplay states (`Playing` / `Running`), especially Behavior/AI/Combat/Economy loops.
3. Any helper called from the above paths that may iterate NPC/building sets.

## GPU Performance Patterns

### Readback Minimization

GPU→CPU readback stalls the pipeline. Rules:
- Render pipeline reads GPU buffers directly (positions/health via bind group 2) — no readback needed for rendering.
- CPU readback is async via Bevy's `ReadbackComplete` observers — never blocks.
- Throttle expensive readbacks: `factions` and `threat_counts` at cadences listed in Current Tunings.
- Size readback buffers to actual entity count, not MAX.
- See [authority.md](authority.md) for which readback fields are authoritative.

### Dirty-Index Buffer Uploads

`EntityGpuState` tracks per-field dirty indices. `extract_npc_data` uploads only changed slots:
- **Strict coalescing** (GPU-authoritative: positions, arrivals): merges only exactly-adjacent dirty indices. Stale CPU values would teleport NPCs.
- **Gap-based coalescing** (CPU-authoritative: targets, speeds, factions, healths, flags): merges nearby dirty indices with configurable gap thresholds. Waste budget and fallback threshold in Current Tunings.
- **Visual/equip**: gap-based coalescing via `visual_uploaded_indices` / `equip_uploaded_indices`. Flash-only slots skip equip entirely.
- Full rebuild only on startup/load (`visual_full_rebuild` flag).

### Instanced Rendering

Custom pipeline replaces Bevy's per-entity sprite renderer:
- 1 `NpcBatch` entity vs 16K entities in the render world.
- Storage buffer path for NPCs (positions/health read from compute output, no readback).
- Instance buffer path for buildings/overlays/projectiles.
- Multi-layer drawing: body + up to 7 overlay layers in one `RenderCommand`.

## CPU Cadencing Patterns

### Bucket-Gated Decision System

At 10K+ NPCs, running `decision_system` for every NPC every frame is too expensive. Solution: bucket gating.

- **Fighting NPCs**: `COMBAT_BUCKET` — fast enough for flee/leash reactions (see Current Tunings for value). Scaled down by `time_scale` at high game speeds so combat decisions keep pace with movement.
- **Non-fighting NPCs**: `think_buckets = max(interval × 60, npc_count / max_decisions_per_frame)` — adaptive bucketing with frame budget cap (see Current Tunings for `max_decisions_per_frame`). Also scaled down by `time_scale` at high game speeds.
- At 10K NPCs with 120 buckets: ~83 NPCs processed per frame instead of 10K.
- Position hoisted once per NPC into `npc_pos` after bucket gate — eliminates scattered position reads.
- Conditional writeback: captures original values, compares at end — only calls `get_mut()` for changed fields. Optimal for `decision_system` where most NPCs exit early via `break 'decide` — avoids unnecessary borrow-mut for unchanged entities.

### Candidate-Driven Healing

Replaced full 50K NPC iteration with O(active_healing + sampled_candidates):
- **Sustain-check (every frame)**: iterates only `ActiveHealingSlots.slots` (active set). O(1) membership via `mark[slot]`.
- **Enter-check (cadenced)**: `slot % N` bucketing per town (see Current Tunings). Hysteresis radii prevent oscillation.

### Fixed-Cadence Systems

- `farm_visual_system`: cadenced (crop state changes slowly; see Current Tunings).
- `spawner_respawn_system`: timer-based per spawner (no per-frame iteration).
- `raider_forage_system`: hourly timer accumulation per raider town.

### Budgeted Pathfinding

A* requests queued by `resolve_movement_system` (from MovementIntents) and `invalidate_paths_on_building_change`. `resolve_movement_system` processes up to `max_per_frame` requests per FixedUpdate tick via `PathRequestQueue.drain_budget()`, sorted by (priority, slot) for determinism. Short-distance moves (< 5 tiles with clear LOS) bypass A* entirely — direct boids steering. Time budget guard (`max_time_budget_ms`) re-queues overflow.

### Event-Driven Systems

- `build_visual_upload`: persistent `NpcVisualUpload`, dirty-signaled via `MarkVisualDirty`. ~4-8ms → ~0.01ms steady state.
- `rebuild_building_grid_system`: runs only on `BuildingGridDirtyMsg`.
- `invalidate_paths_on_building_change`: runs on `BuildingGridDirtyMsg`, re-queues paths crossing changed cells.
- Terrain tilemap sync: `TerrainDirtyMsg`-driven, not `WorldGrid::is_changed()`.

## Debug Overhead

Debug metrics can cost more than the actual simulation. Disable or throttle them.

**Example trap** — O(n²) validation to verify NPC separation:
```rust
fn get_min_separation(positions: &[f32], count: usize) -> f32 {
    let mut min_dist = f32::MAX;
    for i in 0..count {
        for j in (i+1)..count {
            let dx = positions[i*2] - positions[j*2];
            let dy = positions[i*2+1] - positions[j*2+1];
            min_dist = min_dist.min((dx*dx + dy*dy).sqrt());
        }
    }
    min_dist
}
```

With 5,000 NPCs, that's 12.5 million distance checks per frame. Your 60 UPS simulation drops to 15 — but the simulation itself is fine, only the *measurement* is slow.

**Rules:**
1. Make expensive metrics opt-in (debug flags).
2. Throttle: run expensive checks once per second, not every frame.
3. Sample: check 100 random pairs instead of all pairs.
4. **If your metric is O(n²) or worse, it needs a toggle.**

Profiler UI (`SystemTimings`) itself is cadenced: `Local<ProfilerCache>` refresh rate and render limits in Current Tunings.

## Current Tunings

All volatile numeric constants in one place. Policy sections above describe *why*; this table tracks *what value*.

| Tuning | Value | Location |
|--------|-------|----------|
| `COMBAT_BUCKET` | 16 frames (~267ms @ 60 UPS) | `behavior.rs:369` |
| `max_decisions_per_frame` | 300 | `resources.rs:110` |
| `CHECK_INTERVAL` (threat recheck) | 30 frames | `behavior.rs:353` |
| `HEAL_DRIFT_RADIUS` | 100.0 | `behavior.rs:355` |
| `ARCHER_PATROL_WAIT` | 60 ticks | `constants.rs:1207` |
| `ENERGY_TIRED_THRESHOLD` | 30.0 | `constants.rs:1213` |
| `ENERGY_WAKE_THRESHOLD` | 90.0 | `constants.rs:1210` |
| Faction readback throttle | 60 frames | `gpu.rs` |
| Threat readback throttle | 30 frames | `gpu.rs` |
| Farm visual cadence | every 4th frame | `behavior.rs` |
| ProfilerCache refresh | 15 frames, top 10 | `ui/game_hud.rs` |
| Healing enter-check cadence | 1/4 NPCs per frame | `health.rs` |
| Gap coalescing waste budget | ~24KB total across all buffers | `gpu.rs` |
| Visual upload fallback | 40% window → bulk offset write | `gpu.rs` |
| `max_pathfinds_per_frame` | 50 | `resources.rs` (PathfindConfig) |
| `pathfind_short_distance_tiles` | 5 | `resources.rs` (PathfindConfig) |
| `pathfind_max_nodes` | 5000 | `resources.rs` (PathfindConfig) |
| `pathfind_stuck_repath_frames` | 30 | `resources.rs` (PathfindConfig) |

## Migration Templates

Use these templates when converting existing code to the hybrid pattern.

### 1) Query-First Scan (hot loops)

- Use when the primary operation is filtering/iterating active entities each frame/tick.
- Keep data access inside the loop to query fields unless keyed lookup is required.
- Treat `slot` as identity in the loop; use `Entity` only as an execution handle.

```rust
// Fast filtered scan: only matching ECS entities are iterated.
for (town_id, entity_slot) in military_query.iter() {
    let slot = entity_slot.0; // canonical identity
    *units_by_town.entry(town_id.0).or_default() += 1;
    // Optional keyed follow-up only when needed:
    // let entity = entity_map.entities.get(&slot).copied();
}
```

### 2) EntityMap Keyed Lookup

- Use when the primary operation is direct lookup by slot, grid cell, position, kind, or town index.
- Do not replace these with ECS full scans.

```rust
// Canonical lookup by slot:
let slot = selected_slot;
if let Some(entity) = entity_map.entities.get(&slot).copied() {
    commands.entity(entity).insert(MyMarker);
}

if entity_map.has_building_at(col, row) {
    // Handle occupied cell quickly via keyed/indexed access.
}
```

### 3) Mixed Path (scan + keyed follow-up)

- Use query for candidate discovery.
- Use `EntityMap` only for per-candidate keyed operations.

```rust
for (entity_slot, town_id) in squad_units_query.iter() {
    let slot = entity_slot.0; // canonical key from ECS
    if town_id.0 != player_town { continue; }
    // Query gives active candidates; EntityMap resolves keyed world/index data if needed.
    if let Some(inst) = entity_map.get_instance(slot) {
        // keyed slot/index lookup
        if inst.occupants > 0 {
            // ...
        }
    }
}
```

### 4) Local Pre-Index Map (single frame/tick)

- Use when repeated keyed lookups are needed inside one system run.
- Build once at start of system; reuse in inner loops.

```rust
// Derive runtime handles from canonical slot once per system run.
let mut entity_by_slot: HashMap<usize, Entity> = HashMap::new();
for (entity, entity_slot) in unit_query.iter() {
    entity_by_slot.insert(entity_slot.0, entity);
}

for slot in selected_slots.iter().copied() {
    if let Some(&entity) = entity_by_slot.get(&slot) {
        commands.entity(entity).remove::<DirectControl>();
    }
}
```

### 5) Anti-Template (do not use in hot paths)

```rust
// Avoid: full EntityMap scan + manual filtering for data already available via query filters.
for npc in entity_map.iter_npcs() {
    if npc.dead || !npc.is_military { continue; }
    // ...
}
```

## Hot Path Rules

1. Avoid repeated full-query scans for the same key in one UI frame or system tick.
2. Avoid nested linear membership checks (`Vec::contains`, `iter().any/find/position`) inside loops over large sets.
3. Avoid rebuilding the same derived data multiple times in one pass.
4. Avoid per-item expensive string work (`format!`, allocation-heavy debug text) in hot loops unless debug-gated.
5. Avoid full-list dedupe scans in overlays/logical render loops when keyed dedupe is possible.
6. Do not use `entity_map.iter_npcs()` plus per-item ECS `Query.get(...)` in per-frame/per-tick hot loops; use query-first iteration over ECS components instead. Exception: cold paths (save) and event-driven systems (visual upload) may use `iter_npcs()` when NpcInstance fields are the primary data source. See Known Exceptions.
7. In hot decision/combat loops, avoid clone-local-then-writeback patterns for large component state; mutate query-owned components directly where possible.
8. Use mutable query types only when mutation is required (`Query<&T>` over `Query<&mut T>` for read-only paths) to reduce borrow/scheduling contention.

## Common Anti-Patterns and Replacements

- Repeated query lookup:
  - Pattern: call `query.iter().find(...)` multiple times for the same slot/entity in one frame.
  - Replace with: one pre-pass map (`slot -> data`) or one cached lookup result reused in that frame.

- Index scan + component probe in hot loops:
  - Pattern: `for npc in entity_map.iter_npcs() { query.get(npc.entity) ... }` every frame/tick.
  - Replace with: query-native iteration using component filters; use `EntityMap` only for keyed/index lookups that queries cannot express efficiently.

- Nested membership checks:
  - Pattern: `for x in A { if B.contains(x) { ... } }` where `B` is `Vec`.
  - Replace with: `HashSet` for membership or sorted vector + binary search when stable ordering is needed.

- Redundant traversals:
  - Pattern: multiple passes over the same query/collection for related outputs.
  - Replace with: single pass that accumulates all needed outputs.

- Clone + writeback state machine loops:
  - Pattern: clone multiple components into locals, run logic, then write back all fields for every entity.
  - Replace with: mutate query-owned fields in place; only clone small immutable data when needed.

- Per-item dedupe scan:
  - Pattern: maintain `Vec` and run `iter().any(...)` for each candidate.
  - Replace with: quantized/keyed `HashSet` dedupe (`(x_bin, y_bin)` or slot key).

- Unbounded debug cost:
  - Pattern: unconditional log formatting/counting in per-frame systems.
  - Replace with: debug flags + sampling + cached strings where possible.

## PR Review Procedure

1. Mark hot paths touched by the PR (`EguiPrimaryContextPass`, `FixedUpdate` systems in active sets, AI/behavior loops).
2. For each hot path, note collection sizes and complexity (`O(n)`, `O(n^2)` risks).
3. Flag any repeated scans/membership checks and propose concrete replacement.
4. Add/adjust microbenchmarks for modified hotspots.
5. Confirm no new unconditional debug/string work in tight loops.
6. Confirm no stale architecture comments remain after migration work (comments must match current ownership model).
7. Confirm read-only systems do not request mutable queries/resources unless needed.

## Benchmark/Guardrail Expectations

Benchmark tool: `cargo bench --bench system_bench` (Criterion). Run `/benchmark` to execute and record results. In-game profiler: `SystemTimings` (enable `debug_profiler` in settings).

1. Add microbenchmarks for hotspot helpers when introducing or changing their logic.
2. Keep baseline numbers for representative counts (small, medium, stress) against Performance Targets.
3. Fail CI on material regressions (for example, >20 percent for benchmarked hotspots).
4. Document benchmark command and expected range in the PR.

## Current Known Hotspot Patterns

- UI inspector paths doing repeated slot lookups across multiple queries in a single frame.
- Squad/selection flows using `Vec::contains` within nested loops.
- Overlay target dedupe using per-target linear scans.
- Cleanup/reassignment systems scanning full queries repeatedly instead of pre-indexing.
- Decision system conditional writeback: captures original values at loop top, compares at end, only calls `get_mut()` for changed fields. Most NPCs exit early via `break 'decide` with no state changes, skipping all writeback. Remaining overhead: per-NPC component reads at loop top for ~10 fields.

## Known Exceptions

Legitimate violations of the rules above, tracked with exit criteria.

| Exception | Rule violated | Reason | Cost | Exit criteria |
|-----------|-------------|--------|------|---------------|
| `save.rs` uses `iter_npcs()` | Hot Path #6 | Save is cold path (F5 only) | N/A | None needed |
| `npc_render.rs` `build_visual_upload` uses `iter_npcs()` | Hot Path #6 | Needs NpcInstance fields not in ECS; event-driven dirty-only | Acceptable | None — correct pattern |
| `roster_panel.rs` / `left_panel.rs` use `iter_npcs()` | Hot Path #6 | UI roster display needs full NPC list | 30-frame cadence cache | Add pagination or virtual scroll |

## Benchmark History

Run via `cargo bench --bench system_bench` (Criterion). Use `/benchmark` to execute and append results.

### 2026-03-08 — d0191eb (baseline)

| System | 1K | 5K | 10K | 25K | 50K |
|--------|----|----|-----|-----|-----|
| decision | 33µs | 72µs | 118µs | 261µs | 520µs |
| damage | 21µs | 48µs | 55µs | 130µs | 251µs |
| healing | 11µs | 39µs | 80µs | 205µs | 443µs |
| attack | 21µs | 70µs | 134µs | 328µs | 674µs |

Combined 50K: 1.9ms (11.4% of 16ms budget). All systems O(n) — linear scaling confirmed.

### 2026-03-08b — 030c060 (full suite, 9 systems)

**NPC-scaled** (vary entity count):

| System | 1K | 5K | 10K | 25K | 50K | Scaling |
|--------|----|----|-----|-----|-----|---------|
| decision | 34µs | 68µs | 112µs | 261µs | 480µs | O(n) |
| damage | 21µs | 51µs | 54µs | 129µs | 248µs | O(n) |
| healing | 11µs | 39µs | 79µs | 213µs | 448µs | O(n) |
| attack | 22µs | 73µs | 138µs | 344µs | 668µs | O(n) |
| resolve_movement | 2.06ms | 2.07ms | 2.10ms | 2.22ms | 2.32ms | O(1) budget-capped |
| populate_gpu_state | 177µs | 204µs | 231µs | 536µs | 985µs | O(n) |

**Building-scaled** (vary tower count, 5K enemy NPCs):

| System | 10 | 50 | 100 | 250 | 500 |
|--------|----|----|-----|-----|-----|
| building_tower | 5µs | 7µs | 9µs | 15µs | 26µs |

**Spawner-scaled** (vary spawner building count):

| System | 100 | 500 | 1K | 2K |
|--------|-----|-----|----|-----|
| spawner_respawn | 66µs | 4.3ms | 18.5ms | 88.2ms |

Combined 50K (6 NPC-scaled systems): 5.1ms (30.7% of 16ms budget).

**Findings:**
- `spawner_respawn` is **O(n²)** — 2K spawners = 88ms, blows frame budget 5×. Likely caused by `iter_instances()` filter inside per-spawner loop.
- `resolve_movement` is budget-capped (~2ms) via `max_per_frame` pathfind limit — near-constant regardless of NPC count.
- `death_system` bench needs rework — shows flat ~42µs regardless of death count (Dead NPCs not being processed).
