# Issue 163: npc_render visual/equip buffer overflow analysis

## Problem

`npc_render.rs` creates two GPU storage buffers (`npc_visual_data`, `npc_equip_data`) that are
undersized relative to the actual slot pool. Two independent bugs contribute:

**Bug A -- wrong entity cap:** Buffers are sized to `MAX_NPC_COUNT` (100K) but the unified
`GpuSlotPool` namespace covers NPCs + buildings up to `MAX_ENTITIES` (200K). When the slot
high-water mark exceeds 100K the full-upload path writes a vector larger than the buffer.

**Bug B -- wrong layer count:** The equip buffer is created with `6 * size_of::<[f32; 4]>()`
(24 floats = 96 bytes) per slot but `write_npc_visual` writes 7 layers × 4 floats = 28 floats
(112 bytes) per slot. The buffer is 85.7% of required size, overflowing above slot 85,713
even without Bug A.

Overflow error from wgpu at runtime:
```
Copy of 0..6400000 overrunning buffer of size 3200000   (visual, 200K vs 100K × 8 × 4)
Copy of 0..22400000 overrunning buffer of size 9600000  (equip, 200K × 28 × 4 vs 100K × 6 × 4 × 4)
```

Same class of bug as the readback buffer overflow fixed in PR #149.

## Option analysis

### Option 1: Resize both buffers to MAX_ENTITIES with corrected layer count

Size `npc_visual_data` to `MAX_ENTITIES * 8 * 4` and `npc_equip_data` to `MAX_ENTITIES * 7 * 16`.

**Memory impact:**
| Buffer | Before (wrong) | After (correct) | Delta |
|--------|---------------|-----------------|-------|
| npc_visual_data | 100K x 8 x 4 = 3.2 MB | 200K x 8 x 4 = 6.4 MB | +3.2 MB |
| npc_equip_data | 100K x 6 x 16 = 9.6 MB | 200K x 7 x 16 = 22.4 MB | +12.8 MB |
| **Total** | **12.8 MB** | **28.8 MB** | **+16.0 MB** |

Note: the "before" equip size is already wrong (6 layers vs 7 written) so the effective correct
100K size would be 100K x 7 x 16 = 11.2 MB. Going to MAX_ENTITIES adds +11.2 MB on top of
fixing the layer count bug.

**Pros:** Simple, consistent with authority.md rule (buffer sizing uses GpuSlotPool.count()),
same pattern as the readback fix in PR #149.

**Cons:** +16 MB GPU memory (11.2 MB is correcting the existing under-allocation, 4.8 MB is
the actual delta for the unified pool extension to MAX_ENTITIES on the visual buffer, +11.2 MB
for correcting equip layer count × MAX_ENTITIES).

### Option 2: Split NPC and building slot pools

Maintain separate free-lists for NPC slots (0..MAX_NPC_COUNT) and building slots
(MAX_NPC_COUNT..MAX_ENTITIES). NPC-only buffers stay at MAX_NPC_COUNT.

**Pros:** Visual/equip buffers stay smaller for NPC-only data.

**Cons:** Major refactor of GpuSlotPool, all buffer-indexing systems, WGSL shaders, and
EntityMap slot lookups. Conflicts with the unified pool design. High risk of regression.
Not appropriate for a bug fix.

### Option 3: Sparse/indirect upload with a smaller buffer

Instead of direct slot-indexed writes, maintain a dense NPC-only buffer and an indirection
table (slot -> NPC buffer index).

**Pros:** Buffer stays compact regardless of slot fragmentation.

**Cons:** Adds complexity to the render path (extra indirection in WGSL). Dirty-index uploads
are already sparse; the issue is with the full-upload path that writes the entire vector.
Over-engineered for this bug.

## Chosen approach: Option 1

Resize both buffers to `MAX_ENTITIES` with the corrected 7-layer equip stride.

This is the minimum correct fix. Buildings use the visual buffer (sprite rendering) and need
the equip buffer cleared to sentinels (`write_building_visual` explicitly does this), so both
buffers must cover the full unified slot namespace. The unified pool design (authority.md,
Slot Namespace section) requires GpuSlotPool.count() as the sizing authority.

## Files changed

- `rust/src/npc_render.rs` -- buffer creation sizes and sentinel vector sizes
- `rust/src/gpu.rs` -- fix comment "6 layers" -> "7 layers" in NpcVisualUpload

## Memory budget summary

Total GPU memory increase: ~16 MB on a mid-range GPU with 4+ GB VRAM. Within budget.
The 9.6 MB "equip" figure previously shown in analysis was already wrong (6 layers vs 7
written), so the real increase over the actually-correct 100K baseline is ~17.6 MB to
support the full 200K unified slot pool.
