# SIMD/Assembly Optimization Targets

CPU-side SIMD optimization candidates for Endless at 50k NPC / 50k building / 1000x1000 map scale.

SIMD infrastructure exists in `rust/src/simd.rs` (AVX2 + scalar fallback + runtime dispatch). First experiment (gpu_position_readback) regressed -- see "Experiment Results" section and `docs/performance.md`.

## What is already on GPU (not candidates)

The WGSL compute shaders (`npc_compute.wgsl`, `projectile_compute.wgsl`) already handle:

- NPC movement/steering (boid separation, goal seeking)
- Spatial grid build (3-pass: clear, insert, query)
- Combat target selection (nearest enemy via spatial grid)
- Projectile physics and hit detection
- Threat counting (enemies/allies packed u32)

These run massively parallel on GPU. No CPU SIMD work can compete here.

## Priority 1: GPU Position Readback -- TESTED, REVERTED

**File:** `rust/src/systems/movement.rs:26-75` (`gpu_position_readback`)
**Frequency:** Every frame, every NPC (50k iterations)
**Result:** Two-pass SIMD regressed +89%. See "Experiment Results" below.
**Reason:** ECS iteration is the bottleneck (scattered writes), not arithmetic. The slot namespace (200k pre-allocated) makes batch processing wasteful when only 50k slots are live NPCs.

### Current code

```rust
for (es, mut pos, mut flags, path, _activity) in npc_q.iter_mut() {
    let i = es.0;
    if i * 2 + 1 >= positions.len() { continue; }

    let gpu_x = positions[i * 2];
    let gpu_y = positions[i * 2 + 1];
    if gpu_x < -9000.0 { continue; }

    pos.x = gpu_x;
    pos.y = gpu_y;

    if !flags.at_destination {
        if i * 2 + 1 < targets.len() {
            let goal_x = targets[i * 2];
            let goal_y = targets[i * 2 + 1];
            let dx = gpu_x - goal_x;
            let dy = gpu_y - goal_y;
            let dist_sq = dx * dx + dy * dy;
            let is_intermediate = path.current + 1 < path.waypoints.len();
            let thresh_sq = if is_intermediate { intermediate_sq } else { threshold_sq };
            if dist_sq <= thresh_sq { flags.at_destination = true; }
        }
    }
}
```

### Why it is a good target

- `positions` and `targets` are flat `Vec<f32>` -- contiguous memory, no gathers needed
- Inner loop is pure arithmetic: subtract, multiply, add, compare
- No branches in the hot math (the `is_intermediate` branch is per-NPC metadata, not data-dependent)
- 50k iterations per frame = enough work to amortize SIMD setup

### Implementation approach

Split into two passes to separate SIMD-friendly math from scattered ECS writes:

**Pass 1 (SIMD):** Batch arrival check on flat arrays.

