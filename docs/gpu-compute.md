# GPU Compute

## Overview

GPU compute uses Bevy's render graph with wgpu/WGSL. The compute shader `shaders/npc_compute.wgsl` runs NPC movement physics on the GPU. NPC rendering uses a separate instanced draw pipeline in `npc_render.rs` via Bevy's RenderCommand pattern. See [frame-loop.md](frame-loop.md) for where these fit in the frame.

## Architecture

```
Main World (ECS)                       Render World (GPU)
│                                      │
├─ NpcGpuData ───────────────────────▶ ExtractResource (cloned each frame)
├─ NpcComputeParams ─────────────────▶ ExtractResource (cloned each frame)
├─ NpcGpuState ──────────────────────▶ Extract<Res<T>> (zero-clone immutable read, per-buffer dirty flags)
├─ NpcVisualUpload ──────────────────▶ Extract<Res<T>> (zero-clone immutable read)
├─ NpcSpriteTexture (char+world+heal+sleep+arrow+building) ▶ ExtractResource
├─ GpuReadState ─────────────────────────▶ main-world only (no extraction)
├─ ProjPositionState ────────────────────▶ Extract<Res<T>> (zero-clone, for extract_proj_data)
│                                      │
│                                      ├─ init_npc_compute_pipeline (RenderStartup)
│                                      │   └─ Create GPU buffers (no staging — Bevy Readback handles it)
│                                      │
│                                      ├─ extract_npc_data (ExtractSchedule)
│                                      │   ├─ GPU-authoritative (positions/arrivals): per-dirty-index write_buffer
│                                      │   ├─ CPU-authoritative (targets/speeds/factions/healths/flags): bulk write_buffer
│                                      │   └─ Visual + equip arrays: unconditional bulk write_buffer
│                                      │
│                                      ├─ prepare_npc_bind_groups (PrepareBindGroups)
│                                      │   └─ 3 bind groups (one per mode, different uniform)
│                                      │
│                                      ├─ NpcComputeNode (render graph, 3 passes)
│                                      │   ├─ Mode 0: clear grid (atomicStore 0)
│                                      │   ├─ Mode 1: build grid (atomicAdd NPC indices)
│                                      │   ├─ Mode 2: separation + movement + combat targeting
│                                      │   └─ copy positions + combat_targets + factions + healths + threat_counts → ReadbackHandles assets
│                                      │
│                                      └─ Bevy Readback (async, managed by Bevy)
│                                          ReadbackComplete observers fire when GPU data ready:
│                                          ├─ npc_positions → GpuReadState.positions
│                                          ├─ combat_targets → GpuReadState.combat_targets
│                                          ├─ npc_factions → GpuReadState.factions
│                                          ├─ npc_health → GpuReadState.health
│                                          ├─ threat_counts → GpuReadState.threat_counts
│                                          ├─ proj_hits → ProjHitState.0
│                                          └─ proj_positions → ProjPositionState.0
│
└─ gpu_position_readback (after Drain)
      GpuReadState → ECS Position components
      + CPU-side arrival detection (position vs goal within ARRIVAL_THRESHOLD → AtDestination)
```

## Data Flow

```
ECS → GPU (upload):
  GpuUpdateMsg → collect_gpu_updates → GPU_UPDATE_QUEUE
    → populate_gpu_state → NpcGpuState (per-buffer dirty flags + per-index tracking for GPU-authoritative buffers)

  build_visual_upload (PostUpdate, chained after populate_gpu_state):
    Single O(N) pass: ECS query + NpcGpuState → NpcVisualUpload
    Packs visual_data [f32;8] + equip_data [f32;24] per NPC in GPU-ready format
    Both arrays reset to -1.0 sentinel each frame; phantom slots (waypoints) have no ECS entity and stay hidden

  extract_npc_data (ExtractSchedule, zero-clone):
    Extract<Res<NpcGpuState>> → hybrid writes to NpcGpuBuffers:
      GPU-authoritative (positions/arrivals): per-dirty-index write_buffer (~10-50 calls/frame)
      CPU-authoritative (targets/speeds/factions/healths/flags): 1 bulk write_buffer per dirty buffer
    Extract<Res<NpcVisualUpload>> → bulk write_buffer to NpcVisualBuffers

GPU → ECS (readback, Bevy async Readback):
  NpcComputeNode: dispatch compute + copy positions/combat_targets/factions/healths/threat_counts → ReadbackHandles ShaderStorageBuffer assets
  ProjectileComputeNode: copy hits/positions → ReadbackHandles ShaderStorageBuffer assets
    → Bevy Readback entities async-read buffers, fire ReadbackComplete observers:
      npc_positions → GpuReadState.positions
      combat_targets → GpuReadState.combat_targets
      npc_factions → GpuReadState.factions
      npc_health → GpuReadState.health
      threat_counts → GpuReadState.threat_counts
      proj_hits → ProjHitState.0
      proj_positions → ProjPositionState.0
    → gpu_position_readback: GpuReadState → ECS Position components
      + arrival detection: if HasTarget && dist(pos, goal) < ARRIVAL_THRESHOLD → AtDestination
  Data is 1 frame old (~1.6px drift at 100px/s). ARRIVAL_THRESHOLD=8px >> drift.
  npc_count not set from readback (buffer is MAX-sized) — comes from SlotAllocator.count().

GPU → Render:
  Vertex shader reads positions/health directly from NpcGpuBuffers storage buffers (bind group 2).
  NpcVisualBuffers (visual [f32;8] + equip [f32;24]) written by extract_npc_data during Extract.
    → DrawNpcStorageCommands: 7 draw calls via instance offset encoding (body + 6 equipment layers)
    → DrawMiscCommands: farms/BHP via InstanceData
```

