shader_type canvas_item;

uniform sampler2D sprite_sheet;
uniform vec2 sheet_size = vec2(968.0, 526.0);
uniform vec2 sprite_size = vec2(16.0, 16.0);
uniform float margin = 1.0;
uniform vec2 sprite_frame = vec2(31.0, 9.0);

void fragment() {
    float cell_size = sprite_size.x + margin;
    vec2 sprite_offset = vec2(sprite_frame.x * cell_size, sprite_frame.y * cell_size) / sheet_size;
    vec2 sprite_uv_size = sprite_size / sheet_size;
    vec2 flipped_uv = vec2(UV.x, 1.0 - UV.y);
    vec2 tex_uv = sprite_offset + flipped_uv * sprite_uv_size;
    COLOR = texture(sprite_sheet, tex_uv);
}