```rust
// New file: rust/src/simd.rs

#[cfg(target_arch = "x86_64")]
use core::arch::x86_64::*;

/// Batch compute arrival flags for N entities.
/// positions/targets are interleaved [x0,y0,x1,y1,...] (same layout as GPU buffers).
/// Returns Vec<u8> where 1 = arrived, 0 = not arrived, 2 = skip (hidden).
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
pub unsafe fn batch_arrival_check_avx2(
    positions: &[f32],      // GPU readback positions, interleaved xy
    targets: &[f32],        // GPU target positions, interleaved xy
    threshold_sq: f32,
    count: usize,           // number of entities (not floats)
) -> Vec<u8> {
    let mut result = vec![0u8; count];
    let thresh = _mm256_set1_ps(threshold_sq);
    let hidden = _mm256_set1_ps(-9000.0);

    // Process 4 entities per iteration (each entity = 2 floats, 4 entities = 8 floats = 1 AVX reg)
    let chunks = count / 4;
    for chunk in 0..chunks {
        let base = chunk * 8; // 4 entities * 2 floats each
        if base + 7 >= positions.len() || base + 7 >= targets.len() { break; }

        let pos = _mm256_loadu_ps(positions.as_ptr().add(base));
        let tgt = _mm256_loadu_ps(targets.as_ptr().add(base));
        let delta = _mm256_sub_ps(pos, tgt);
        let delta_sq = _mm256_mul_ps(delta, delta);

        // Horizontal add pairs: [dx0*dx0+dy0*dy0, _, dx1*dx1+dy1*dy1, _, ...]
        let dist_sq = _mm256_hadd_ps(delta_sq, delta_sq);

        // Compare dist_sq <= threshold_sq
        let arrived = _mm256_cmp_ps(dist_sq, thresh, _CMP_LE_OQ);

        // Check hidden (pos.x < -9000)
        let hidden_mask = _mm256_cmp_ps(pos, hidden, _CMP_LT_OQ);

        // Extract masks and write results
        let arrived_bits = _mm256_movemask_ps(arrived);
        let hidden_bits = _mm256_movemask_ps(hidden_mask);

        // hadd puts results in slots 0,1,4,5 (AVX lane layout)
        let entity_base = chunk * 4;
        for e in 0..4usize {
            let bit_idx = match e { 0 => 0, 1 => 1, 2 => 4, 3 => 5, _ => unreachable!() };
            if hidden_bits & (1 << (e * 2)) != 0 {
                result[entity_base + e] = 2; // hidden
            } else if arrived_bits & (1 << bit_idx) != 0 {
                result[entity_base + e] = 1; // arrived
            }
        }
    }

    // Scalar remainder
    for i in (chunks * 4)..count {
        let base = i * 2;
        if base + 1 >= positions.len() || base + 1 >= targets.len() { continue; }
        if positions[base] < -9000.0 {
            result[i] = 2;
            continue;
        }
        let dx = positions[base] - targets[base];
        let dy = positions[base + 1] - targets[base + 1];
        if dx * dx + dy * dy <= threshold_sq {
            result[i] = 1;
        }
    }

    result
}

/// Scalar fallback for non-x86 targets.
pub fn batch_arrival_check_scalar(
    positions: &[f32],
    targets: &[f32],
    threshold_sq: f32,
    count: usize,
) -> Vec<u8> {
    let mut result = vec![0u8; count];
    for i in 0..count {
        let base = i * 2;
        if base + 1 >= positions.len() || base + 1 >= targets.len() { continue; }
        if positions[base] < -9000.0 {
            result[i] = 2;
            continue;
        }
        let dx = positions[base] - targets[base];
        let dy = positions[base + 1] - targets[base + 1];
        if dx * dx + dy * dy <= threshold_sq {
            result[i] = 1;
        }
    }
    result
}

/// Runtime dispatch.
pub fn batch_arrival_check(
    positions: &[f32],
    targets: &[f32],
    threshold_sq: f32,
    count: usize,
) -> Vec<u8> {
    #[cfg(target_arch = "x86_64")]
    {
        if is_x86_feature_detected!("avx2") {
            return unsafe {
                batch_arrival_check_avx2(positions, targets, threshold_sq, count)
            };
        }
    }
    batch_arrival_check_scalar(positions, targets, threshold_sq, count)
}
```

**Pass 2 (ECS):** Apply precomputed flags.

```rust
// In gpu_position_readback, replace the current loop:
let arrivals = batch_arrival_check(
    &gpu_state.positions,
    &buffer_writes.targets,
    threshold_sq,
    npc_count,
);

for (es, mut pos, mut flags, path, _activity) in npc_q.iter_mut() {
    let i = es.0;
    if i >= arrivals.len() { continue; }
    match arrivals[i] {
        2 => continue,  // hidden
        _ => {}
    }
    let base = i * 2;
    pos.x = gpu_state.positions[base];
    pos.y = gpu_state.positions[base + 1];

    if !flags.at_destination && arrivals[i] == 1 {
        // Still need per-NPC intermediate threshold check
        let is_intermediate = path.current + 1 < path.waypoints.len();
        if is_intermediate {
            // Re-check with relaxed threshold (scalar, only for intermediate waypoints)
            let dx = pos.x - buffer_writes.targets[base];
            let dy = pos.y - buffer_writes.targets[base + 1];
            if dx * dx + dy * dy <= intermediate_sq {
                flags.at_destination = true;
            }
        } else {
            flags.at_destination = true;
        }
    }
}
```