Note: `sprite_indices` and `flash_values` live in `NpcGpuState`. Colors and equipment are derived from ECS components by `build_visual_upload` each frame, which packs them into `NpcVisualUpload` (visual_data + equip_data). These are uploaded to NPC visual/equipment storage buffers (`NpcVisualBuffers`) for the render shader — not to compute shader buffers. Positions and health for rendering come directly from compute output (`NpcGpuBuffers.positions`, `.healths`) via storage buffer binding, not via readback. `NpcGpuState` and `NpcVisualUpload` are read during Extract via `Extract<Res<T>>` — zero-clone immutable access, no ExtractResourcePlugin.

## NPC Compute Shader (npc_compute.wgsl)

Workgroup size: 64 threads. 3 dispatches per frame with different `mode` uniform values. Each mode dispatches `ceil(count / 64)` workgroups (mode 0 uses `ceil(grid_cells / 64)`).

### Mode 0: Clear Grid
One thread per grid cell. Atomically clears `grid_counts[cell]` to 0. Early exit if `i >= grid_cells`.

### Mode 1: Build Grid
One thread per NPC. Computes cell from `floor(pos / cell_size)`, atomically increments `grid_counts[cell]`, writes NPC index into `grid_data[cell * max_per_cell + slot]`. Skips hidden NPCs (pos.x < -9000).

### Mode 2: Separation + Movement + Combat Targeting
One thread per NPC. Three-tier optimization based on NPC type:

**Tier 1 — Buildings (speed=0, not tower)**: Early exit. Writes `combat_targets[i] = -1`, `threat_counts[i] = 0`, returns. Buildings are in the grid for projectile collision only — they don't move, separate, or target.

**Tier 1b — Tower buildings (speed=0, npc_flags bit 1 = 1)**: Skip movement/separation but run combat targeting. Fountains use this tier — GPU finds their nearest enemy, CPU `building_tower_system` reads `combat_targets[slot]` and fires projectiles.

**Tier 2 — Non-combatants (npc_flags bit 0 = 0, farmers/miners)**: Full separation + movement. Threat scan uses `threat_radius` (7×7=49 cells). Skips the expensive combat targeting scan (9×9=81 cells). Writes `combat_targets[i] = -1`.

**Tier 3 — Combatants (npc_flags bit 0 = 1, archers/raiders/fighters)**: Full separation + movement + combat targeting. Scans `combat_range` radius (9×9=81 cells) for both threat assessment and nearest enemy targeting.

Four phases per thread (tiers 2+3):

**Separation + dodge** (single 3x3 grid scan): For each neighbor within `separation_radius`, computes push-away force proportional to overlap. **Skips neighbors with speed=0** (buildings are collision-only, no separation force). Asymmetric push: moving NPCs (settled=0) push through settled ones (0.2x strength), settled NPCs get shoved by movers (2.0x). Same-faction neighbors get 1.5x push to spread out convoys. Exact overlaps use golden angle spread. Dodge is computed in the same loop: for moving NPCs approaching other moving NPCs within 2x `separation_radius`, dodges perpendicular to movement direction. Detects head-on (0.5), crossing (0.4), and overtaking (0.3) scenarios via dot-product convergence check. Consistent side-picking via index comparison (`i < j`). Dodge scaled by `strength * 0.7`. Total avoidance clamped to `speed * 1.5` to prevent wild overshoot.

