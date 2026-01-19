shader_type canvas_item;

varying float health;
varying float flash;

void vertex() {
    health = INSTANCE_CUSTOM.r;
    flash = INSTANCE_CUSTOM.g;
}

void fragment() {
    vec4 base_color = COLOR;

    // Health bar in top 15% of sprite
    if (UV.y < 0.15) {
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
        COLOR = mix(base_color, vec4(1.0, 1.0, 1.0, 1.0), flash);
    }
}