### Intermediate waypoint handling

The current code uses two thresholds: `threshold_sq` (final waypoint) and `intermediate_sq` (relaxed, for mid-path waypoints). The SIMD batch uses the tighter threshold. Entities that pass the tight check are definitely arrived. Entities that fail might still pass the relaxed check -- but we only need to scalar-check the small subset that are on intermediate waypoints and failed the tight check. This keeps the SIMD path simple.

## Priority 2: Path Cost Accumulation

**File:** `rust/src/systems/pathfinding.rs:123-145` (`accumulate_path_cost`)
**Frequency:** Per path found (dozens per frame), per waypoint in path, per spread cell
**Estimated speedup:** 2-4x

### Current code

```rust
pub fn accumulate_path_cost(
    costs: &mut [u16],
    width: usize,
    height: usize,
    path: &[IVec2],
    spread: i32,
    cost_add: u16,
) {
    for cell in path {
        for dy in -spread..=spread {
            for dx in -spread..=spread {
                let x = cell.x + dx;
                let y = cell.y + dy;
                if x >= 0 && y >= 0 && (x as usize) < width && (y as usize) < height {
                    let idx = y as usize * width + x as usize;
                    if costs[idx] > 0 {
                        costs[idx] = costs[idx].saturating_add(cost_add);
                    }
                }
            }
        }
    }
}
```

### Why it is a good target

- Inner loop operates on contiguous `u16` rows in a flat array
- `saturating_add` maps directly to `_mm256_adds_epu16` (single instruction, 16 elements)
- For spread=1, each row is 3 elements (not worth SIMD). For spread=2+, rows are 5+ elements
- At 1000x1000 map with many paths, this touches significant memory

### Implementation approach

For each path cell, process each row of the spread rectangle as a SIMD operation:

```rust
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn accumulate_row_avx2(costs: &mut [u16], start: usize, len: usize, cost_add: u16) {
    let add_vec = _mm256_set1_epi16(cost_add as i16);
    let zero = _mm256_setzero_si256();
    let mut offset = 0;

    // Process 16 u16s at a time
    while offset + 16 <= len {
        let ptr = costs.as_mut_ptr().add(start + offset) as *mut __m256i;
        let current = _mm256_loadu_si256(ptr);
        // Skip cells with cost==0 (impassable): mask them out
        let passable = _mm256_cmpgt_epi16(current, zero);
        let added = _mm256_adds_epu16(current, add_vec);
        // Blend: keep original where impassable, use added where passable
        let result = _mm256_blendv_epi8(current, added, passable);
        _mm256_storeu_si256(ptr, result);
        offset += 16;
    }

    // Scalar remainder
    for i in offset..len {
        let idx = start + i;
        if costs[idx] > 0 {
            costs[idx] = costs[idx].saturating_add(cost_add);
        }
    }
}
```

Then in `accumulate_path_cost`, for each path cell, compute the row bounds once and call the SIMD row function:

```rust
for cell in path {
    for dy in -spread..=spread {
        let y = cell.y + dy;
        if y < 0 || y as usize >= height { continue; }
        let x_min = (cell.x - spread).max(0) as usize;
        let x_max = ((cell.x + spread + 1) as usize).min(width);
        let row_start = y as usize * width + x_min;
        let row_len = x_max - x_min;
        accumulate_row(costs, row_start, row_len, cost_add);
    }
}
```

This only helps when `spread >= 8` (16 u16s per AVX2 register). For smaller spreads, the scalar version is fine. Use runtime dispatch based on spread size.

## Priority 3: A* Neighbor Expansion

