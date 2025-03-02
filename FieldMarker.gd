# Add this code to a new script called FieldMarker.gd
extends Node3D

@export var field_size: Vector2 = Vector2(10, 10)  # Width and length of the field
@export var field_color: Color = Color(0.2, 0.8, 0.2, 0.5)  # Green with transparency

func _ready():
	create_field_marker()

func create_field_marker():
	# Create a mesh instance for our field
	var field = MeshInstance3D.new()
	
	# Create a plane mesh for the field
	var plane_mesh = PlaneMesh.new()
	plane_mesh.size = field_size
	field.mesh = plane_mesh
	
	# Create material for the field
	var material = StandardMaterial3D.new()
	material.albedo_color = field_color
	material.flags_transparent = true  # Enable transparency
	field.material_override = material
	
	# Position slightly above ground to prevent z-fighting
	field.position.y = 0.01
	
	# Add to scene
	add_child(field)
