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

- **Fighting NPCs**: `COMBAT_BUCKET` — fast enough for flee/leash reactions (see Current Tunings for value). No `time_scale` adjustment -- FixedUpdate runs at constant 60 Hz; `game_time.delta()` handles game-speed scaling per tick.
- **Non-fighting NPCs**: `think_buckets = max(interval × 60, npc_count / max_decisions_per_frame)` — adaptive bucketing with frame budget cap (see Current Tunings for `max_decisions_per_frame`). No `time_scale` adjustment -- same reason.
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

### Time-Scale Scheduling (sync_fixed_hz)

FixedUpdate runs at constant 60 Hz regardless of `time_scale`. `sync_fixed_hz` (Update schedule) enforces `Time<Fixed>.period = 1/60` at all speeds.

| Speed | Fixed Hz | Ticks/real-s | Game-s/real-s |
|-------|----------|-------------|---------------|
| 1x | 60 Hz | 60 | 1.0 |
| 2x | 60 Hz | 60 | 2.0 |
| 4x | 60 Hz | 60 | 4.0 |
| 8x | 60 Hz | 60 | 8.0 |
| 16x | 60 Hz | 60 | 16.0 |

Game-time scaling is handled by `game_time.delta()` which returns `time.delta_secs() * time_scale`. At 4x, each tick advances game time by 4/60 game-seconds instead of 1/60. Net rate: `60 ticks/s * time_scale/60 = time_scale` game-s/real-s.

No cascade risk: at 60 Hz period (16.67ms), Bevy runs ~1 fixed tick per frame at 60+ FPS. Per-tick CPU cost is constant because systems iterate the same entities regardless of delta size.

### HPA* Hierarchical Pathfinding

Custom HPA* (Hierarchical Pathfinding A*) replaces raw A* for cross-chunk paths. Grid divided into 16×16 chunks (~256 chunks on 250×250 grid). Entrance nodes placed at chunk boundary crossings. Intra-chunk paths precomputed via A* between all entrance pairs within each chunk. Queries search the abstract graph (~500-1000 entrance nodes) instead of the full 62,500-cell grid, then stitch cached intra-chunk segments into full paths.

- **Build**: `HpaCache::build()` called in `init_pathfind_costs()`. Scans horizontal/vertical borders for entrance nodes, runs intra-chunk A* between all pairs, connects cross-border edges.
- **Query**: `pathfind_hpa()` — same-chunk paths use chunk-bounded A* directly (small search space). Cross-chunk paths insert temporary start/goal nodes, A* on abstract graph, stitch cached paths.
- **Update**: `HpaCache::rebuild_chunks()` called in `sync_building_costs()` when buildings change. Incremental: removes nodes in dirty chunks + neighbors, re-scans borders and recomputes intra-chunk edges for affected chunks only. Shared `build_chunks()` method used by both `build()` and `rebuild_chunks()`.
- **Heuristic**: Abstract graph A* uses `manhattan_distance × HPA_MIN_COST` (67 = road cost) for tight, admissible heuristic.

### Budgeted Pathfinding

A* requests queued by `resolve_movement_system` (from MovementIntents) and `invalidate_paths_on_building_change`. `resolve_movement_system` processes up to `max_per_frame` requests per FixedUpdate tick via `PathRequestQueue.drain_budget()`, sorted by (priority, slot) for determinism. Short-distance moves (< 12 tiles with clear LOS) bypass A* entirely — direct boids steering. Time budget guard (`max_time_budget_ms`) re-queues overflow. With HPA*, the budget cap is nearly unnecessary — 5000 unbounded requests cost <1ms — but retained as safety margin.

### Event-Driven Systems

