shader_type canvas_item;

uniform sampler2D sprite_sheet;
uniform vec2 sheet_size = vec2(918.0, 203.0);  // Texture dimensions
uniform vec2 sprite_size = vec2(16.0, 16.0);
uniform float margin = 1.0;
uniform int hp_bar_mode = 1;  // 0 = off, 1 = when damaged, 2 = always

varying float health;
varying float heal_flash;  // Encoded: 0-1 = flash, 2-3 = healing+flash, 10+ = carrying (+10 offset)
varying float carrying;    // 1.0 if carrying food
varying vec2 sprite_offset;

void vertex() {
    health = INSTANCE_CUSTOM.r;
    heal_flash = INSTANCE_CUSTOM.g;
    float frame_x = INSTANCE_CUSTOM.b * 255.0;
    float frame_y = INSTANCE_CUSTOM.a * 255.0;

    // Calculate pixel offset for this sprite
    float cell_size = sprite_size.x + margin;  // 17px per cell
    sprite_offset = vec2(frame_x * cell_size, frame_y * cell_size) / sheet_size;
}

void fragment() {
    // Decode healing and flash from combined value
    // 0-1 = flash only, 2-3 = healing + flash (subtract 2 for flash intensity)
    bool healing = heal_flash >= 2.0;
    float flash = healing ? (heal_flash - 2.0) : heal_flash;

    // Calculate UV within sprite sheet (flip Y)
    vec2 sprite_uv_size = sprite_size / sheet_size;
    vec2 flipped_uv = vec2(UV.x, 1.0 - UV.y);
    vec2 tex_uv = sprite_offset + flipped_uv * sprite_uv_size;
    vec4 tex_color = texture(sprite_sheet, tex_uv);

    // Health bar in top 15% of sprite
    // hp_bar_mode: 0 = off, 1 = when damaged, 2 = always
    bool show_hp_bar = (hp_bar_mode == 2) || (hp_bar_mode == 1 && health < 0.99);
    if (UV.y < 0.15 && show_hp_bar) {
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
        // Apply tint from instance color
        vec4 tinted = tex_color * COLOR;

        // Healing effect: green tint + pulsing glow
        if (healing) {
            // Pulsing effect using TIME
            float pulse = 0.5 + 0.5 * sin(TIME * 3.0);

            if (tex_color.a > 0.1) {
                // On sprite: green tint that pulses
                vec3 heal_color = vec3(0.4, 1.0, 0.5);
                tinted.rgb = mix(tinted.rgb, heal_color, 0.2 + 0.1 * pulse);
            } else {
                // In transparent areas: soft glow
                float dist_from_center = length(UV - vec2(0.5, 0.5)) * 2.0;
                float halo = smoothstep(1.0, 0.3, dist_from_center) * (0.4 + 0.2 * pulse);
                tinted = vec4(0.4, 1.0, 0.5, halo);
            }
        }

        // Flash effect (white overlay, on top of everything)
        vec4 final_color = mix(tinted, vec4(1.0, 1.0, 1.0, tinted.a), flash);
        COLOR = final_color;
    }
}
