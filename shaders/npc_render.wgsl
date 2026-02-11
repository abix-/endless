// Universal Instanced Render Shader
// Renders terrain, buildings, NPCs, equipment, and projectiles via vertex instancing.
// Slot 0 = quad vertices, slot 1 = per-instance data (position, sprite, color, health, flash, scale, atlas_id)

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
    @location(8) atlas_id: f32,          // 0=character atlas, 1=world atlas, 2=heal halo
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

// Camera uniform (bind group 1)
struct Camera {
    pos: vec2<f32>,
    zoom: f32,
    _pad: f32,
    viewport: vec2<f32>,
};
@group(1) @binding(0) var<uniform> camera: Camera;

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

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Expand quad by per-instance scale and offset by instance position
    let world_pos = in.instance_pos + in.quad_pos * in.scale;

    // Orthographic projection with camera transform
    let offset = (world_pos - camera.pos) * camera.zoom;
    let ndc = offset / (camera.viewport * 0.5);
    out.clip_position = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);

    // Calculate UV based on which atlas to use
    if in.atlas_id >= 1.5 {
        // Heal halo: single-sprite texture, UV = quad_uv directly
        out.uv = in.quad_uv;
    } else if in.atlas_id < 0.5 {
        // Character atlas
        let pixel_x = in.sprite_cell.x * CHAR_CELL + in.quad_uv.x * CHAR_SPRITE;
        let pixel_y = in.sprite_cell.y * CHAR_CELL + in.quad_uv.y * CHAR_SPRITE;
        out.uv = vec2<f32>(pixel_x / CHAR_TEX_W, pixel_y / CHAR_TEX_H);
    } else {
        // World atlas
        let pixel_x = in.sprite_cell.x * WORLD_CELL + in.quad_uv.x * WORLD_SPRITE;
        let pixel_y = in.sprite_cell.y * WORLD_CELL + in.quad_uv.y * WORLD_SPRITE;
        out.uv = vec2<f32>(pixel_x / WORLD_TEX_W, pixel_y / WORLD_TEX_H);
    }

    out.color = in.color;
    out.health = in.health;
    out.quad_uv = in.quad_uv;
    out.flash = in.flash;
    out.atlas_id = in.atlas_id;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Heal halo sprite (atlas_id >= 2): sample from heal_texture
    if in.atlas_id >= 1.5 {
        let tex_color = textureSample(heal_texture, heal_sampler, in.uv);
        if tex_color.a < 0.1 { discard; }
        return vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
    }

    // Health bar in bottom 15% of sprite (quad_uv.y > 0.85 = bottom rows)
    // Only show when damaged (health < 99%) and on character atlas sprites
    let show_hp_bar = in.health < 0.99 && in.atlas_id < 0.5;
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

    var final_color = vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);

    // Damage flash: white overlay that fades out (character sprites only)
    if in.flash > 0.0 && in.atlas_id < 0.5 {
        final_color = vec4<f32>(mix(final_color.rgb, vec3<f32>(1.0, 1.0, 1.0), in.flash), final_color.a);
    }
    return final_color;
}