- `build_visual_upload`: persistent `NpcVisualUpload`, dirty-signaled via `MarkVisualDirty`. ~4-8ms → ~0.01ms steady state.
- `rebuild_building_grid_system`: runs only on `BuildingGridDirtyMsg`. After first init, skips the O(N) `rebuild_spatial()` -- spatial grid is maintained incrementally by `add_instance`/`remove_instance` on every building change. First init (or reload) does a full rebuild. Fixed in #207: was O(all_buildings) on every building change, caused 19ms spikes. Now O(1) (~1.5us regardless of building count).
- `sync_pathfind_costs_system`: runs on `BuildingGridDirtyMsg`. Rebuilds pathfind cost grid and HPA* cache. Fixed in #203: diffs before/after costs so HPA* `rebuild_chunks` only runs for cells whose cost actually changed (not all building cells). No-change case (redundant dirty signal) skips HPA* entirely.
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
| `COMBAT_BUCKET` | 16 frames (~267ms @ 60 UPS) | `systems/decision/mod.rs:347` |
| `max_decisions_per_frame` | 300 | `resources.rs:188` |
| `CHECK_INTERVAL` (threat recheck) | 30 frames | `systems/decision/mod.rs:336` |
| `HEAL_DRIFT_RADIUS` | 100.0 | `systems/decision/mod.rs:338` |
| `ARCHER_PATROL_WAIT` | 60 ticks | `constants/mod.rs:102` |
| `ENERGY_TIRED_THRESHOLD` | 30.0 | `constants/mod.rs:108` |
| `ENERGY_WAKE_THRESHOLD` | 90.0 | `constants/mod.rs:105` |
| Faction readback throttle | 60 frames | `gpu.rs` |
| Threat readback throttle | 30 frames | `gpu.rs` |
| Farm visual cadence | event-driven (FarmReadyMsg/FarmHarvestedMsg) | `systems/economy/mod.rs` |
| ProfilerCache refresh | 15 frames, top 10 | `ui/left_panel/mod.rs` |
| Healing enter-check cadence | 1/4 NPCs per frame | `systems/health/mod.rs` |
| Gap coalescing waste budget | ~24KB total across all buffers | `gpu.rs` |
| Visual upload fallback | 40% window → bulk offset write | `gpu.rs` |
| `max_pathfinds_per_frame` | 200 | `settings.rs:797` / `resources.rs:772` (PathfindConfig) |
| `pathfind_short_distance_tiles` | 12 | `resources.rs` (PathfindConfig) |
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

- Per-item Vec::retain in loops (O(n²)):
  - Pattern: `for item in items { vec.retain(|x| x != item); }` — linear scan per removal.
  - Replace with: `DenseSlotMap<T>` / `DenseSlotSet` (entity_map.rs) — dense parallel Vecs + reverse HashMap, O(1) insert/remove/get, cache-friendly iteration via `slot_slice()`/`values()`. `DenseSlotSet` is thin wrapper (`DenseSlotMap<()>`). Applied to building indexes + building instances. See 2026-03-08h benchmark for 6.5× speedup on death_system.

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
- Decision system conditional writeback: captures original values at loop top, compares at end, only calls `get_mut()` for changed fields. Most NPCs exit early via `break 'decide` with no state changes, skipping all writeback. Remaining overhead: per-NPC component reads at loop top for ~10 fields.

## Known Exceptions

Legitimate violations of the rules above, tracked with exit criteria.

| Exception | Rule violated | Reason | Cost | Exit criteria |
|-----------|-------------|--------|------|---------------|
| `save.rs` uses `iter_npcs()` | Hot Path #6 | Save is cold path (F5 only) | N/A | None needed |
| `npc_render.rs` `build_visual_upload` uses `iter_npcs()` | Hot Path #6 | Needs NpcInstance fields not in ECS; event-driven dirty-only | Acceptable | None — correct pattern |
| `roster_panel.rs` / `left_panel.rs` use `iter_npcs()` | Hot Path #6 | UI roster display needs full NPC list | 30-frame cadence cache | Add pagination or virtual scroll |

## Current Benchmark Results

Run via `cargo bench --bench system_bench` (Criterion). Use `/benchmark` to execute and append results. Last full run: 2026-03-15 -- 306decc. See bottom section for latest numbers.

### NPC-scaled (vary entity count, 50K NPCs baseline)