**File:** `rust/src/systems/pathfinding.rs:56-84` (`pathfind_on_grid`)
**Frequency:** Thousands of node expansions per A* call, dozens of calls per frame
**Estimated speedup:** Modest (priority queue is the real bottleneck)

### Current code (inner closure)

```rust
|&pos| {
    let mut result = Vec::with_capacity(4);
    for d in NEIGHBOR_DIRS {  // [X, NEG_X, Y, NEG_Y]
        let np = pos + d;
        if let Some(cost) = neighbor_cost(grid, np) {
            result.push((np, cost));
        }
    }
    result
}
```

Each call to `neighbor_cost` does:
1. Bounds check: `x < 0 || y < 0 || x >= width || y >= height`
2. Array index: `costs[y * width + x]`
3. Zero check: `cost == 0` means impassable

### Implementation approach

Batch all 4 neighbors into a single SIMD operation:

```rust
/// Check 4 cardinal neighbors simultaneously. Returns (neighbor_positions, costs, valid_mask).
/// valid_mask bit i = 1 means neighbor i is in-bounds and passable.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "sse4.1")]
unsafe fn check_neighbors_sse(
    pos: IVec2,
    costs: &[u16],
    width: i32,
    height: i32,
) -> ([IVec2; 4], [u32; 4], u8) {
    // Neighbor positions: (x+1,y), (x-1,y), (x,y+1), (x,y-1)
    let nx = _mm_setr_epi32(pos.x + 1, pos.x - 1, pos.x, pos.x);
    let ny = _mm_setr_epi32(pos.y, pos.y, pos.y + 1, pos.y - 1);

    // Bounds: 0 <= nx < width AND 0 <= ny < height
    let zero = _mm_setzero_si128();
    let w = _mm_set1_epi32(width);
    let h = _mm_set1_epi32(height);

    // x >= 0 && x < width && y >= 0 && y < height
    // Using signed comparison: all must be >= 0 and < bound
    let x_ge_0 = _mm_cmpgt_epi32(nx, _mm_set1_epi32(-1)); // x >= 0 as x > -1
    let x_lt_w = _mm_cmplt_epi32(nx, w);
    let y_ge_0 = _mm_cmpgt_epi32(ny, _mm_set1_epi32(-1));
    let y_lt_h = _mm_cmplt_epi32(ny, h);
    let in_bounds = _mm_and_si128(
        _mm_and_si128(x_ge_0, x_lt_w),
        _mm_and_si128(y_ge_0, y_lt_h),
    );

    let mask = _mm_movemask_ps(_mm_castsi128_ps(in_bounds)) as u8;

    let mut positions = [IVec2::ZERO; 4];
    let mut out_costs = [0u32; 4];
    let mut valid = 0u8;

    // Extract and lookup costs for valid neighbors
    let nx_arr: [i32; 4] = core::mem::transmute(nx);
    let ny_arr: [i32; 4] = core::mem::transmute(ny);

    for i in 0..4 {
        if mask & (1 << i) != 0 {
            let idx = ny_arr[i] as usize * width as usize + nx_arr[i] as usize;
            let c = costs[idx];
            if c > 0 {
                positions[i] = IVec2::new(nx_arr[i], ny_arr[i]);
                out_costs[i] = c as u32;
                valid |= 1 << i;
            }
        }
    }

    (positions, out_costs, valid)
}
```

This eliminates 4 separate bounds-check branches per node expansion. The cost lookups still require scalar gather (u16 at computed indices), but the bounds check is fully vectorized.

**Note:** The real A* bottleneck is the binary heap operations in the `pathfinding` crate, not the neighbor expansion. This optimization is worth implementing but has diminishing returns. A custom A* with a bucket queue would help more than SIMD here.

## Priority 4: Combat Target Candidate Scan

**File:** `rust/src/systems/combat.rs:106-149` (`pick_npc_target`)
**Frequency:** Per fighting NPC whose GPU target is retreating (gated by fast path)
**Estimated speedup:** 4-6x on scan, but limited real-world impact

