shader_type canvas_item;

uniform sampler2D sprite_sheet;
uniform vec2 sheet_size = vec2(918.0, 203.0);  // Texture dimensions
uniform vec2 sprite_size = vec2(16.0, 16.0);
uniform float margin = 1.0;

varying float health;
varying float flash;
varying vec2 sprite_offset;

void vertex() {
    health = INSTANCE_CUSTOM.r;
    flash = INSTANCE_CUSTOM.g;
    float frame_x = INSTANCE_CUSTOM.b * 255.0;
    float frame_y = INSTANCE_CUSTOM.a * 255.0;

    // Calculate pixel offset for this sprite
    float cell_size = sprite_size.x + margin;  // 17px per cell
    sprite_offset = vec2(frame_x * cell_size, frame_y * cell_size) / sheet_size;
}

void fragment() {
    // Calculate UV within sprite sheet (flip Y)
    vec2 sprite_uv_size = sprite_size / sheet_size;
    vec2 flipped_uv = vec2(UV.x, 1.0 - UV.y);
    vec2 tex_uv = sprite_offset + flipped_uv * sprite_uv_size;
    vec4 tex_color = texture(sprite_sheet, tex_uv);

    // Health bar in top 15% of sprite (only show if damaged)
    if (UV.y < 0.15 && health < 0.99) {
        vec4 bar_color = vec4(0.2, 0.2, 0.2, 1.0);

        if (UV.x < health) {
            if (health > 0.5) {
                bar_color = vec4(0.0, 0.8, 0.0, 1.0);
            } else if (health > 0.25) {
                bar_color = vec4(1.0, 0.8, 0.0, 1.0);
            } else {
                bar_color = vec4(1.0, 0.0, 0.0, 1.0);
            }
        }

        COLOR = bar_color;
    } else {
        // Apply tint from instance color, then flash effect
        vec4 tinted = tex_color * COLOR;
        vec4 final_color = mix(tinted, vec4(1.0, 1.0, 1.0, tinted.a), flash);
        COLOR = final_color;
    }
}
