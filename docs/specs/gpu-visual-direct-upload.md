# Spec: Direct GPU Upload — Eliminate ALL ExtractResource Clones for NPC Data

## Problem

`NpcBufferWrites` (gpu.rs:119) is `#[derive(Clone, ExtractResource)]`. Bevy clones the entire struct from main→render world every frame. At 50K `MAX_NPCS` (pre-allocated regardless of alive count), this clones **~6.4MB/frame** via 15 heap alloc+memcpy operations. `ExtractResourcePlugin::<NpcBufferWrites>::default()` is registered at gpu.rs:679.

Visual data (9 of 15 Vecs, ~5MB) is only consumed by `prepare_npc_buffers` (npc_render.rs:459) which repacks it into two GPU storage buffers (`NpcVisualBuffers.visual` and `.equip`). The repack is a redundant O(N) loop — data is unpacked from per-field arrays into packed arrays, then uploaded via `write_buffer`. This clone+repack can be eliminated by packing data in the main world and writing directly to GPU buffers during Extract.

## Architecture Decision

**Why direct GPU write in Extract (not Mutex, not double-buffer)?**

- `queue.write_buffer()` is wgpu's native CPU→GPU channel. Copies to staging memory immediately; GPU sees data at next `submit()`.
- `RenderQueue` is `pub struct RenderQueue(pub Arc<WgpuWrapper<Queue>>)` — `Send+Sync`, safely usable across threads.
- Extract systems have access to both `Extract<Res<T>>` (immutable main world read, zero clone) AND render world resources (`RenderQueue`, `NpcVisualBuffers`).
- No Mutex needed. No allocation. No frame delay.
- Pattern proven in [Bevy issue #12856](https://github.com/bevyengine/bevy/issues/12856).

## Current Data Flow (6.4MB clone/frame)

```
Update (after Step::Behavior):
  sync_visual_sprites() [gpu.rs:277]
    → ResMut<NpcBufferWrites>
    → iterates ALL alive NPCs via ECS query
    → writes: colors[c*4..], weapon_sprites[j*3..], helmet_sprites[j*3..],
              armor_sprites[j*3..], item_sprites[j*3..], status_sprites[j*3..],
              healing_sprites[j*3..] — sets buffer.dirty = true
    → scheduled: lib.rs:319

  collect_gpu_updates() [systems/drain.rs:20]
    → drains GpuUpdateMsg events → pushes to GPU_UPDATE_QUEUE static

PostUpdate:
  populate_buffer_writes() [gpu.rs:357]
    → ResMut<NpcBufferWrites>
    → drains GPU_UPDATE_QUEUE → calls buffer.apply() per message
    → SetPosition/SetTarget/SetSpeed/SetFaction/SetHealth/HideNpc/ApplyDamage → compute fields
    → SetSpriteFrame → sprite_indices[idx*4..]
    → SetDamageFlash → flash_values[idx]
    → flash decay loop: flash_values[0..active].iter_mut(), rate 5.0/s
    → scheduled: gpu.rs:663

Extract:
  ExtractResourcePlugin::<NpcBufferWrites> clones entire struct (~6.4MB)
  (only when is_changed() — but both sync_visual_sprites and populate_buffer_writes
   take ResMut, so it's ALWAYS marked changed)

Render (PrepareResources):
  write_npc_buffers() [gpu.rs:1080]
    → Res<NpcBufferWrites> (extracted)
    → per-dirty-index write_buffer for: positions, targets, speeds, factions, healths, arrivals

  prepare_npc_buffers() [npc_render.rs:459]
    → Res<NpcBufferWrites> (extracted)
    → O(N) repack loop (lines 540-560):
        visual_data[i*8..] = [sprite_col, sprite_row, atlas, flash, r, g, b, a]
        equip_data[i*24..] = [col, row, atlas, pad] × 6 layers (via EQUIP_LAYER_FIELDS)
    → write_buffer(visual_buffers.visual, 0, visual_data)
    → write_buffer(visual_buffers.equip, 0, equip_data)
    → also creates NpcVisualBuffers on first frame, recreates bind group each frame
    → also handles NpcMiscBuffers (farms, building HP bars)
```

## New Data Flow (zero clone — all direct GPU writes in Extract)

```
PostUpdate (chained):
  populate_compute_writes()
    → ResMut<NpcComputeWrites> — compute messages only (positions, targets, etc.)
    → ResMut<NpcSpriteState> — visual messages (SetSpriteFrame, SetDamageFlash) + flash decay

  build_npc_visual_upload()
    → ECS query (same as current sync_visual_sprites) + Res<NpcSpriteState>
    → packs visual_data + equip_data in GPU-ready format
    → writes to ResMut<NpcVisualUpload>

Extract:
  extract_npc_compute()
    → Extract<Res<NpcComputeWrites>> (immutable, zero clone)
    → per-dirty-index write_buffer directly to NpcGpuBuffers (positions, targets, etc.)

  extract_npc_visual()
    → Extract<Res<NpcVisualUpload>> (immutable, zero clone)
    → write_buffer visual + equip data directly to NpcVisualBuffers

Render:
  write_npc_buffers() → DELETED (moved to extract_npc_compute)
  prepare_npc_buffers() → visual repack deleted, keeps: buffer creation, bind group, misc buffers
```

Note: NpcComputeWrites stays as a main world Resource (game_hud and movement.rs
read .targets from it). It just no longer needs Clone or ExtractResource — never cloned.

## New Resources

### `NpcSpriteState` (gpu.rs, main world only, NOT extracted)

Holds persistent per-NPC visual state that's updated by message queue (not ECS-derived).

```rust
#[derive(Resource)]
pub struct NpcSpriteState {
    /// Sprite atlas coordinates: [col, row, atlas, 0] per NPC, stride 4
    pub sprite_indices: Vec<f32>,  // pre-alloc MAX_NPCS * 4 = 800KB
    /// Damage flash intensity: 0.0-1.0 per NPC, decays at 5.0/s
    pub flash_values: Vec<f32>,    // pre-alloc MAX_NPCS = 200KB
}
```

Default: same values as current `NpcBufferWrites` defaults for these fields.
Written by: `populate_compute_writes` (SetSpriteFrame, SetDamageFlash, flash decay).
Read by: `build_npc_visual_upload`.

### `NpcVisualUpload` (gpu.rs, main world only, NOT extracted)

GPU-ready packed arrays built each frame, uploaded to GPU during Extract.

```rust
#[derive(Resource, Default)]
pub struct NpcVisualUpload {
    /// [sprite_col, sprite_row, atlas, flash, r, g, b, a] per NPC — matches NpcVisual in npc_render.wgsl
    pub visual_data: Vec<f32>,
    /// [col, row, atlas, pad] × 6 layers per NPC — matches EquipSlot in npc_render.wgsl
    pub equip_data: Vec<f32>,
    /// Number of NPCs packed
    pub npc_count: usize,
}
```

Vec allocations are reused frame-to-frame (resize only when npc_count grows).
Built by: `build_npc_visual_upload`.
Read by: `extract_npc_visual` via `Extract<Res<NpcVisualUpload>>` (immutable, zero clone).

### `NpcComputeWrites` (gpu.rs, renamed from NpcBufferWrites)

Slimmed to compute-only fields. No longer Clone or ExtractResource — read via `Extract<Res<T>>` in Extract phase (zero clone).

```rust
#[derive(Resource)]
pub struct NpcComputeWrites {
    pub positions: Vec<f32>,      // [x, y] per NPC, stride 2
    pub targets: Vec<f32>,        // [x, y] per NPC, stride 2
    pub speeds: Vec<f32>,         // 1 per NPC
    pub factions: Vec<i32>,       // 1 per NPC
    pub healths: Vec<f32>,        // 1 per NPC
    pub arrivals: Vec<i32>,       // 1 per NPC
    pub dirty: bool,
    pub position_dirty_indices: Vec<usize>,
    pub target_dirty_indices: Vec<usize>,
    pub speed_dirty_indices: Vec<usize>,
    pub faction_dirty_indices: Vec<usize>,
    pub health_dirty_indices: Vec<usize>,
    pub arrival_dirty_indices: Vec<usize>,
}
```

Same Default impl as current NpcBufferWrites but without the 9 visual fields.
~1.6MB at 50K MAX_NPCS (down from 6.4MB).

## Step-by-Step Implementation

### Step 1: Define new resources (gpu.rs)

Add `NpcSpriteState` and `NpcVisualUpload` structs as shown above. Add `Default` impls:
- `NpcSpriteState::default()`: pre-alloc `sprite_indices = vec![0.0; MAX_NPCS * 4]`, `flash_values = vec![0.0; MAX_NPCS]`
- `NpcVisualUpload::default()`: empty vecs, npc_count=0

### Step 2: Rename `NpcBufferWrites` → `NpcComputeWrites` (gpu.rs)

1. Rename the struct (gpu.rs:119-157)
2. Remove these fields: `sprite_indices`, `colors`, `flash_values`, `armor_sprites`, `helmet_sprites`, `weapon_sprites`, `item_sprites`, `status_sprites`, `healing_sprites`
3. Update `Default` impl (gpu.rs:159-187) — remove the 9 visual field initializations
4. Update `apply()` method (gpu.rs:190-271):
   - Remove the `SetSpriteFrame` arm (lines 255-262) — this will move to NpcSpriteState
   - Remove the `SetDamageFlash` arm (lines 264-268) — this will move to NpcSpriteState
   - Keep all compute arms (SetPosition, SetTarget, SetSpeed, SetFaction, SetHealth, HideNpc, ApplyDamage)

### Step 3: Split `populate_buffer_writes` → `populate_compute_writes` (gpu.rs:357-387)

Rename function. Change signature:
```rust
pub fn populate_compute_writes(
    mut compute: ResMut<NpcComputeWrites>,
    mut sprites: ResMut<NpcSpriteState>,
    time: Res<Time>,
    slots: Res<SlotAllocator>,
) {
```

Body changes:
1. Reset compute dirty flags on `compute` (same as current lines 358-365)
2. Drain GPU_UPDATE_QUEUE. For each message:
   - Compute messages → `compute.apply(&update)` (same as before)
   - `SetSpriteFrame { idx, col, row, atlas }` → write to `sprites.sprite_indices[idx*4..]`
   - `SetDamageFlash { idx, intensity }` → write to `sprites.flash_values[idx]`
   - Set `compute.dirty = true` for compute messages only
3. Flash decay loop (lines 373-386): operate on `sprites.flash_values` instead of `buffer_writes.flash_values`. This no longer sets `compute.dirty` — flash is visual-only.

Note: `populate_compute_writes` still takes `ResMut<NpcComputeWrites>` every frame (dirty reset + clear). This marks the resource as "changed" every frame via Bevy's change detection. That's fine — there's no `ExtractResourcePlugin` to skip; `Extract<Res<T>>` is a zero-clone immutable read regardless.

### Step 4: Create `build_npc_visual_upload` (gpu.rs, replaces sync_visual_sprites)

Delete `sync_visual_sprites` (gpu.rs:277-353). Create new function:

```rust
pub fn build_npc_visual_upload(
    sprites: Res<NpcSpriteState>,
    gpu_data: Res<NpcGpuData>,
    mut upload: ResMut<NpcVisualUpload>,
    all_npcs: Query<(
        &NpcIndex, &Faction, &Job, &Activity,
        Option<&Healing>,
        Option<&EquippedWeapon>, Option<&EquippedHelmet>, Option<&EquippedArmor>,
    ), Without<Dead>>,
    timings: Res<SystemTimings>,
) {
    let _t = timings.scope("build_npc_visual_upload");
    let npc_count = gpu_data.npc_count as usize;
    upload.npc_count = npc_count;

    // Resize Vecs (reuses allocation if already large enough)
    upload.visual_data.resize(npc_count * 8, 0.0);
    upload.equip_data.resize(npc_count * 24, -1.0);

    // Single pass: pack visual + equip data per alive NPC
    for (npc_idx, faction, job, activity, healing, weapon, helmet, armor) in all_npcs.iter() {
        let idx = npc_idx.0;
        if idx >= npc_count { continue; }

        // --- Visual data: [sprite_col, sprite_row, atlas, flash, r, g, b, a] ---
        let base = idx * 8;
        // Sprite frame from NpcSpriteState
        upload.visual_data[base]     = sprites.sprite_indices.get(idx * 4).copied().unwrap_or(0.0);
        upload.visual_data[base + 1] = sprites.sprite_indices.get(idx * 4 + 1).copied().unwrap_or(0.0);
        upload.visual_data[base + 2] = sprites.sprite_indices.get(idx * 4 + 2).copied().unwrap_or(0.0);
        // Flash from NpcSpriteState
        upload.visual_data[base + 3] = sprites.flash_values.get(idx).copied().unwrap_or(0.0);
        // Color from ECS (same logic as current sync_visual_sprites lines 294-305)
        let (r, g, b, a) = if faction.0 == 0 {
            job.color()
        } else {
            crate::constants::raider_faction_color(faction.0)
        };
        upload.visual_data[base + 4] = r;
        upload.visual_data[base + 5] = g;
        upload.visual_data[base + 6] = b;
        upload.visual_data[base + 7] = a;

        // --- Equip data: 6 layers × [col, row, atlas, pad] ---
        let eq_base = idx * 24;

        // Layer 0: Armor
        let (ac, ar) = armor.map(|a| (a.0, a.1)).unwrap_or((-1.0, 0.0));
        upload.equip_data[eq_base]     = ac;
        upload.equip_data[eq_base + 1] = ar;
        upload.equip_data[eq_base + 2] = 0.0;
        upload.equip_data[eq_base + 3] = 0.0;

        // Layer 1: Helmet
        let (hc, hr) = helmet.map(|h| (h.0, h.1)).unwrap_or((-1.0, 0.0));
        upload.equip_data[eq_base + 4] = hc;
        upload.equip_data[eq_base + 5] = hr;
        upload.equip_data[eq_base + 6] = 0.0;
        upload.equip_data[eq_base + 7] = 0.0;

        // Layer 2: Weapon
        let (wc, wr) = weapon.map(|w| (w.0, w.1)).unwrap_or((-1.0, 0.0));
        upload.equip_data[eq_base + 8] = wc;
        upload.equip_data[eq_base + 9] = wr;
        upload.equip_data[eq_base + 10] = 0.0;
        upload.equip_data[eq_base + 11] = 0.0;

        // Layer 3: Item (food on returning raiders)
        let (ic, ir, ia) = if matches!(activity, Activity::Returning { has_food: true, .. }) {
            (crate::constants::FOOD_SPRITE.0, crate::constants::FOOD_SPRITE.1, 1.0)
        } else {
            (-1.0, 0.0, 0.0)
        };
        upload.equip_data[eq_base + 12] = ic;
        upload.equip_data[eq_base + 13] = ir;
        upload.equip_data[eq_base + 14] = ia;
        upload.equip_data[eq_base + 15] = 0.0;

        // Layer 4: Status (sleep icon)
        let (sc, sr, sa) = if matches!(activity, Activity::Resting) {
            (0.0, 0.0, 3.0)
        } else {
            (-1.0, 0.0, 0.0)
        };
        upload.equip_data[eq_base + 16] = sc;
        upload.equip_data[eq_base + 17] = sr;
        upload.equip_data[eq_base + 18] = sa;
        upload.equip_data[eq_base + 19] = 0.0;

        // Layer 5: Healing (heal halo)
        let (hlc, hla) = if healing.is_some() { (0.0, 2.0) } else { (-1.0, 0.0) };
        upload.equip_data[eq_base + 20] = hlc;
        upload.equip_data[eq_base + 21] = 0.0;
        upload.equip_data[eq_base + 22] = hla;
        upload.equip_data[eq_base + 23] = 0.0;
    }
}
```

### Step 5: Create `extract_npc_visual` (npc_render.rs, ExtractSchedule)

Add to npc_render.rs, register in the existing ExtractSchedule system tuple (line 371):

```rust
fn extract_npc_visual(
    upload: Extract<Res<NpcVisualUpload>>,
    visual_buffers: Option<Res<NpcVisualBuffers>>,
    render_queue: Res<RenderQueue>,
) {
    let Some(visual_buffers) = visual_buffers else { return };
    if upload.npc_count == 0 { return; }
    render_queue.write_buffer(
        &visual_buffers.visual, 0,
        bytemuck::cast_slice(&upload.visual_data),
    );
    render_queue.write_buffer(
        &visual_buffers.equip, 0,
        bytemuck::cast_slice(&upload.equip_data),
    );
}
```

Add `use crate::gpu::NpcVisualUpload;` to npc_render.rs imports.

Register: change line 371 from:
```rust
(extract_npc_batch, extract_proj_batch, extract_camera_state),
```
to:
```rust
(extract_npc_batch, extract_proj_batch, extract_camera_state, extract_npc_visual),
```

### Step 6: Slim `prepare_npc_buffers` (npc_render.rs:459)

Remove from the function:
1. The `buffer_writes: Option<Res<NpcBufferWrites>>` parameter (line 465)
2. The `let Some(writes) = buffer_writes else { return };` early return (line 474)
3. The visual/equip repack loop (lines 537-560): `let mut visual_data = ...`, `let mut equip_data = ...`, and the `for i in 0..npc_count` loop
4. The `write_buffer` calls for visual + equip data (lines 565-566)

Keep:
- NpcVisualBuffers creation on first frame (lines 583-625)
- Bind group recreation each frame (lines 569-586)
- NpcMiscBuffers (farms, building HP bars) (lines 478-533)
- The `npc_count` read from `gpu_data` (line 475 — now just used for buffer sizing)

**First-frame sentinel init**: When creating NpcVisualBuffers (first frame), replace the data-driven `write_buffer` (lines 599-602) with sentinel writes:
```rust
// Write sentinel data so all sprites are hidden until extract_npc_visual writes real data (frame 2+)
let sentinel_visual = vec![-1.0f32; MAX_NPC_COUNT * 8];
let sentinel_equip = vec![-1.0f32; MAX_NPC_COUNT * 6 * 4];
render_queue.write_buffer(&visual_buffer, 0, bytemuck::cast_slice(&sentinel_visual));
render_queue.write_buffer(&equip_buffer, 0, bytemuck::cast_slice(&sentinel_equip));
```
This ensures undefined GPU memory is overwritten with hidden sentinels. The vertex shader checks `vis.sprite_col < 0.0` → HIDDEN and `eq.col < 0.0` → HIDDEN, so nothing renders until `extract_npc_visual` writes real data on frame 2.

Also delete the `EQUIP_LAYER_FIELDS` const array (npc_render.rs:449-456) — no longer used.

### Step 7: Create `extract_npc_compute` (npc_render.rs, ExtractSchedule)

Replaces `write_npc_buffers` (gpu.rs:1080) — same per-dirty-index logic but runs in Extract instead of Render.

```rust
fn extract_npc_compute(
    compute: Extract<Res<NpcComputeWrites>>,
    gpu_buffers: Option<Res<NpcGpuBuffers>>,
    render_queue: Res<RenderQueue>,
) {
    let Some(gpu_buffers) = gpu_buffers else { return };
    if !compute.dirty { return; }

    // Per-dirty-index write_buffer — same logic as current write_npc_buffers
    for &idx in &compute.position_dirty_indices {
        let offset = (idx * 2 * 4) as u64;
        let data = &compute.positions[idx * 2..idx * 2 + 2];
        render_queue.write_buffer(&gpu_buffers.positions, offset, bytemuck::cast_slice(data));
    }
    for &idx in &compute.target_dirty_indices {
        let offset = (idx * 2 * 4) as u64;
        let data = &compute.targets[idx * 2..idx * 2 + 2];
        render_queue.write_buffer(&gpu_buffers.targets, offset, bytemuck::cast_slice(data));
    }
    for &idx in &compute.speed_dirty_indices {
        let offset = (idx * 4) as u64;
        render_queue.write_buffer(&gpu_buffers.speeds, offset, bytemuck::bytes_of(&compute.speeds[idx]));
    }
    for &idx in &compute.faction_dirty_indices {
        let offset = (idx * 4) as u64;
        render_queue.write_buffer(&gpu_buffers.factions, offset, bytemuck::bytes_of(&compute.factions[idx]));
    }
    for &idx in &compute.health_dirty_indices {
        let offset = (idx * 4) as u64;
        render_queue.write_buffer(&gpu_buffers.healths, offset, bytemuck::bytes_of(&compute.healths[idx]));
    }
    for &idx in &compute.arrival_dirty_indices {
        let offset = (idx * 4) as u64;
        render_queue.write_buffer(&gpu_buffers.arrivals, offset, bytemuck::bytes_of(&compute.arrivals[idx]));
    }
}
```

Register in ExtractSchedule alongside `extract_npc_visual` (step 5):
```rust
(extract_npc_batch, extract_proj_batch, extract_camera_state, extract_npc_visual, extract_npc_compute),
```

Add `use crate::gpu::{NpcComputeWrites, NpcGpuBuffers};` to npc_render.rs imports (NpcGpuBuffers may need `pub` visibility on its fields).

### Step 8: Delete `write_npc_buffers` and `ExtractResourcePlugin` (gpu.rs)

1. **Delete `write_npc_buffers`** (gpu.rs:1080) entirely — logic moved to `extract_npc_compute` above.
2. **Delete `ExtractResourcePlugin::<NpcBufferWrites>::default()`** (gpu.rs:679) — no ExtractResource needed for NPC data.
3. **Remove the render-world system registration** for `write_npc_buffers` in `GpuComputePlugin::build`.
4. **Remove `#[derive(Clone, ExtractResource)]`** from `NpcComputeWrites` — replaced by `#[derive(Resource)]` only.

### Step 9: Update plugin init (gpu.rs:655-663)

Change:
```rust
.init_resource::<NpcBufferWrites>()
// ...
.add_systems(PostUpdate, populate_buffer_writes);
```
to:
```rust
.init_resource::<NpcComputeWrites>()
.init_resource::<NpcSpriteState>()
.init_resource::<NpcVisualUpload>()
// ...
.add_systems(PostUpdate, (populate_compute_writes, build_npc_visual_upload).chain());
```

### Step 10: Update scheduling (lib.rs:319)

Delete:
```rust
.add_systems(Update, gpu::sync_visual_sprites.after(Step::Behavior).run_if(game_active.clone()))
```

`build_npc_visual_upload` is already scheduled via gpu.rs plugin in PostUpdate (step 9).

### Step 11: Update references in other files

**ui/game_hud.rs** (lines 8, 203, 427, 947):
- Change `use crate::gpu::NpcBufferWrites;` → `use crate::gpu::NpcComputeWrites;`
- Change `Res<NpcBufferWrites>` → `Res<NpcComputeWrites>` (3 occurrences)
- Change `buffer_writes: &NpcBufferWrites` → `buffer_writes: &NpcComputeWrites` (1 occurrence)
- These only read `.targets` which stays in NpcComputeWrites.

**systems/movement.rs** (lines 7, 17):
- Change `use crate::gpu::NpcBufferWrites;` → `use crate::gpu::NpcComputeWrites;`
- Change `buffer_writes: Res<NpcBufferWrites>` → `buffer_writes: Res<NpcComputeWrites>`
- Only reads `.targets` which stays in NpcComputeWrites.

### Step 12: Update tests

Tests should read `NpcVisualUpload.equip_data` — the GPU-ready packed data that's the last CPU-visible
step before `write_buffer`. This validates the full packing pipeline (ECS → build_npc_visual_upload → GPU).

Equip layer index mapping (24 floats per NPC, 4 per layer):
- Layer 5 (Healing): offset `idx * 24 + 20` → [col, row, atlas, pad]
- Layer 4 (Status/Sleep): offset `idx * 24 + 16` → [col, row, atlas, pad]

**tests/heal_visual.rs**:
- Change `use crate::gpu::NpcBufferWrites;` → `use crate::gpu::NpcVisualUpload;`
- Change `buffer: Res<NpcBufferWrites>` → `upload: Res<NpcVisualUpload>` in `tick()`
- Phase 2: replace `buffer.healing_sprites[j]` reads with:
  ```rust
  let eq_base = idx * 24 + 20;  // layer 5 = healing
  let col = upload.equip_data.get(eq_base).copied().unwrap_or(-1.0);
  let atlas = upload.equip_data.get(eq_base + 2).copied().unwrap_or(0.0);
  if col >= 0.0 && atlas == 2.0 {
      test.pass_phase(elapsed, format!("Halo active (idx={}, atlas={:.0})", idx, atlas));
  }
  ```
- Phase 3: replace `buffer.healing_sprites[idx*3]` with:
  ```rust
  let col = upload.equip_data.get(idx * 24 + 20).copied().unwrap_or(-1.0);
  if col == -1.0 {
      test.pass_phase(elapsed, format!("Halo cleared (hp={:.0})", hp));
      test.complete(elapsed);
  }
  ```

**tests/sleep_visual.rs**:
- Change `use crate::gpu::NpcBufferWrites;` → `use crate::gpu::NpcVisualUpload;`
- Change `buffer: Res<NpcBufferWrites>` → `upload: Res<NpcVisualUpload>` in `tick()`
- Phase 2: replace `buffer.status_sprites[j]` reads with:
  ```rust
  let eq_base = idx.0 * 24 + 16;  // layer 4 = status
  let col = upload.equip_data.get(eq_base).copied().unwrap_or(-1.0);
  let atlas = upload.equip_data.get(eq_base + 2).copied().unwrap_or(0.0);
  if col >= 0.0 && atlas >= 2.5 {
      test.pass_phase(elapsed, format!("Sleep icon set (idx={}, atlas={:.0})", idx.0, atlas));
  }
  ```
- Phase 3: replace `buffer.status_sprites[j]` with:
  ```rust
  let col = upload.equip_data.get(idx.0 * 24 + 16).copied().unwrap_or(-1.0);
  if col == -1.0 {
      test.pass_phase(elapsed, format!("Sleep icon cleared (idx={}, energy={:.0})", idx.0, energy));
      test.complete(elapsed);
  }
  ```

**tests/mod.rs** (lines 508, 538, 553):
- Remove `.after(crate::gpu::sync_visual_sprites)` from test system scheduling
- Tests now depend on `build_npc_visual_upload` which runs in PostUpdate (after all Update systems), so no explicit ordering needed

### Step 13: Update module-level doc comment (gpu.rs:1-10)

Change data flow description to reflect new architecture.

## GPU Buffer Layout Reference

These are the target GPU storage buffers (defined in npc_render.rs:112-121, read by npc_render.wgsl vertex_npc):

```
NpcVisualBuffers.visual: Buffer
  Layout: [f32; 8] per NPC slot
  [sprite_col, sprite_row, body_atlas, flash, r, g, b, a]
  Matches struct NpcVisual in npc_render.wgsl:67-70

NpcVisualBuffers.equip: Buffer
  Layout: [f32; 4] per equipment layer per NPC slot (6 layers × npc_count)
  [col, row, atlas, _pad]
  Indexed as: npc_equip[slot * 6 + layer]
  Matches struct EquipSlot in npc_render.wgsl:72-74
  Layer order: 0=Armor, 1=Helmet, 2=Weapon, 3=Item, 4=Status, 5=Healing
```

## Verification

1. `cargo check` — compiles clean
2. `cargo build --release` — no warnings
3. Run game, spawn 20K NPCs — verify:
   - NPC body sprites render (correct sprite frame per job)
   - NPC colors correct (job colors for villagers, faction colors for raiders)
   - Equipment overlays visible (weapon/helmet/armor on equipped NPCs)
   - Damage flash works (white overlay on hit, fades over ~0.2s)
   - Sleep icon appears on resting NPCs, disappears on wake
   - Healing halo appears on NPCs in healing zone, disappears when healed
   - Carried food item visible on returning raiders
4. F5 save / F9 load — visuals restore correctly
5. Run test suite: `cargo test` — heal-visual and sleep-visual tests pass
