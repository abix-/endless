// NPC Instanced Render Shader
// Uses vertex instancing: slot 0 = quad vertices, slot 1 = per-instance data

// Vertex input from two vertex buffers
struct VertexInput {
    // Slot 0: Static quad vertex
    @location(0) quad_pos: vec2<f32>,
    @location(1) quad_uv: vec2<f32>,
    // Slot 1: Per-instance data (step_mode = Instance)
    @location(2) instance_pos: vec2<f32>,
    @location(3) sprite_cell: vec2<f32>,  // col, row
    @location(4) color: vec4<f32>,
    @location(5) health: f32,            // 0.0-1.0 normalized
};

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) health: f32,
    @location(3) quad_uv: vec2<f32>,     // raw 0-1 UV within sprite quad
};

// Texture (bind group 0)
@group(0) @binding(0) var sprite_texture: texture_2d<f32>;
@group(0) @binding(1) var sprite_sampler: sampler;

// Camera uniform (bind group 1)
struct Camera {
    pos: vec2<f32>,
    zoom: f32,
    _pad: f32,
    viewport: vec2<f32>,
};
@group(1) @binding(0) var<uniform> camera: Camera;

// Constants
const SPRITE_SIZE: f32 = 16.0;  // Size of sprite in world units (matches 16px atlas cells)

// Sprite atlas layout (roguelikeChar_transparent.png: 918x203 pixels)
// 16x16 sprites with 1px margin = 17px cells
const CELL_SIZE: f32 = 17.0;
const SPRITE_TEX_SIZE: f32 = 16.0;
const TEXTURE_WIDTH: f32 = 918.0;
const TEXTURE_HEIGHT: f32 = 203.0;

@vertex
fn vertex(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;

    // Expand quad by sprite size and offset by instance position
    let world_pos = in.instance_pos + in.quad_pos * SPRITE_SIZE;

    // Orthographic projection with camera transform
    let offset = (world_pos - camera.pos) * camera.zoom;
    let ndc = offset / (camera.viewport * 0.5);
    out.clip_position = vec4<f32>(ndc.x, ndc.y, 0.0, 1.0);

    // Calculate UV for sprite atlas
    let cell_uv = in.quad_uv;
    let pixel_x = in.sprite_cell.x * CELL_SIZE + cell_uv.x * SPRITE_TEX_SIZE;
    let pixel_y = in.sprite_cell.y * CELL_SIZE + cell_uv.y * SPRITE_TEX_SIZE;
    out.uv = vec2<f32>(pixel_x / TEXTURE_WIDTH, pixel_y / TEXTURE_HEIGHT);

    out.color = in.color;
    out.health = in.health;
    out.quad_uv = in.quad_uv;

    return out;
}

@fragment
fn fragment(in: VertexOutput) -> @location(0) vec4<f32> {
    // Health bar in bottom 15% of sprite (quad_uv.y > 0.85 = bottom rows)
    // Only show when damaged (health < 99%)
    let show_hp_bar = in.health < 0.99;
    if in.quad_uv.y > 0.85 && show_hp_bar {
        // Dark grey background for missing health
        var bar_color = vec4<f32>(0.2, 0.2, 0.2, 1.0);

        // Filled portion colored by health level
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

    // Normal sprite rendering
    let tex_color = textureSample(sprite_texture, sprite_sampler, in.uv);
    if tex_color.a < 0.1 {
        discard;
    }
    return vec4<f32>(tex_color.rgb * in.color.rgb, tex_color.a);
}