**Projectile dodge** (spatial grid scan): After separation, scans 3x3 neighborhood of the projectile spatial grid (built by projectile compute modes 0+1 in the previous frame). For each enemy projectile within 60px heading toward the NPC (approach dot > 0.3), computes a perpendicular dodge force. Direction is away from the projectile's path (consistent side-picking via `select`). Urgency scales linearly with proximity (closer = stronger). Normalized and scaled to `speed * 1.5`. Applied as a separate force in the position update (`movement + avoidance + proj_dodge`), independent of avoidance clamping. 1-frame latency is acceptable: at 60fps, an arrow at speed 500 moves ~8px — within the 60px dodge radius.

**Movement with lateral steering**: Moves toward goal at full speed (no backoff persistence penalty). When avoidance pushes against the goal direction (alignment < -0.3), the NPC steers laterally (perpendicular to goal, in the direction avoidance is pushing) at 60% speed instead of slowing down. This routes NPCs around obstacles rather than jamming them. Backoff increments +1 when blocked, decrements -3 when clear, cap at 30.

**Combat targeting + threat assessment**: Scan radius depends on tier — `combat_range` (300px, 9×9 cells) for combatants, `threat_radius` (200px, 7×7 cells) for non-combatants. For each NPC in neighboring cells, checks: alive (health > 0), not self, **not a building (speed > 0)**. Buildings are excluded from both combat targeting and threat counts — building attacks are handled CPU-side via `find_nearest_enemy_building()`. Faction -1 (neutral) is treated as same-faction — never targeted, never counted as enemy. Combat targeting (tier 3 only) tracks nearest enemy by squared distance → `combat_targets[i]` (-1 if none or non-combatant). Threat assessment counts enemies and allies within `threat_radius`, packs both into a single u32 → `threat_counts[i]` as `(enemies << 16) | allies`. CPU decision_system unpacks these for flee threshold calculations.

## GPU Buffers

### Compute Buffers (gpu.rs NpcGpuBuffers)

Created once in `init_npc_compute_pipeline`. All storage buffers are `read_write`. GPU→CPU readback uses Bevy's async `Readback` + `ReadbackComplete` pattern via `ShaderStorageBuffer` assets (no manual staging buffers).

| Binding | Name | Type | Per-NPC Size | Uploaded From | Purpose |
|---------|------|------|-------------|---------------|---------|
| 0 | positions | vec2\<f32\> | 8B | NpcGpuState.positions | Current XY, read/written by shader. Init: -9999 sentinel (hidden). GPU-authoritative: per-index writes only. |
| 1 | goals | vec2\<f32\> | 8B | NpcGpuState.targets | Movement target |
| 2 | speeds | f32 | 4B | NpcGpuState.speeds | Movement speed |
| 3 | grid_counts | atomic\<i32\>[] | — | Not uploaded | NPCs per grid cell (atomically written by mode 0+1) |
| 4 | grid_data | i32[] | — | Not uploaded | NPC indices per cell (written by mode 1) |
| 5 | arrivals | i32 | 4B | NpcGpuState.arrivals | Settled flag (0=moving, 1=arrived), reset on SetTarget |
| 6 | backoff | i32 | 4B | Not uploaded | TCP-style collision backoff counter (read/written by mode 2) |
| 7 | factions | i32 | 4B | NpcGpuState.factions | -1=Neutral (unspawned/world buildings), 0=Player, 1+=AI. Init: -1. Neutral treated as same-faction in combat targeting + projectile collision. COPY_SRC for readback. |
| 8 | healths | f32 | 4B | NpcGpuState.healths | Current HP (COPY_SRC for readback) |
| 9 | combat_targets | i32 | 4B | Not uploaded | Nearest enemy index or -1 (written by shader, init -1) |
| 10 | params | Params (uniform) | — | NpcComputeParams | Count, delta (0 when paused), grid config, thresholds |
| 11 | proj_grid_counts | i32[] | — | ProjGpuBuffers.grid_counts (read) | Projectile spatial grid cell counts |
| 12 | proj_grid_data | i32[] | — | ProjGpuBuffers.grid_data (read) | Projectile indices per cell |
| 13 | proj_positions | vec2\<f32\>[] | — | ProjGpuBuffers.positions (read) | Projectile positions for dodge |
| 14 | proj_velocities | vec2\<f32\>[] | — | ProjGpuBuffers.velocities (read) | Projectile velocities for approach check |
| 15 | proj_factions | i32[] | — | ProjGpuBuffers.factions (read) | Projectile factions for friendly fire skip |
| 16 | threat_counts | u32 | 4B | Not uploaded | Packed threat assessment: (enemies << 16 \| allies) per NPC |
| 17 | npc_flags | u32 | 4B | NpcGpuState.npc_flags | Bit 0: combat scan enabled (archers/raiders/fighters). Bit 1: tower building (skip movement, run combat targeting). Fountains = 3 (bits 0+1). Other buildings + farmers/miners = 0. |