### Current code

```rust
for c in candidates {
    if c.faction == attacker_faction || c.faction == FACTION_NEUTRAL || c.x < -9000.0 {
        continue;
    }
    let dx = c.x - attacker_pos.x;
    let dy = c.y - attacker_pos.y;
    if dx * dx + dy * dy > range_sq {
        continue;
    }
    let s = score_of(c);
    if s < best_score {
        best_score = s;
        best_slot = c.slot;
    }
}
```

### Implementation approach

Restructure `TargetCandidate` storage from AoS to SoA for SIMD-friendly access:

```rust
/// SoA layout for SIMD-friendly target scanning.
pub struct TargetCandidatesSoA {
    pub x: Vec<f32>,
    pub y: Vec<f32>,
    pub slot: Vec<usize>,
    pub faction: Vec<i32>,
    pub is_retreating: Vec<u8>,
    pub health: Vec<f32>,
}
```

Then scan 8 candidates at a time with AVX2:

```rust
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn scan_targets_avx2(
    candidates: &TargetCandidatesSoA,
    attacker_x: f32,
    attacker_y: f32,
    range_sq: f32,
    attacker_faction: i32,
) -> Option<(usize, (u8, f32))> {
    let ax = _mm256_set1_ps(attacker_x);
    let ay = _mm256_set1_ps(attacker_y);
    let rsq = _mm256_set1_ps(range_sq);
    let af = _mm256_set1_epi32(attacker_faction);
    let neutral = _mm256_set1_epi32(0); // FACTION_NEUTRAL
    let hidden = _mm256_set1_ps(-9000.0);

    let mut best_slot = usize::MAX;
    let mut best_score = (1u8, f32::MAX);
    let n = candidates.x.len();

    let chunks = n / 8;
    for chunk in 0..chunks {
        let base = chunk * 8;

        // Load 8 x and y coordinates
        let cx = _mm256_loadu_ps(candidates.x.as_ptr().add(base));
        let cy = _mm256_loadu_ps(candidates.y.as_ptr().add(base));

        // Distance check
        let dx = _mm256_sub_ps(cx, ax);
        let dy = _mm256_sub_ps(cy, ay);
        let d2 = _mm256_add_ps(_mm256_mul_ps(dx, dx), _mm256_mul_ps(dy, dy));
        let in_range = _mm256_cmp_ps(d2, rsq, _CMP_LE_OQ);

        // Hidden check
        let not_hidden = _mm256_cmp_ps(cx, hidden, _CMP_GE_OQ);

        // Faction filter (need integer compare)
        let factions = _mm256_loadu_si256(candidates.faction.as_ptr().add(base) as *const __m256i);
        let not_same = _mm256_andnot_si256(
            _mm256_cmpeq_epi32(factions, af),
            _mm256_set1_epi32(-1),
        );
        let not_neutral = _mm256_andnot_si256(
            _mm256_cmpeq_epi32(factions, neutral),
            _mm256_set1_epi32(-1),
        );

        // Combined mask
        let valid = _mm256_movemask_ps(_mm256_and_ps(
            _mm256_and_ps(in_range, not_hidden),
            _mm256_castsi256_ps(_mm256_and_si256(not_same, not_neutral)),
        ));

        // Scalar extraction of survivors
        if valid != 0 {
            for bit in 0..8 {
                if valid & (1 << bit) != 0 {
                    let idx = base + bit;
                    let s = (candidates.is_retreating[idx], candidates.health[idx]);
                    if s < best_score {
                        best_score = s;
                        best_slot = candidates.slot[idx];
                    }
                }
            }
        }
    }

    // Scalar remainder
    // ... (same pattern as current code for remaining elements)

    if best_slot != usize::MAX { Some((best_slot, best_score)) } else { None }
}
```

**Trade-off:** This requires converting the existing `Vec<TargetCandidate>` (AoS) to SoA layout each frame. At 50k candidates, the conversion itself costs ~200us. Only worth it if many NPCs need the scan (large battles with retreating enemies). Consider keeping both layouts or switching to SoA permanently for the candidate list build in `attack_system`.

