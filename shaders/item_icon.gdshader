shader_type canvas_item;

uniform sampler2D sprite_sheet;
uniform vec2 sheet_size = vec2(968.0, 526.0);  // roguelikeSheet dimensions
uniform vec2 sprite_size = vec2(16.0, 16.0);
uniform float margin = 1.0;
uniform vec2 sprite_pos = vec2(24.0, 9.0);  // Food sprite at (24, 9)

varying float progress;

void vertex() {
    progress = INSTANCE_CUSTOM.r;
}

void fragment() {
    // Progress bar in top 15% (matching NPC HP bar style)
    if (UV.y < 0.15) {
        vec4 bar_color = vec4(0.2, 0.2, 0.2, 1.0);

        if (UV.x < progress) {
            if (progress >= 0.99) {
                bar_color = vec4(1.0, 0.85, 0.2, 1.0);  // Ready = gold
            } else {
                bar_color = vec4(0.3, 0.8, 0.3, 1.0);  // Growing = green
            }
        }
        COLOR = bar_color;
    } else {
        // Sample sprite from sheet
        float cell_size = sprite_size.x + margin;  // 17px per cell
        vec2 sprite_offset = vec2(sprite_pos.x * cell_size, sprite_pos.y * cell_size) / sheet_size;
        vec2 sprite_uv_size = sprite_size / sheet_size;
        vec2 flipped_uv = vec2(UV.x, 1.0 - UV.y);
        vec2 tex_uv = sprite_offset + flipped_uv * sprite_uv_size;
        vec4 tex_color = texture(sprite_sheet, tex_uv);

        // Tint with instance color and show sprite
        COLOR = tex_color * COLOR;
    }
}
