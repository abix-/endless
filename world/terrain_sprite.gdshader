shader_type canvas_item;

uniform sampler2D spritesheet : source_color, filter_nearest;

varying vec2 sprite_uv_pos;
varying vec2 sprite_uv_size;

void vertex() {
	// custom_data: r=u, g=v, b=width, a=height (UV coordinates)
	sprite_uv_pos = vec2(INSTANCE_CUSTOM.r, INSTANCE_CUSTOM.g);
	sprite_uv_size = vec2(INSTANCE_CUSTOM.b, INSTANCE_CUSTOM.a);
}

void fragment() {
	// Flip Y for QuadMesh
	vec2 flipped_uv = vec2(UV.x, 1.0 - UV.y);
	vec2 tex_uv = sprite_uv_pos + flipped_uv * sprite_uv_size;
	COLOR = texture(spritesheet, tex_uv);
}
