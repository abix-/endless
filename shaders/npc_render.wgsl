// Universal Instanced Render Shader
// Renders terrain, buildings, NPCs, equipment, and projectiles via vertex instancing.
// Slot 0 = quad vertices, slot 1 = per-instance data (position, sprite, color, health, flash, scale, atlas_id, rotation)

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

    // Apply rotation to quad vertices (identity when rotation == 0)
    let c = cos(in.rotation);
    let s = sin(in.rotation);
    let rotated = vec2<f32>(
        in.quad_pos.x * c - in.quad_pos.y * s,
        in.quad_pos.x * s + in.quad_pos.y * c,
    );
    let world_pos = in.instance_pos + rotated * in.scale;

    // Orthographic projection with camera transform
    let offset = (world_pos - camera.pos) * camera.zoom;
    let ndc = offset / (camera.viewport * 0.5);
    out.clip_position = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);

    // Calculate UV based on which atlas to use
    if in.atlas_id >= 1.5 {
        // Single-sprite textures (heal, sleep, arrow): UV = quad_uv directly
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
