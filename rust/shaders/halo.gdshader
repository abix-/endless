shader_type canvas_item;

// Golden healing halo effect
void fragment() {
	vec2 center = vec2(0.5, 0.5);
	float dist = distance(UV, center);

	// Soft circle with glow
	float inner = 0.3;
	float outer = 0.5;
	float alpha = smoothstep(outer, inner, dist) * 0.6;

	// Pulsing effect
	float pulse = sin(TIME * 3.0) * 0.15 + 0.85;

	// Golden color
	vec3 gold = vec3(1.0, 0.85, 0.3);

	COLOR = vec4(gold * pulse, alpha * pulse);
}
