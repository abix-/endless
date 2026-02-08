//! Rendering - MultiMesh setup for NPCs, projectiles, items, and locations.

use godot::classes::{RenderingServer, QuadMesh, ShaderMaterial, ResourceLoader};
use godot_bevy::prelude::godot_prelude::*;

use crate::constants::*;
use crate::world;
use crate::EcsNpcManager;

/// Max location sprites (farms, beds, posts, fountains, camps).
/// Each multi-cell sprite (2x2 farm) uses multiple slots.
pub const MAX_LOCATION_SPRITES: usize = 10_000;

impl EcsNpcManager {
    pub fn setup_multimesh(&mut self, max_count: i32) {
        let mut rs = RenderingServer::singleton();

        self.multimesh_rid = rs.multimesh_create();

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(16.0, 16.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.multimesh_rid, mesh_rid);

        // Enable custom_data for sprite shader (health, flash, sprite frame)
        rs.multimesh_allocate_data_ex(
            self.multimesh_rid,
            max_count,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).custom_data_format(true).done();

        let count = max_count as usize;
        let mut init_buffer = vec![0.0f32; count * FLOATS_PER_INSTANCE];
        for i in 0..count {
            let base = i * FLOATS_PER_INSTANCE;
            init_buffer[base + 0] = 1.0;   // scale x
            init_buffer[base + 5] = 1.0;   // scale y
            init_buffer[base + 11] = 1.0;  // color alpha
            init_buffer[base + 12] = 1.0;  // custom_data.r = health (100%)
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.multimesh_rid, &packed);

        self.canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.canvas_item, parent_canvas);

        // Load and apply sprite shader material
        // IMPORTANT: Store material in struct to keep it alive (RenderingServer expects references to be kept around)
        let mut loader = ResourceLoader::singleton();
        if let Some(shader) = loader.load("res://shaders/npc_sprite.gdshader") {
            if let Some(texture) = loader.load("res://assets/roguelikeChar_transparent.png") {
                let mut material = ShaderMaterial::new_gd();
                let shader_res: Gd<godot::classes::Shader> = shader.cast();
                material.set_shader(&shader_res);
                material.set_shader_parameter("sprite_sheet", &texture.to_variant());
                material.set_shader_parameter("sheet_size", &Vector2::new(918.0, 203.0).to_variant());
                material.set_shader_parameter("sprite_size", &Vector2::new(16.0, 16.0).to_variant());
                material.set_shader_parameter("margin", &1.0f32.to_variant());
                material.set_shader_parameter("hp_bar_mode", &1i32.to_variant());
                rs.canvas_item_set_material(self.canvas_item, material.get_rid());
                self.material = Some(material);
            }
        }

        rs.canvas_item_add_multimesh(self.canvas_item, self.multimesh_rid);
        // Disable visibility culling — Godot's auto AABB is wrong for world-spanning MultiMesh
        rs.canvas_item_set_custom_rect_ex(self.canvas_item, true).rect(Rect2::new(Vector2::new(-100000.0, -100000.0), Vector2::new(200000.0, 200000.0))).done();