| System | 1K | 5K | 10K | 25K | 50K | Scaling |
|--------|----|----|-----|-----|-----|---------|
| decision | 41µs | 71µs | 111µs | 220µs | 404µs | O(n) |
| damage | 173µs | 189µs | 212µs | 393µs | 835µs | O(n) |
| healing | 12µs | 45µs | 92µs | 231µs | 481µs | O(n) |
| attack | 22µs | 72µs | 138µs | 331µs | 674µs | O(n) |
| resolve_movement | 19µs | 45µs | 63µs | 115µs | 212µs | O(n) HPA* |
| resolve_movement_unbounded | 33µs | 84µs | 156µs | 367µs | 723µs | O(n) HPA* |
| populate_gpu_state | 53µs | 193µs | 226µs | 520µs | 958µs | O(n) |
| energy | 3µs | 10µs | 19µs | 41µs | 79µs | O(n) |
| arrival | 16µs | 23µs | 31µs | 52µs | 92µs | O(n) |
| gpu_position_readback | 5µs | 19µs | 34µs | 82µs | 160µs | O(n) |
| advance_waypoints | 5µs | 13µs | 22µs | 48µs | 92µs | O(n) |
| cooldown | 5µs | 8µs | 14µs | 30µs | 58µs | O(n) |
| npc_regen | 2µs | 2µs | 3µs | 2µs | 3µs | O(1)† |
| on_duty_tick | 3µs | 8µs | 14µs | 30µs | 59µs | O(n) |

†npc_regen: O(1) because only 25% have hp_regen > 0; branch prediction skips most NPCs.

Combined 50K (13 systems, excluding unbounded variant): 4.1ms (25.6% of 16ms budget).

### Building-scaled (vary building count)

| System | 100 | 500 | 1K | 5K | 50K | Scaling |
|--------|-----|-----|----|-----|------|---------|
| building_tower | 13µs | 18µs | 22µs | 61µs | 573µs | O(n) |
| growth | 13µs | 17µs | 22µs | 55µs | 580µs | O(n) |
| construction_tick | 6µs | 14µs | 24µs | 103µs | 927µs | O(n) |

### Spawner-scaled (vary spawner building count)

| System | 100 | 500 | 1K | 5K | 50K | Scaling |
|--------|-----|-----|----|-----|------|---------|
| spawner_respawn | 19µs | 37µs | 59µs | 233µs | 1.7ms | O(n) |

### Spawn-scaled (materialize_npc per-spawn cost)

| Spawns/frame | 10 | 50 | 100 | 500 | Scaling |
|-------------|-----|------|------|------|---------|
| spawn_npc | 993µs | 2.7ms | 3.7ms | 10.0ms | O(n) ~20µs/spawn |

### Projectile-scaled (process_proj_hits)

| Projectiles | 100 | 500 | 1K | 5K | Scaling |
|------------|-----|------|-----|-----|---------|
| process_proj_hits | 2µs | 2µs | 4µs | 10µs | O(n) |

### Death-scaled (full death→despawn→respawn cycle, fixed 50K NPCs)

| Deaths/frame | 100 | 500 | 1K | 5K | 25K | Scaling |
|-------------|-----|-----|----|-----|------|---------|
| death_system | 270µs | 803µs | 1.6ms | 10.1ms | 54.0ms | O(n) ~1.6µs/death |

500 deaths/frame (heavy combat) = 803µs (5% of budget).

### Event-driven (BuildingGridDirtyMsg, vary building count)

| System | 500 | 2K | 5K | Scaling |
|--------|-----|----|----|---------|
| rebuild_building_grid | 1.5us | 1.6us | 1.6us | O(1) after init |
| sync_pathfind_costs | 16us | 102us | 190us | O(buildings) overlay + O(changed) HPA* |

rebuild_building_grid is O(1) because spatial grid is maintained incrementally; the system just checks `is_spatial_initialized()`. sync_pathfind_costs scales with total buildings (overlay pass) but HPA* rebuild is scoped to actually-changed cells.

### Budget Summary (50K NPCs + realistic 2K buildings, heavy combat frame)

| Component | Cost | % of 16ms |
|-----------|------|-----------|
| 13 NPC-scaled systems (50K NPCs) | 4.1ms | 25.6% |
| death_system (500 deaths) | 803µs | 5.0% |
| process_proj_hits (1K projectiles) | 4µs | 0.0% |
| building systems (2K buildings) | ~120µs | 0.8% |
| spawner_respawn (2K spawners) | ~70µs | 0.4% |
| **Total measured** | **5.1ms** | **31.9%** |