## Not worth optimizing

### Decision system (`decision/mod.rs`, 3044 lines)

Priority-based state machine. Each NPC branches differently based on activity, combat state, energy, job type, squad membership, etc. No vectorizable inner loop. Assembly provides zero benefit for branch-heavy per-entity logic.

### Energy system (`energy.rs`)

```rust
energy.0 = (energy.0 - drain * hours).max(0.0);
```

Single multiply-add per NPC. The ECS iteration overhead (archetype matching, component access) dominates the actual arithmetic. SIMD on the math saves nanoseconds that are invisible next to the ECS cost.

### Line of sight (`pathfinding.rs:787-813`)

Bresenham ray march. Each step depends on the error accumulator from the previous step -- inherently serial. Only used for short-distance LOS bypass (manhattan <= `short_distance_tiles`). Not a bottleneck.

### Spatial queries (`entity_map.rs:740-838`)

Already use kind-filtered spatial grid with cell-ring expansion. The inner loops iterate small buckets (typically 1-5 buildings per cell). Not enough work per cell to justify SIMD.

## Module structure

```
rust/src/
  simd.rs              -- SIMD routines with scalar fallbacks
  simd/
    arrival.rs         -- batch_arrival_check (priority 1)
    pathcost.rs        -- accumulate_row (priority 2)
    neighbors.rs       -- check_neighbors (priority 3)
    combat_scan.rs     -- scan_targets (priority 4)
```

Each submodule exports:
- `#[target_feature(enable = "avx2")]` fast path
- Scalar fallback
- Public dispatch function with runtime feature detection

## Build requirements

- Rust nightly not required (`core::arch` intrinsics are stable)
- `#[target_feature]` functions must be `unsafe` (caller ensures feature availability)
- Runtime detection via `is_x86_feature_detected!("avx2")` (stable, cached after first call)
- No additional crate dependencies needed

## Testing strategy

1. All existing tests pass unchanged (`k3sc cargo-lock test`)
2. Property tests: scalar and SIMD produce identical results for random inputs
3. Edge cases: empty arrays, count=0, positions shorter than targets, hidden entities
4. Criterion benchmarks at 50k scale for each SIMD function
5. In-game Tracy frame time comparison (requires local GPU machine, not k3s)

## Experiment Results (2026-04-04)

### gpu_position_readback: Two-Pass SIMD (REVERTED)

**Hypothesis:** Batch AVX2 arrival check on flat f32 arrays, then apply to ECS. Expected 4-8x on arithmetic, ~15% frame improvement.

**Standalone SIMD microbench (batch_arrival_check only):**

| Count | Scalar | AVX2 | Speedup |
|-------|--------|------|---------|
| 1k | 1.04us | 0.61us | 1.7x |
| 10k | 10.2us | 6.0us | 1.7x |
| 50k | 50.9us | 32.4us | 1.6x |
| 100k | 105.5us | 64.3us | 1.6x |

**Full system benchmark (gpu_position_readback with ECS):**

| Config | Baseline | Two-pass SIMD | Delta |
|--------|----------|---------------|-------|
| 50k NPCs, tiny grid | 159us | 301us | +89% |
| 50k NPCs + 50k buildings, 1000x1000 | 160us | 315us | +97% |

**Why it failed:**

1. Slot namespace is 200k (MAX_ENTITIES). Positions buffer is pre-allocated to full size. NPC GpuSlot values are non-contiguous (buildings get lower slots from the LIFO pool). Must process entire 200k buffer to cover all NPCs = ~128us batch overhead.

2. The distance arithmetic (sub, mul, add, cmp) is only ~10-20us of the original 160us. The remaining 140us is ECS query iteration, component access, and scattered cache-line writes to Position/NpcFlags. SIMD cannot help with scattered memory access.

3. Per-frame `Vec<u8>` allocation (200k bytes) adds memset overhead.

