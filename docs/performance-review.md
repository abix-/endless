# Performance Review Checklist

Use this checklist during PR review for code that runs per frame, per tick, per NPC, or per building.

## Hybrid Data Access Rule

Use a hybrid access pattern by default:

1. Use Bevy `Query` for hot filtered scans (per-frame/per-tick loops).
2. Use `EntityMap` for keyed lookup/index access (`slot -> entity`, grid/cell occupancy, kind/town indexes, spatial helpers).
3. Do not replace keyed `EntityMap` lookups with full ECS scans.
4. Do not replace hot filtered query scans with full `EntityMap` NPC/building scans.

## Canonical Key Model

Treat `slot` as the canonical foreign key between ECS and `EntityMap`.

1. Canonical identity key: `slot` (`EntitySlot` in ECS).
2. Runtime handle: `Entity` (ephemeral; not persistence identity).
3. Required bridge: `slot <-> Entity` mapping stays synchronized.
4. Uniqueness rule: NPCs and buildings share one slot namespace; a slot value cannot be owned by both at the same time.
5. Secondary indexes are allowed for performance (`Entity -> slot`, grid cell, kind+town, spatial buckets), but all must resolve to the same canonical `slot`.

## Scope

Apply the hybrid rule to any runtime path that is expected to scale with population or map size:

1. `EguiPrimaryContextPass` systems and inspector/overlay rendering code.
2. `Update` systems in active gameplay states (`Playing` / `Running`), especially Behavior/AI/Combat/Economy loops.
3. Any helper called from the above paths that may iterate NPC/building sets.

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
6. Do not use `entity_map.iter_npcs()` plus per-item ECS `Query.get(...)` in per-frame/per-tick hot loops; use query-first iteration over ECS components instead.
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

1. Mark hot paths touched by the PR (`EguiPrimaryContextPass`, `Update` systems in active sets, AI/behavior loops).
2. For each hot path, note collection sizes and complexity (`O(n)`, `O(n^2)` risks).
3. Flag any repeated scans/membership checks and propose concrete replacement.
4. Add/adjust microbenchmarks for modified hotspots.
5. Confirm no new unconditional debug/string work in tight loops.
6. Confirm no stale architecture comments remain after migration work (comments must match current ownership model).
7. Confirm read-only systems do not request mutable queries/resources unless needed.

## Benchmark/Guardrail Expectations

1. Add microbenchmarks for hotspot helpers when introducing or changing their logic.
2. Keep baseline numbers for representative counts (small, medium, stress).
3. Fail CI on material regressions (for example, >20 percent for benchmarked hotspots).
4. Document benchmark command and expected range in the PR.

## Current Known Hotspot Patterns

- UI inspector paths doing repeated slot lookups across multiple queries in a single frame.
- Squad/selection flows using `Vec::contains` within nested loops.
- Overlay target dedupe using per-target linear scans.
- Cleanup/reassignment systems scanning full queries repeatedly instead of pre-indexing.
- Decision system still uses clone-local-then-writeback for ~15 component fields per NPC (query-first outer loop eliminates HashMap scan, but inner `get_mut(entity)` random access + clone/writeback remains).
- `game_hud.rs` and `health.rs` death detection still use `entity_map.iter_npcs()` due to SystemParam borrow conflicts with existing bundles (`BuildingInspectorData`, `DeathResources`).