### NPC Visual Storage Buffers (npc_render.rs)

Uploaded per frame by `extract_npc_data` (ExtractSchedule) to `NpcVisualBuffers` via `queue.write_buffer()`. Positions and health read directly from compute output via bind group 2.

**Visual buffer** (`[f32; 8]` per slot, 32B/NPC):

| Offset | Field | Source |
|--------|-------|--------|
| 0 | sprite_col | NpcGpuState.sprite_indices[i*4] |
| 1 | sprite_row | NpcGpuState.sprite_indices[i*4+1] |
| 2 | body_atlas | NpcGpuState.sprite_indices[i*4+2] |
| 3 | flash | NpcGpuState.flash_values[i] (decays at 5.0/s) |
| 4-7 | r, g, b, a | Derived from ECS Faction/Job by build_visual_upload |

**Equipment buffer** (`[f32; 24]` per slot = 6 layers × `[col, row, atlas, _pad]`, 96B/NPC):
Built by `build_visual_upload` from ECS components (EquippedArmor, EquippedHelmet, EquippedWeapon, Activity, Healing). `col < 0` = unequipped.

## Uniform Params (NpcComputeParams)

| Field | Default | Purpose |
|-------|---------|---------|
| count | 0 | NPC slot high-water mark (set from SlotAllocator.count() each frame) |
| separation_radius | 20.0 | Minimum distance NPCs try to maintain |
| separation_strength | 100.0 | Repulsion force multiplier |
| delta | 0.016 | Frame delta time |
| grid_width | 256 | Spatial grid columns |
| grid_height | 256 | Spatial grid rows |
| cell_size | 128.0 | Pixels per grid cell |
| max_per_cell | 48 | Max NPCs per cell |
| arrival_threshold | 8.0 | Distance to mark as arrived |
| mode | 0 | Dispatch mode (0=clear grid, 1=build grid, 2=separation+movement+targeting) |
| combat_range | 300.0 | Maximum distance for combat targeting |
| proj_max_per_cell | 48 | Max projectiles per spatial grid cell (for dodge scan) |
| dodge_unlocked | 0 | Whether projectile dodge is enabled (tech tree unlock) |
| threat_radius | 200.0 | Radius for threat assessment enemy/ally counting |

## Spatial Grid

Built on GPU each frame via 3-mode dispatch with atomic operations:

- **Cell size**: 128px
- **Grid dimensions**: 256x256 (covers 32,768×32,768 world — supports up to 1000×1000 grid at 32px cells)
- **Max per cell**: 48
- **Total cells**: 65,536
- **Memory**: grid_counts = 256KB, grid_data = 12MB

NPCs are binned by `floor(pos / cell_size)`. Mode 0 clears all cell counts, mode 1 inserts NPCs via `atomicAdd`, mode 2 uses 3x3 neighborhood for separation/dodge forces and `combat_range / cell_size + 1` radius for combat targeting.

## NPC Rendering

Separate from compute. Uses `npc_render.rs` with Bevy's RenderCommand pattern hooked into the Transparent2d phase. Two render paths share one pipeline:

- **Storage buffer path** (NPCs): `vertex_npc` reads positions/health directly from `NpcGpuBuffers` compute output + visual/equip from `NpcVisualBuffers`. 7 draw calls with instance offset encoding (body + 6 equipment layers).
- **Instance buffer path** (farms, BHP, projectiles): `vertex` reads from classic `InstanceData` vertex attributes.

The render shader (`shaders/npc_render.wgsl`) shares `calc_uv()`, `world_to_clip()`, and fragment shader between both paths. Fragment shader handles alpha discard, color tinting, health bars, damage flash, and per-atlas texture sampling.

## Constants

```rust
const WORKGROUP_SIZE: u32 = 64;
const MAX_NPCS: u32 = 100000;
const GRID_WIDTH: u32 = 256;
const GRID_HEIGHT: u32 = 256;
const MAX_PER_CELL: u32 = 48;
```

## Known Issues

- **Health is CPU-authoritative**: GPU reads health for targeting but never modifies it.
- **sprite_indices not uploaded to compute**: These fields exist in NpcGpuState for the render pipeline only. The compute shader has no access to them.
- **GpuReadState no longer extracted**: Previously cloned ~1.2MB/frame to render world where nothing read it. Now main-world only.
- **ProjPositionState zero-clone**: Read via `Extract<Res<T>>` during extract_proj_data, no cloning.
