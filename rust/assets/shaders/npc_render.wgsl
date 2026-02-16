// Universal Instanced Render Shader
// Two vertex paths:
//   vertex     — instance buffer (farms, building HP bars, projectiles)
//   vertex_npc — storage buffer (NPCs + equipment, reads compute shader output directly)

struct VertexInput {
    // Slot 0: Static quad vertex
    @location(0) quad_pos: vec2<f32>,
    @location(1) quad_uv: vec2<f32>,
    // Slot 1: Per-instance data (step_mode = Instance)
    @location(2) instance_pos: vec2<f32>,
    @location(3) sprite_cell: vec2<f32>,  // col, row in sprite atlas
    @location(4) color: vec4<f32>,
    @location(5) health: f32,            // 0.0-1.0 normalized
    @location(6) flash: f32,             // 0.0-1.0 damage flash intensity
    @location(7) scale: f32,             // world-space quad size (16=NPC, 32=terrain)
    @location(8) atlas_id: f32,          // 0=character, 1=world, 2=heal halo, 3=sleep icon, 4=arrow
    @location(9) rotation: f32,          // radians, 0=no rotation (used for projectile orientation)
};

struct NpcVertexInput {
    @location(0) quad_pos: vec2<f32>,
    @location(1) quad_uv: vec2<f32>,
    @builtin(instance_index) instance_index: u32,
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) health: f32,
    @location(3) quad_uv: vec2<f32>,     // raw 0-1 UV within sprite quad
    @location(4) flash: f32,
    @location(5) atlas_id: f32,
};

// Character atlas (bind group 0, bindings 0-1)
@group(0) @binding(0) var char_texture: texture_2d<f32>;
@group(0) @binding(1) var char_sampler: sampler;

// World atlas (bind group 0, bindings 2-3)
@group(0) @binding(2) var world_texture: texture_2d<f32>;
@group(0) @binding(3) var world_sampler: sampler;

// Heal halo sprite (bind group 0, bindings 4-5)
@group(0) @binding(4) var heal_texture: texture_2d<f32>;
@group(0) @binding(5) var heal_sampler: sampler;

// Sleep icon sprite (bind group 0, bindings 6-7)
@group(0) @binding(6) var sleep_texture: texture_2d<f32>;
@group(0) @binding(7) var sleep_sampler: sampler;

// Arrow projectile sprite (bind group 0, bindings 8-9)
@group(0) @binding(8) var arrow_texture: texture_2d<f32>;
@group(0) @binding(9) var arrow_sampler: sampler;

// Camera uniform (bind group 1)
struct Camera {
    pos: vec2<f32>,
    zoom: f32,
    npc_count: u32,
    viewport: vec2<f32>,
};
@group(1) @binding(0) var<uniform> camera: Camera;

// NPC storage buffers (bind group 2, used by vertex_npc only)
struct NpcVisual {
    sprite_col: f32, sprite_row: f32, atlas_id: f32, flash: f32,
    r: f32, g: f32, b: f32, a: f32,
};

struct EquipSlot {
    col: f32, row: f32, atlas: f32, _pad: f32,
};

@group(2) @binding(0) var<storage, read> npc_positions: array<vec2<f32>>;
@group(2) @binding(1) var<storage, read> npc_healths: array<f32>;
@group(2) @binding(2) var<storage, read> npc_visual_buf: array<NpcVisual>;
@group(2) @binding(3) var<storage, read> npc_equip: array<EquipSlot>;

// Character atlas layout (roguelikeChar_transparent.png: 918x203)
const CHAR_CELL: f32 = 17.0;
const CHAR_SPRITE: f32 = 16.0;
const CHAR_TEX_W: f32 = 918.0;
const CHAR_TEX_H: f32 = 203.0;

// World atlas layout (roguelikeSheet_transparent.png: 968x526)
const WORLD_CELL: f32 = 17.0;
const WORLD_SPRITE: f32 = 16.0;
const WORLD_TEX_W: f32 = 968.0;
const WORLD_TEX_H: f32 = 526.0;

// Degenerate triangle — moves vertex off-screen to discard
const HIDDEN: vec4<f32> = vec4<f32>(0.0, 0.0, -2.0, 1.0);

// =============================================================================
// SHARED HELPERS
// =============================================================================