Remaining budget for GPU compute, rendering, UI, and unmeasured systems: ~10.9ms (68%).

Note: building systems at realistic counts (2K) are negligible. 50K-per-type stress tests are in the building-scaled tables above but don't represent real gameplay.

## Optimization Log

Compact record of performance fixes applied. Each entry preserves the root cause analysis and pattern used.

### spawner_respawn O(n²) → O(n) — 1,176× faster

**Root cause**: `find_nearest_free()` (world.rs) used generic `for_each_nearby` spatial search iterating ALL building types per cell. N spawners × N buildings = O(n²). 2K spawners = 88ms.

**Fix**: (1) `spawner_slots` index in entity_map.rs — O(spawners) collection instead of O(all_buildings). (2) Kind-filtered spatial search via `for_each_nearby_kind_town` — pre-built indexes containing only matching building kinds. Empty buckets = instant no-op.

**Pattern**: Candidate-Driven — use pre-built type-specific indexes instead of scanning all entities and filtering. 2K spawners: 88ms → 75µs.

### rebuild_building_grid_system redundant full rebuilds — incremental dirty path restored

**Root cause**: `rebuild_building_grid_system` used to call `EntityMap::rebuild_spatial()` on every `BuildingGridDirtyMsg`, even after the spatial grid had already been initialized. That turned each add/remove building event into an O(n_buildings) rebuild even though `EntityMap::add_instance()` and `remove_instance()` already maintain the spatial indexes incrementally via `spatial_insert` / `spatial_remove`.

**Fix**: Add `EntityMap::is_spatial_initialized()` and gate the full rebuild behind first-time initialization only. The system still performs one full rebuild after startup so buildings placed before `init_spatial()` become queryable, but all subsequent dirty messages now take the incremental path and skip the O(n) rebuild.

**Guardrails**: `world::tests::building_added_after_init_findable_without_dirty_message` verifies that a post-init `add_instance()` becomes queryable without requiring another full rebuild.

**Criterion results** (Windows, i7-9700K, rebuild_building_grid):

| Buildings | full_rebuild (before) | incremental_add_one (after) | Speedup |
|-----------|----------------------|----------------------------|---------|
| 100 | 5.8us | 178ns | 33x |
| 500 | 55.8us | 515ns | 108x |
| 1000 | 96.1us | 413ns | 233x |
| 5000 | 554.5us | 679ns | 817x |
| 50000 | 12.8ms | 380ns | 33,695x |

Incremental `add_instance` (spatial_insert inline) is O(1) at ~180-680ns regardless of building count. Full `rebuild_spatial` scales O(n): 12.8ms at 50K buildings.

**Pattern**: Event-driven incremental maintenance -- when the authoritative index is already updated inline on add/remove, dirty-message handlers should only reconcile first-time initialization or true bulk rebuild cases, not blindly rescan the entire collection every tick.

### HPA* hierarchical pathfinding — 341× faster

**Root cause**: Raw A* searched ~5000 grid cells per request. At 50K NPCs with 10% pathing: 5000 requests × 51µs = 257ms unbounded.

**Fix**: Custom HPA* — 16×16 chunks, entrance nodes at boundaries, precomputed intra-chunk paths. Abstract graph A* searches ~100-500 nodes instead of ~5000. Also increased LOS bypass 5→12 tiles and eliminated 125KB/frame cost grid clone.

**Result**: Unbounded 50K: 257ms → 753µs. Budgeted 50K: 2.27ms → 214µs. Budget cap now nearly unnecessary. Cache build is one-time (~50-100ms at world init), chunk rebuilds on building change <1ms.

### death_system O(n²) → O(1) via DenseSlotSet — 7× faster

**Root cause**: `EntityMap.unregister_npc` called `npc_by_town[town].retain(|&s| s != slot)` per death — O(town_size) linear scan per removal. N deaths × town_size = O(n²). Also, `NpcsByTownCache` duplicated the same data, paying the O(n) retain twice.