**Conclusion:** SIMD two-pass is wrong for ECS-iteration-bound systems. The SIMD module and benchmarks are retained as infrastructure. Better targets are pure-data operations on contiguous arrays (accumulate_path_cost, SoA candidate scans) where ECS overhead is not a factor.

## Research Findings (2026-04-04)

External research confirms the experiment results: CPU SIMD is a dead end for ECS-iteration-bound systems. Smarter engineers solved these problems differently.

### GPU already computes arrivals -- CPU double-computes

`npc_compute.wgsl` binding 5 (`arrivals`) tracks per-entity settled state on GPU. The `gpu_position_readback` system then re-computes arrival via CPU distance check. Priority 1 tried to SIMD-optimize a computation that already happens on GPU. The fix is reading the GPU arrivals buffer back instead of recomputing. See roadmap.md for tracking.

### `accumulate_path_cost` is dead code

Defined at `pathfinding.rs:123` but never called. Priority 2 has no real workload to optimize. Wire it in first before considering SIMD.

### Priority 3 (A* neighbors) is likely autovectorized

LLVM will vectorize 4 bounds checks on its own. Verify with `cargo asm` before writing manual intrinsics. A bucket queue for the A* priority queue would help more than SIMD on neighbor expansion.

### Priority 4 break-even is narrow

AoS-to-SoA conversion costs ~200us/frame. The scan itself is ~20-30us scalar at 50k candidates. Need 7+ NPCs scanning per frame to break even. Only viable in large battles with many retreating enemies.

### `_mm256_cmpgt_epi16` is signed (Priority 2 bug)

The proposed AVX2 `accumulate_row` uses `_mm256_cmpgt_epi16` which is signed comparison. u16 cost values above 32767 would be treated as negative, skipping valid cells. If costs can exceed 32767, need XOR bias with 0x8000 before comparing.

### Key external references

- [NativeFlowField](https://github.com/kingstone426/NativeFlowField) -- GPU compute flow fields, multi-pass distance propagation, async readback (Unity DOTS, algorithm is engine-agnostic)
- [nullprogram GPU pathfinding](https://nullprogram.com/blog/2014/06/22/) -- breadth-first flood on GPU, O(grid) shared across all agents
- [Dreaming381 "Your ECS Probably Still Sucks"](https://gist.github.com/Dreaming381/89d65f81b9b430ffead443a2d430defc) -- if bottleneck is ECS iteration, SIMD on arithmetic is wasted; restructure storage or move off CPU
- [Bevy batched ECS query discussion](https://github.com/bevyengine/bevy/issues/1990) -- AoSoA layout wins benchmarks but Bevy doesn't expose it yet
- [bevy_hanabi indirect dispatch](https://deepwiki.com/djeedai/bevy_hanabi) -- zero CPU-GPU sync via GPU-computed dispatch counts

## Revised priorities for 50k+ scale

SIMD is deprioritized. These architectural changes are the path to 100k+, ordered by impact:

1. **Stop double-computing arrivals.** Read GPU `arrivals` buffer back alongside positions. Delete the CPU distance check entirely. Pure subtraction of work. See roadmap.md.

2. **GPU flow fields.** Replace per-NPC CPU A* with GPU-computed flow field. Single compute dispatch floods the grid from each destination. NPCs sample a direction vector instead of following waypoints. Shared across all NPCs heading to the same target. Existing spatial grid WGSL infrastructure can be extended.

3. **GPU decision offload.** Move simple state transitions (energy threshold, arrival->idle) to compute shaders as GPU-side flags. CPU reads back as-needed rather than per-frame.

4. **Reduced readback architecture.** Follow bevy_hanabi's pattern: GPU computes its own dispatch counts via indirect dispatch, draws via indirect draw. Zero CPU-GPU sync points except initial allocation.

SIMD module (`simd.rs`) is retained as infrastructure. Viable future targets are pure-data operations on contiguous arrays where ECS overhead is not a factor.