fn calc_uv(sprite_col: f32, sprite_row: f32, atlas_id: f32, quad_uv: vec2<f32>) -> vec2<f32> {
    if atlas_id >= 1.5 {
        // Single-sprite textures (heal, sleep, arrow): UV = quad_uv directly
        return quad_uv;
    } else if atlas_id < 0.5 {
        // Character atlas
        let px = sprite_col * CHAR_CELL + quad_uv.x * CHAR_SPRITE;
        let py = sprite_row * CHAR_CELL + quad_uv.y * CHAR_SPRITE;
        return vec2<f32>(px / CHAR_TEX_W, py / CHAR_TEX_H);
    } else {
        // World atlas
        let px = sprite_col * WORLD_CELL + quad_uv.x * WORLD_SPRITE;
        let py = sprite_row * WORLD_CELL + quad_uv.y * WORLD_SPRITE;
        return vec2<f32>(px / WORLD_TEX_W, py / WORLD_TEX_H);
    }
}

fn world_to_clip(world_pos: vec2<f32>) -> vec4<f32> {
    let offset = (world_pos - camera.pos) * camera.zoom;
    let ndc = offset / (camera.viewport * 0.5);
    return vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);
}

// =============================================================================
// VERTEX: Instance buffer path (farms, building HP bars, projectiles)
// =============================================================================

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Apply rotation to quad vertices (identity when rotation == 0)
    let c = cos(in.rotation);
    let s = sin(in.rotation);
    let rotated = vec2<f32>(
        in.quad_pos.x * c - in.quad_pos.y * s,
        in.quad_pos.x * s + in.quad_pos.y * c,
    );
    let world_pos = in.instance_pos + rotated * in.scale;

    out.clip_position = world_to_clip(world_pos);
    out.uv = calc_uv(in.sprite_cell.x, in.sprite_cell.y, in.atlas_id, in.quad_uv);
    out.color = in.color;
    out.health = in.health;
    out.quad_uv = in.quad_uv;
    out.flash = in.flash;
    out.atlas_id = in.atlas_id;

    return out;
}

// =============================================================================
// VERTEX_NPC: Storage buffer path (NPCs + equipment layers)
// =============================================================================

@vertex
fn vertex_npc(in: NpcVertexInput) -> VertexOutput {
    var out: VertexOutput;

    let slot = in.instance_index % camera.npc_count;
    let layer = in.instance_index / camera.npc_count;
    let pos = npc_positions[slot];

    // Hidden NPC (tombstoned position)
    if pos.x < -9000.0 { out.clip_position = HIDDEN; return out; }

    let vis = npc_visual_buf[slot];
    var sprite_col: f32; var sprite_row: f32;
    var atlas_id: f32; var flash: f32;
    var color: vec4<f32>; var scale: f32 = 16.0; var health: f32;

    if layer == 0u {
        // Body layer
        if vis.sprite_col < 0.0 { out.clip_position = HIDDEN; return out; }
        sprite_col = vis.sprite_col;
        sprite_row = vis.sprite_row;
        atlas_id = vis.atlas_id;
        flash = vis.flash;
        color = vec4<f32>(vis.r, vis.g, vis.b, vis.a);
        health = clamp(npc_healths[slot] / 100.0, 0.0, 1.0);
    } else {
        // Equipment layer (1-6)
        let eq = npc_equip[slot * 6u + (layer - 1u)];
        if eq.col < 0.0 { out.clip_position = HIDDEN; return out; }
        sprite_col = eq.col;
        sprite_row = eq.row;
        atlas_id = eq.atlas;
        flash = vis.flash;
        health = 1.0; // equipment layers don't show HP bars

        // Color/scale by atlas type (matches CPU-side prepare_npc_buffers logic)
        if atlas_id >= 2.5 {
            color = vec4<f32>(1.0, 1.0, 1.0, 1.0);            // sleep icon: white
        } else if atlas_id >= 1.5 {
            scale = 20.0;
            color = vec4<f32>(1.0, 0.9, 0.2, 1.0);            // heal halo: larger, yellow
        } else if atlas_id >= 0.5 {
            color = vec4<f32>(1.0, 1.0, 1.0, 1.0);            // carried item: white
        } else {
            color = vec4<f32>(vis.r, vis.g, vis.b, 1.0);      // equipment: job color
        }
    }

    out.clip_position = world_to_clip(pos + in.quad_pos * scale);
    out.uv = calc_uv(sprite_col, sprite_row, atlas_id, in.quad_uv);
    out.color = color;
    out.health = health;
    out.quad_uv = in.quad_uv;
    out.flash = flash;
    out.atlas_id = atlas_id;

    return out;
}

