shader_type canvas_item;

// Terrain tile shader - samples spritesheet using UV from custom_data
uniform sampler2D spritesheet : source_color, filter_nearest;

void fragment() {
	// custom_data: r=u, g=v, b=width, a=height (UV coordinates)
	vec4 uv_data = INSTANCE_CUSTOM;

	// Calculate UV within the sprite
	vec2 sprite_uv = uv_data.xy + UV * uv_data.zw;

	COLOR = texture(spritesheet, sprite_uv);
}
