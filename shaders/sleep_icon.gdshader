shader_type canvas_item;

// Simple "z" indicator for sleeping NPCs
void fragment() {
    vec2 uv = UV;
    float alpha = 0.0;

    // Draw a stylized "z" shape
    // Top horizontal line
    if (uv.y > 0.7 && uv.y < 0.85 && uv.x > 0.2 && uv.x < 0.8) {
        alpha = 1.0;
    }
    // Diagonal line
    float diag = 1.0 - uv.x;
    if (abs(uv.y - diag) < 0.12 && uv.x > 0.2 && uv.x < 0.8) {
        alpha = 1.0;
    }
    // Bottom horizontal line
    if (uv.y > 0.15 && uv.y < 0.3 && uv.x > 0.2 && uv.x < 0.8) {
        alpha = 1.0;
    }

    // Light blue color with glow effect
    vec3 col = vec3(0.6, 0.8, 1.0);
    COLOR = vec4(col, alpha * 0.9);
}