// =============================================================================
// FRAGMENT (shared by both vertex paths)
// =============================================================================

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Mining progress bar (atlas_id 6): gold bar in bottom 15%, discard rest
    if in.atlas_id >= 5.5 {
        if in.quad_uv.y > 0.85 {
            var bar_color = vec4<f32>(0.2, 0.2, 0.2, 1.0);
            if in.quad_uv.x < in.health {
                bar_color = vec4<f32>(1.0, 0.85, 0.0, 1.0); // Gold
            }
            return bar_color;
        }
        discard;
    }

    // Building HP bar-only mode (atlas_id 5): health bar in bottom 15%, discard rest
    if in.atlas_id >= 4.5 {
        if in.quad_uv.y > 0.85 && in.health < 0.99 {
            var bar_color = vec4<f32>(0.2, 0.2, 0.2, 1.0);
            if in.quad_uv.x < in.health {
                if in.health > 0.5 {
                    bar_color = vec4<f32>(0.0, 0.8, 0.0, 1.0);
                } else if in.health > 0.25 {
                    bar_color = vec4<f32>(1.0, 0.8, 0.0, 1.0);
                } else {
                    bar_color = vec4<f32>(1.0, 0.0, 0.0, 1.0);
                }
            }
            return bar_color;
        }
        discard;
    }

    // Arrow projectile sprite (atlas_id 4)
    if in.atlas_id >= 3.5 {
        let tex_color = textureSample(arrow_texture, arrow_sampler, in.uv);
        if tex_color.a < 0.1 { discard; }
        return vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
    }

    // Sleep icon sprite (atlas_id 3)
    if in.atlas_id >= 2.5 {
        let tex_color = textureSample(sleep_texture, sleep_sampler, in.uv);
        if tex_color.a < 0.1 { discard; }
        return vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
    }

    // Heal halo sprite (atlas_id 2)
    if in.atlas_id >= 1.5 {
        let tex_color = textureSample(heal_texture, heal_sampler, in.uv);
        if tex_color.a < 0.1 { discard; }
        return vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
    }

    // Health bar in bottom 15% of sprite (quad_uv.y > 0.85 = bottom rows)
    // Show when damaged (health < 99%) — applies to NPCs and farm growth bars
    let show_hp_bar = in.health < 0.99;
    if in.quad_uv.y > 0.85 && show_hp_bar {
        var bar_color = vec4<f32>(0.2, 0.2, 0.2, 1.0);

        if in.quad_uv.x < in.health {
            if in.health > 0.5 {
                bar_color = vec4<f32>(0.0, 0.8, 0.0, 1.0);  // Green: healthy
            } else if in.health > 0.25 {
                bar_color = vec4<f32>(1.0, 0.8, 0.0, 1.0);  // Yellow: wounded
            } else {
                bar_color = vec4<f32>(1.0, 0.0, 0.0, 1.0);  // Red: critical
            }
        }
        return bar_color;
    }

    // Sample from the correct atlas
    var tex_color: vec4<f32>;
    if in.atlas_id < 0.5 {
        tex_color = textureSample(char_texture, char_sampler, in.uv);
    } else {
        tex_color = textureSample(world_texture, world_sampler, in.uv);
    }

    if tex_color.a < 0.1 {
        discard;
    }

    // Equipment layers (health >= 1.0): discard bottom pixels to preserve health bar visibility
    if in.health >= 0.99 && in.quad_uv.y > 0.85 && in.atlas_id < 0.5 {
        discard;
    }

    let brightness = dot(tex_color.rgb, vec3<f32>(0.299, 0.587, 0.114));
    var final_color = vec4<f32>(brightness * in.color.rgb, tex_color.a);

    // Damage flash: white overlay that fades out (character sprites only)
    if in.flash > 0.0 && in.atlas_id < 0.5 {
        final_color = vec4<f32>(mix(final_color.rgb, vec3<f32>(1.0, 1.0, 1.0), in.flash), final_color.a);
    }
    return final_color;
}
