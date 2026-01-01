shader_type canvas_item;

void fragment() {
    float health = INSTANCE_CUSTOM.r;
    vec4 base_color = COLOR;
    
    // Health bar in top 15% of sprite
    if (UV.y < 0.15) {
        // Background (dark gray)
        vec4 bar_color = vec4(0.2, 0.2, 0.2, 1.0);
        
        // Green/red based on health
        if (UV.x < health) {
            if (health > 0.5) {
                bar_color = vec4(0.0, 0.8, 0.0, 1.0);  // Green
            } else if (health > 0.25) {
                bar_color = vec4(1.0, 0.8, 0.0, 1.0);  // Yellow
            } else {
                bar_color = vec4(1.0, 0.0, 0.0, 1.0);  // Red
            }
        }
        
        COLOR = bar_color;
    } else {
        // Normal sprite with tint
        COLOR = base_color;
    }
}