        self.mesh = Some(mesh);
    }

    pub fn setup_proj_multimesh(&mut self, _max_count: i32) {
        let mut rs = RenderingServer::singleton();

        self.proj_multimesh_rid = rs.multimesh_create();

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(12.0, 4.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.proj_multimesh_rid, mesh_rid);

        rs.multimesh_allocate_data_ex(
            self.proj_multimesh_rid,
            0,  // Start empty, resized dynamically per-frame
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).done();

        // Share NPC canvas item (second canvas_item_create doesn't render - Godot quirk)
        // Projectiles draw on top since this add_multimesh is called after NPC's
        self.proj_canvas_item = self.canvas_item;
        rs.canvas_item_add_multimesh(self.proj_canvas_item, self.proj_multimesh_rid);

        self.proj_mesh = Some(mesh);
    }

    pub fn setup_item_multimesh(&mut self, max_count: i32) {
        let mut rs = RenderingServer::singleton();

        self.item_multimesh_rid = rs.multimesh_create();

        // Small quad for carried item icon (8x8, rendered above NPC head)
        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(8.0, 8.0));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.item_multimesh_rid, mesh_rid);

        // Color + custom_data for progress bar (INSTANCE_CUSTOM.r = progress)
        rs.multimesh_allocate_data_ex(
            self.item_multimesh_rid,
            max_count,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).color_format(true).custom_data_format(true).done();

        // Initialize all items as hidden (position -9999)
        let count = max_count as usize;
        let mut init_buffer = vec![0.0f32; count * 16]; // Transform2D(8) + Color(4) + CustomData(4)
        for i in 0..count {
            let base = i * 16;
            init_buffer[base + 0] = 1.0;      // scale x
            init_buffer[base + 3] = -9999.0;  // pos x (hidden)
            init_buffer[base + 5] = 1.0;      // scale y
            init_buffer[base + 7] = -9999.0;  // pos y (hidden)
            init_buffer[base + 11] = 1.0;     // color alpha
            // CustomData[0-3] = 0.0 (progress = 0)
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.item_multimesh_rid, &packed);

        // Create canvas item for items (above NPCs)
        self.item_canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.item_canvas_item, parent_canvas);
        rs.canvas_item_set_z_index(self.item_canvas_item, 10);  // Above NPCs (z=0)

        // Load and apply item icon shader with sprite sheet
        let mut loader = ResourceLoader::singleton();
        if let Some(shader) = loader.load("res://shaders/item_icon.gdshader") {
            if let Some(texture) = loader.load("res://assets/roguelikeSheet_transparent.png") {
                let mut material = ShaderMaterial::new_gd();
                let shader_res: Gd<godot::classes::Shader> = shader.cast();
                material.set_shader(&shader_res);
                material.set_shader_parameter("sprite_sheet", &texture.to_variant());
                rs.canvas_item_set_material(self.item_canvas_item, material.get_rid());
                self.item_material = Some(material);
            }
        }

        rs.canvas_item_add_multimesh(self.item_canvas_item, self.item_multimesh_rid);
        // Disable visibility culling for world-spanning MultiMesh
        rs.canvas_item_set_custom_rect_ex(self.item_canvas_item, true).rect(Rect2::new(Vector2::new(-100000.0, -100000.0), Vector2::new(200000.0, 200000.0))).done();

        self.item_mesh = Some(mesh);
    }

    pub fn setup_location_multimesh(&mut self) {
        let mut rs = RenderingServer::singleton();

        self.location_multimesh_rid = rs.multimesh_create();

        let mut mesh = QuadMesh::new_gd();
        mesh.set_size(Vector2::new(world::SPRITE_SIZE, world::SPRITE_SIZE));
        let mesh_rid = mesh.get_rid();
        rs.multimesh_set_mesh(self.location_multimesh_rid, mesh_rid);

        // Allocate max capacity upfront (like NPC MultiMesh)
        rs.multimesh_allocate_data_ex(
            self.location_multimesh_rid,
            MAX_LOCATION_SPRITES as i32,
            godot::classes::rendering_server::MultimeshTransformFormat::TRANSFORM_2D,
        ).custom_data_format(true).done();

        // Initialize all as hidden
        let mut init_buffer = vec![0.0f32; MAX_LOCATION_SPRITES * 12];
        for i in 0..MAX_LOCATION_SPRITES {
            let base = i * 12;
            init_buffer[base + 0] = 1.0;       // scale x
            init_buffer[base + 3] = -99999.0;  // pos x (hidden)
            init_buffer[base + 5] = 1.0;       // scale y
            init_buffer[base + 7] = -99999.0;  // pos y (hidden)
            init_buffer[base + 11] = 1.0;      // custom_data.a
        }
        let packed = PackedFloat32Array::from(init_buffer.as_slice());
        rs.multimesh_set_buffer(self.location_multimesh_rid, &packed);

        // Create canvas item for locations (behind NPCs)
        self.location_canvas_item = rs.canvas_item_create();
        let parent_canvas = self.base().get_canvas_item();
        rs.canvas_item_set_parent(self.location_canvas_item, parent_canvas);
        rs.canvas_item_set_z_index(self.location_canvas_item, -100);  // Behind NPCs

        // Load terrain_sprite shader and roguelikeSheet texture
        let mut loader = ResourceLoader::singleton();
        if let Some(shader) = loader.load("res://world/terrain_sprite.gdshader") {
            if let Some(texture) = loader.load("res://assets/roguelikeSheet_transparent.png") {
                let mut material = ShaderMaterial::new_gd();
                let shader_res: Gd<godot::classes::Shader> = shader.cast();
                material.set_shader(&shader_res);
                material.set_shader_parameter("spritesheet", &texture.to_variant());
                rs.canvas_item_set_material(self.location_canvas_item, material.get_rid());
                self.location_material = Some(material);
            }
        }

        rs.canvas_item_add_multimesh(self.location_canvas_item, self.location_multimesh_rid);
        // Disable visibility culling for world-spanning MultiMesh
        rs.canvas_item_set_custom_rect_ex(self.location_canvas_item, true)
            .rect(Rect2::new(Vector2::new(-100000.0, -100000.0), Vector2::new(200000.0, 200000.0)))
            .done();

        self.location_mesh = Some(mesh);
    }

    /// Build location MultiMesh buffer from WorldData.
    /// Call once after all add_farm/add_bed/add_guard_post/add_town calls complete.
    pub fn build_location_buffer(&mut self) {
        // Get sprites from WorldData
        let sprites: Vec<world::SpriteInstance> = if let Some(bevy_app) = self.get_bevy_app() {
            let app_ref = bevy_app.bind();
            if let Some(app) = app_ref.get_app() {
                if let Some(world_data) = app.world().get_resource::<world::WorldData>() {
                    world_data.get_all_sprites()
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        if sprites.is_empty() {
            return;
        }

        let count = sprites.len().min(MAX_LOCATION_SPRITES);
        self.location_sprite_count = count;

        // Build buffer: Transform2D (8) + CustomData (4) = 12 floats per instance
        let mut buffer = vec![0.0f32; MAX_LOCATION_SPRITES * 12];

        for (i, sprite) in sprites.iter().take(count).enumerate() {
            let base = i * 12;

            // Transform2D
            buffer[base + 0] = sprite.scale;  // scale x
            buffer[base + 1] = 0.0;           // skew y
            buffer[base + 2] = 0.0;           // unused
            buffer[base + 3] = sprite.pos.x;  // pos x
            buffer[base + 4] = 0.0;           // skew x
            buffer[base + 5] = sprite.scale;  // scale y
            buffer[base + 6] = 0.0;           // unused
            buffer[base + 7] = sprite.pos.y;  // pos y

            // CustomData: UV for terrain_sprite shader
            let uv_x = (sprite.uv.0 as f32 * world::CELL) / world::SHEET_SIZE.0;
            let uv_y = (sprite.uv.1 as f32 * world::CELL) / world::SHEET_SIZE.1;
            let uv_width = world::SPRITE_SIZE / world::SHEET_SIZE.0;

            buffer[base + 8] = uv_x;      // r = u
            buffer[base + 9] = uv_y;      // g = v
            buffer[base + 10] = uv_width; // b = width
            buffer[base + 11] = 1.0;      // a = tile_count
        }

        // Hide remaining slots
        for i in count..MAX_LOCATION_SPRITES {
            let base = i * 12;
            buffer[base + 0] = 1.0;
            buffer[base + 3] = -99999.0;
            buffer[base + 5] = 1.0;
            buffer[base + 7] = -99999.0;
            buffer[base + 11] = 1.0;
        }

        let mut rs = RenderingServer::singleton();
        let packed = PackedFloat32Array::from(buffer.as_slice());
        rs.multimesh_set_buffer(self.location_multimesh_rid, &packed);

        godot_print!("[EcsNpcManager] Built location MultiMesh: {} sprites", count);
    }
}
