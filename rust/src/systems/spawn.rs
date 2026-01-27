//! Spawn systems - Create Bevy entities from spawn messages

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

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
    mut npc_map: ResMut<NpcEntityMap>,
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
        npc_map.0.clear();  // Clear entity map on reset
    }
}

/// Process spawn messages: create Bevy entities and initialize GPU data.
pub fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
    mut npc_map: ResMut<NpcEntityMap>,
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
        let entity = commands.spawn((
            NpcIndex(idx),
            job,
            Speed::default(),
            Health::default(),
        )).id();

        // O(1) lookup: store entity in map
        npc_map.0.insert(idx, entity);
        count.0 += 1;
    }
}

/// Process guard spawn messages: create guard entities with full component set.
pub fn spawn_guard_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnGuardMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
    mut npc_map: ResMut<NpcEntityMap>,
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

        // Set faction in GPU data
        gpu_data.factions[idx] = Faction::Villager.to_i32();
        gpu_data.healths[idx] = 100.0;

        // Create guard entity with full component set
        let entity = commands.spawn((
            NpcIndex(idx),
            Job::Guard,
            Speed::default(),
            Energy::default(),
            Health::default(),
            Faction::Villager,
            AttackStats::default(),
            AttackTimer(0.0),
            Guard { town_idx: event.town_idx },
            Home(Vector2::new(event.home_x, event.home_y)),
            PatrolRoute {
                posts: patrol_posts,
                current: event.starting_post as usize,
            },
            OnDuty { ticks_waiting: 0 },
        )).id();

        // O(1) lookup: store entity in map
        npc_map.0.insert(idx, entity);
        count.0 += 1;
    }
}

/// Process farmer spawn messages: create farmer entities with WorkPosition + Home.
pub fn spawn_farmer_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnFarmerMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
    mut npc_map: ResMut<NpcEntityMap>,
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

        // Set faction in GPU data
        gpu_data.factions[idx] = Faction::Villager.to_i32();
        gpu_data.healths[idx] = 100.0;

        // Create farmer entity with behavior components
        let entity = commands.spawn((
            NpcIndex(idx),
            Job::Farmer,
            Speed::default(),
            Energy::default(),
            Health::default(),
            Faction::Villager,
            Farmer { town_idx: event.town_idx },
            Home(Vector2::new(event.home_x, event.home_y)),
            WorkPosition(Vector2::new(event.work_x, event.work_y)),
            GoingToWork,
        )).id();

        // O(1) lookup: store entity in map
        npc_map.0.insert(idx, entity);
        count.0 += 1;
    }
}

/// Process raider spawn messages: create raider entities with combat components.
pub fn spawn_raider_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnRaiderMsg>,
    mut count: ResMut<NpcCount>,
    mut gpu_data: ResMut<GpuData>,
    mut npc_map: ResMut<NpcEntityMap>,
) {
    for event in events.read() {
        let idx = gpu_data.npc_count;
        if idx >= MAX_NPC_COUNT {
            continue;
        }

        let (r, g, b, a) = Job::Raider.color();
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
        gpu_data.factions[idx] = Faction::Raider.to_i32();
        gpu_data.healths[idx] = 100.0;
        gpu_data.npc_count += 1;
        gpu_data.dirty = true;

        // Create raider entity with combat components
        let entity = commands.spawn((
            NpcIndex(idx),
            Job::Raider,
            Speed::default(),
            Health::default(),
            Faction::Raider,
            AttackStats::default(),
            AttackTimer(0.0),
            Home(Vector2::new(event.camp_x, event.camp_y)),
        )).id();

        // O(1) lookup: store entity in map
        npc_map.0.insert(idx, entity);
        count.0 += 1;
    }
}
