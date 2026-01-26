//! Spawn systems - Create Bevy entities from spawn messages

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;
use godot_bevy::prelude::*;

use crate::components::*;
use crate::messages::*;
use crate::resources::*;
use crate::constants::*;
use crate::world::WORLD_DATA;

/// Despawn all Bevy entities when RESET_BEVY flag is set.
pub fn reset_bevy_system(
    mut commands: Commands,
    query: Query<Entity, With<NpcIndex>>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    let should_reset = RESET_BEVY.lock().map(|mut f| {
        let val = *f;
        *f = false;  // Clear flag
        val
    }).unwrap_or(false);

    if should_reset {
        // Despawn all NPC entities
        for entity in query.iter() {
            commands.entity(entity).despawn();
        }
        count.0 = 0;
        gpu_data.npc_count = 0;
        gpu_data.dirty = false;
    }
}

/// Process spawn messages: create Bevy entities and initialize GPU data.
pub fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    for event in events.read() {
        let idx = gpu_data.npc_count;
        if idx >= MAX_NPC_COUNT {
            continue;
        }

        let job = Job::from_i32(event.job);
        let (r, g, b, a) = job.color();
        let speed = Speed::default().0;

        // Initialize GPU data (CPU-side copy)
        gpu_data.positions[idx * 2] = event.x;
        gpu_data.positions[idx * 2 + 1] = event.y;
        // Target starts at spawn position (no movement until set_target called)
        gpu_data.targets[idx * 2] = event.x;
        gpu_data.targets[idx * 2 + 1] = event.y;
        gpu_data.colors[idx * 4] = r;
        gpu_data.colors[idx * 4 + 1] = g;
        gpu_data.colors[idx * 4 + 2] = b;
        gpu_data.colors[idx * 4 + 3] = a;
        gpu_data.speeds[idx] = speed;
        gpu_data.npc_count += 1;
        gpu_data.dirty = true;

        // Create Bevy entity with components
        commands.spawn((
            NpcIndex(idx),
            job,
            Speed::default(),
            Health::default(),
        ));
        count.0 += 1;
    }
}

/// Process guard spawn messages: create guard entities with full component set.
pub fn spawn_guard_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnGuardMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    for event in events.read() {
        let idx = gpu_data.npc_count;
        if idx >= MAX_NPC_COUNT {
            continue;
        }

        let (r, g, b, a) = Job::Guard.color();
        let speed = Speed::default().0;

        // Initialize GPU data
        gpu_data.positions[idx * 2] = event.x;
        gpu_data.positions[idx * 2 + 1] = event.y;
        gpu_data.targets[idx * 2] = event.x;
        gpu_data.targets[idx * 2 + 1] = event.y;
        gpu_data.colors[idx * 4] = r;
        gpu_data.colors[idx * 4 + 1] = g;
        gpu_data.colors[idx * 4 + 2] = b;
        gpu_data.colors[idx * 4 + 3] = a;
        gpu_data.speeds[idx] = speed;
        gpu_data.npc_count += 1;
        gpu_data.dirty = true;

        // Build patrol route from world data
        let patrol_posts: Vec<Vector2> = if let Ok(world) = WORLD_DATA.lock() {
            let mut posts: Vec<(u32, Vector2)> = world.guard_posts.iter()
                .filter(|p| p.town_idx == event.town_idx)
                .map(|p| (p.patrol_order, p.position))
                .collect();
            posts.sort_by_key(|(order, _)| *order);
            posts.into_iter().map(|(_, pos)| pos).collect()
        } else {
            Vec::new()
        };

        // Create guard entity with full component set
        commands.spawn((
            NpcIndex(idx),
            Job::Guard,
            Speed::default(),
            Energy::default(),
            Health::default(),
            Guard { town_idx: event.town_idx },
            Home(Vector2::new(event.home_x, event.home_y)),
            PatrolRoute {
                posts: patrol_posts,
                current: event.starting_post as usize,
            },
            OnDuty { ticks_waiting: 0 },  // Start on duty at their post
        ));
        count.0 += 1;
    }
}

/// Process farmer spawn messages: create farmer entities with WorkPosition + Home.
pub fn spawn_farmer_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnFarmerMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
) {
    for event in events.read() {
        let idx = gpu_data.npc_count;
        if idx >= MAX_NPC_COUNT {
            continue;
        }

        let (r, g, b, a) = Job::Farmer.color();
        let speed = Speed::default().0;

        // Initialize GPU data
        gpu_data.positions[idx * 2] = event.x;
        gpu_data.positions[idx * 2 + 1] = event.y;
        gpu_data.targets[idx * 2] = event.x;
        gpu_data.targets[idx * 2 + 1] = event.y;
        gpu_data.colors[idx * 4] = r;
        gpu_data.colors[idx * 4 + 1] = g;
        gpu_data.colors[idx * 4 + 2] = b;
        gpu_data.colors[idx * 4 + 3] = a;
        gpu_data.speeds[idx] = speed;
        gpu_data.npc_count += 1;
        gpu_data.dirty = true;

        // Create farmer entity with behavior components
        commands.spawn((
            NpcIndex(idx),
            Job::Farmer,
            Speed::default(),
            Energy::default(),
            Health::default(),
            Farmer { town_idx: event.town_idx },
            Home(Vector2::new(event.home_x, event.home_y)),
            WorkPosition(Vector2::new(event.work_x, event.work_y)),
            GoingToWork,  // Start walking to work
        ));
        count.0 += 1;
    }
}
