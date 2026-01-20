shader_type canvas_item;

uniform sampler2D spritesheet : source_color, filter_nearest;

varying vec2 sprite_uv_pos;
varying vec2 sprite_uv_size;
varying float tile_count;

void vertex() {
	// custom_data: r=u, g=v, b=width, a=tile_count (1=stretch, 2=tile 2x2)
	sprite_uv_pos = vec2(INSTANCE_CUSTOM.r, INSTANCE_CUSTOM.g);
	sprite_uv_size = vec2(INSTANCE_CUSTOM.b, INSTANCE_CUSTOM.b);  // Square sprites
	tile_count = INSTANCE_CUSTOM.a;
}

void fragment() {
	// Flip Y for QuadMesh
	vec2 flipped_uv = vec2(UV.x, 1.0 - UV.y);

	// Tile if tile_count > 1
	vec2 tiled_uv = fract(flipped_uv * tile_count);

	vec2 tex_uv = sprite_uv_pos + tiled_uv * sprite_uv_size;
	COLOR = texture(spritesheet, tex_uv);
}