**Fix** (three parts):
1. **Delete NpcsByTownCache** — redundant with `EntityMap.npc_by_town`. Removed from 8 files.
2. **DenseSlotMap\<T\>** (entity_map.rs) — generic dense parallel `Vec<usize>` (slots) + `Vec<T>` (data) + reverse `HashMap<usize, usize>` (slot → position). O(1) insert/remove/get, cache-friendly iteration via `slot_slice()`/`values()`. `DenseSlotSet` is thin wrapper (`DenseSlotMap<()>`). Same pattern as [EnTT sparse sets](https://gist.github.com/dakom/82551fff5d2b843cbe1601bbaff2acbf) and [`IndexSet::swap_remove`](https://docs.rs/indexmap/latest/indexmap/set/struct.IndexSet.html). Applied to `npc_by_town`, `by_kind`, `by_kind_town`, `spawner_slots`, and `instances` (building data).
3. **Defer equipment extraction** — moved equipment queries inside `if last_hit_by >= 0` block. Starvation deaths skip 2 Vec allocations.

**Pattern**: For any collection needing O(1) keyed access and cache-friendly iteration, use `DenseSlotMap<T>` (with data) or `DenseSlotSet` (slot-only). 500 deaths/frame: 7.8ms → 951µs.

### slot_for_entity O(n) → O(1) via reverse index — 8× faster at 5K

**Root cause**: `EntityMap::slot_for_entity(Entity)` did `self.entities.iter().find(...)` — O(n) linear scan of HashMap by value. Called per `DamageMsg`, per worksite release, per squad member lookup. N messages × N entities = O(n²) in damage_system.

**Fix**: Added `entity_to_slot: HashMap<Entity, usize>` reverse index to EntityMap, maintained via `set_entity()`/`remove_entity_mapping()` helpers. `slot_for_entity` becomes `entity_to_slot.get(&entity).copied()` — O(1).

**Pattern**: Bijection index — when a forward map (slot→entity) is frequently queried in reverse (entity→slot), add a parallel reverse HashMap. Documented in Canonical Key Model: "Secondary indexes are allowed for performance (`Entity -> slot`)". damage_system 5K: 1860µs → 228µs.

### build_building_body_instances cache-miss fix — reduced scattered 200K-array reads (#209)

**Root cause**: `build_building_body_instances` read `gpu_state.positions[idx*2]` and `gpu_state.factions[idx]` per building. Building slots start at MAX_NPC_COUNT (~100K), so these were scattered reads into 200K-element flat arrays -- poor cache locality. Also, per-building ECS `query.get()` for construction progress instead of a single pre-indexed HashMap. Observed peak: 4.55ms at ~1111 NPCs.

**Fix**: (1) Read `inst.position` and `inst.faction` from `BuildingInstance` in the compact `DenseSlotMap` (cache-friendly sequential layout) instead of scattered array reads. Buildings are static after placement (position) and faction is CPU-authoritative on EntityMap (authority.md), so the values are always correct. (2) Pre-build `under_construction_by_slot: HashMap<usize, f32>` once per frame via `Query<(&GpuSlot, &ConstructionProgress)>` -- O(under_construction) not O(all_buildings). Dirty guard (issue #187) skips the rebuild entirely when nothing changed.

**Pattern**: Compact authority read -- when data is available on a compact ECS/EntityMap structure AND is CPU-authoritative (won't be overwritten by GPU compute), read from there instead of the large parallel GPU arrays. Reduces scattered reads from 5 to 2 per building (sprite_indices + flash).

**Before/after**: Needs Windows cargo bench and live endless-cli get_perf measurement (k3s cannot run game or criterion benchmarks). See bench_build_building_body_instances (dirty: 68K buildings) in system_bench.rs.

### rebuild_building_grid O(N) → O(1) — eliminated 19ms spike (#207)

**Root cause**: `rebuild_building_grid_system` called `entity_map.rebuild_spatial()` (O(all_buildings) full spatial grid rebuild) on every `BuildingGridDirtyMsg`. At high game speed with AI placing buildings, towers killing raiders, etc., this fired frequently. Observed 19.05ms peak spike in live gameplay at 1111 NPCs.

**Fix**: `add_instance`/`remove_instance` already maintain the spatial grid incrementally on every building change. After first init, skip `rebuild_spatial()` entirely -- only call `init_spatial()` (no-op at same size). First init or reload still does a full rebuild.

**Pattern**: Incremental maintenance -- if insert/remove operations already update the data structure, don't also do a full rebuild on every change notification. rebuild_building_grid at 5K buildings: O(N) full rebuild → 1.5us constant.

### resolve_work_targets O(68K iter_instances) -> O(~1K ECS query) (#186)

**Root cause**: `resolve_work_targets` built `production_map` and `cow_farm_slots` by calling `iter_instances()` (68K building + resource node instances) with a per-instance `query.get(entity)` probe for each building. Ran unconditionally on every non-empty message batch. In-game peak: 4ms via `endless-cli get_perf`.

**Fix**: Replace `iter_instances()` scan with two ECS component queries:
- `Query<(&GpuSlot, &ProductionState), With<Building>>` -- only ~1K buildings with ProductionState
- `Query<(&GpuSlot, &FarmModeComp), With<Building>>` -- only farms with FarmModeComp
- Lazy gate: skip both queries entirely for Release/MarkPresent-only batches (common steady-state path)

**Pattern**: Migration Template #1 (Query-First Scan). When a hot path scans all instances to find ones with a specific component, replace the `iter_instances()` + `query.get()` pattern with a direct ECS query filtered by that component. O(68K) -> O(~1K).

**Benchmark**: `cargo bench --bench system_bench -- resolve_work_targets`. Bench covers burst_claim/500, burst_claim/2000, and burst_claim/1000_bld_65k_nodes (1K buildings + 65K TreeNode/RockNode instances to prove the query is O(~1K) at game scale, not O(all instances)).

### sync_pathfind_costs HPA* rebuild scoped to changed cells (#203)

**Root cause**: `sync_building_costs` passed ALL `building_cost_cells` to HPA* `rebuild_chunks` on every `BuildingGridDirtyMsg`. Even if only one building changed, it rebuilt HPA* chunks for ALL buildings. At 16x speed with frequent AI placement, this cost 3.05ms per tick (vs 0.02ms at 1x).

**Fix**: Snapshot `(idx, cost)` pairs before rebuild. After re-applying overlays, diff old vs new to find cells whose cost actually changed. Only pass changed cells to `rebuild_chunks`. Detects both set membership changes (added/removed buildings) AND cost value changes (wall replaced by road at same cell -- `symmetric_difference` would miss this).

**Pattern**: Before/after diff -- when an expensive downstream operation (HPA* rebuild) takes a set of dirty items, diff the actual values to avoid feeding unchanged items. No-change case (redundant dirty signal) skips HPA* entirely. sync_pathfind_costs at 200 buildings no-change: ~2.5us.

### ai_decision_system rate-limited to real-time cadence (#204)

**Root cause**: `ai_decision_system` advanced decision timers using `game_time.delta(&time)` (game-time-scaled delta). At 16x speed, each tick delta was 16x larger, so AI timers fired 16x more often per real-time second. Observed: 0.01ms at 1x -> 1.57ms at 16x (157x increase) for 447 NPCs.

**Fix**: Use `time.delta_secs()` (real-time delta) instead of `game_time.delta(&time)`. Added pause guard: skip timer advance when `game_time.paused()`. AI strategic decisions run at real-time cadence regardless of game speed -- decision quality does not improve from 16x more evaluations.

**Pattern**: Rate-limit in real-time -- AI decision timers should advance at wall-clock rate, not game-time rate. Strategic decisions (where to build, whom to attack) do not need to scale with simulation speed.

**Before/after**: 447 NPCs at 16x (issue baseline): 1.57ms/tick. After fix (BRP verified, ~313 NPCs): 0.02ms/tick at 1x, 0.33ms/frame at 16x (= 0.021ms/tick x 16 ticks). Per-tick cost is now constant regardless of game speed.

### sync_sleeping_system O(65K) -> O(dirty) event-driven (#195)

**Root cause**: `sync_sleeping_system` iterated all 65K NPC+building instances every tick to sync sleeping state. Ran unconditionally even when nothing changed.

**Fix**: Converted to message-driven: runs only when a dirty message is received. No-change frames skip entirely. Consistent with the event-driven pattern used for farm_visual, building_grid, and pathfind_costs.

**Pattern**: Event-driven -- poll → message-driven. When state changes are sparse, replace unconditional O(n) scan with a message-triggered system that fires only when needed.

### mason work targeting iter_instances -> for_each_nearby spatial query (#169)

**Root cause**: `decision_system` mason logic used `entity_map.iter_instances()` to find nearest damaged building -- O(all_instances) = O(~68K) per mason NPC per decision tick. With many mason NPCs at bucket cadence, cumulative cost was significant.

**Fix**: Replace `iter_instances()` scan with `entity_map.for_each_nearby(current_pos, repair_radius, ...)` -- only visits buildings within the spatial grid cell(s) overlapping the search radius. O(nearby) instead of O(all_instances).

**Pattern**: Migration Template #3 (Mixed Path) + spatial query -- when a hot decision loop scans all instances to find spatially nearby ones, replace with the pre-built spatial index. Benchmark added in system_bench.rs for mason 50K-building scale validation.

### road candidate scoring O(n*m) -> O(n) adjacency check (#218)

**Root cause**: Road candidate scoring in `ai_player` built adjacency lists via nested loops -- O(candidates * neighbors) = O(n*m) per AI build evaluation. At large town sizes with many road candidates, this dominated AI build step cost.

**Fix**: Replaced nested adjacency loop with a single pre-indexed pass. O(n) total.

**Pattern**: Pre-index -- when an inner loop re-scans the same collection for each outer item, build a lookup structure once and reuse it.

### damage_system debug sampling removed (#170)

**Root cause**: `damage_system` contained an `iter_npcs()` + `query.get()` debug sampling loop that ran unconditionally. The loop had no debug gate and was not visible in the hot-path audit. At 50K NPCs, this added unmeasured overhead to every damage tick.

**Fix**: Removed the unconditional debug sampling loop entirely. No replacement needed -- the data was only used for debug validation, not gameplay.

**Pattern**: Debug overhead removal -- unconditional per-frame `iter_npcs()` + `query.get()` loops in hot paths must be gated behind a debug flag or removed. See Debug Overhead rules.

## Benchmarks (2026-03-15 -- 306decc)

| System | 1K | 5K | 10K | 25K | 50K |
|--------|----|----|-----|-----|-----|
| decision | 47us | 76us | 118us | 225us | 396us |
| damage | 183us | 191us | 218us | 406us | 932us |
| healing | 12us | 42us | 84us | 227us | 493us |
| attack | 23us | 78us | 144us | 355us | 726us |
| resolve_movement | 19us | 46us | 63us | 117us | 221us |
| resolve_movement_unbounded | 33us | 86us | 158us | 375us | 766us |
| populate_gpu_state | 189us | 193us | 236us | 480us | 1035us |
| energy | 5us | 11us | 20us | 45us | 95us |
| arrival | 18us | 27us | 35us | 63us | 112us |
| gpu_position_readback | 5us | 17us | 32us | 76us | 152us |
| advance_waypoints | 5us | 15us | 25us | 58us | 113us |
| cooldown | 4us | 8us | 14us | 31us | 61us |
| npc_regen | 2us | 2us | 3us | 2us | 2us |
| on_duty_tick | 4us | 8us | 15us | 32us | 61us |

| Building system | 100 | 500 | 1K | 5K | 50K |
|-----------------|-----|-----|----|-----|------|
| building_tower | 15us | 20us | 28us | 65us | 644us |
| growth | 14us | 18us | 24us | 61us | 636us |
| construction_tick | 6us | 15us | 26us | 104us | 934us |
| spawner_respawn | 28us | 39us | 64us | 267us | 2063us |

| Death system | 1K | 50K |
|-------------|-----|------|
| death_system | 107us | 114us |
| death_pipeline | 1173us | 59748us |
| death_idle@50K | 100us | - |

Frame-capped at 2000 deaths/frame (DeathQueue). Mass deaths spread over multiple frames. Combat log and equipment clone gated behind mass_death threshold (>50 deaths/frame).

Combined 50K NPC-scaled (14 systems): 5.2ms (32.3% of 16ms budget)
Combined 50K all measured (realistic 2K buildings): 6.3ms (39.4% of 16ms budget)
