//! Spawn systems - Create Bevy entities from spawn messages

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

use crate::components::*;
use crate::messages::*;
use crate::resources::*;
use crate::world::WORLD_DATA;

/// Despawn all Bevy entities when RESET_BEVY flag is set.
pub fn reset_bevy_system(
    mut commands: Commands,
    query: Query<Entity, With<NpcIndex>>,
    mut count: ResMut<NpcCount>,
    mut npc_map: ResMut<NpcEntityMap>,
) {
    let should_reset = RESET_BEVY.lock().map(|mut f| {
        let val = *f;
        *f = false;
        val
    }).unwrap_or(false);

    if should_reset {
        for entity in query.iter() {
            commands.entity(entity).despawn();
        }
        count.0 = 0;
        npc_map.0.clear();
    }
}

/// Generic spawn system. Job determines the component template.
/// All GPU writes go through GPU_UPDATE_QUEUE (no direct buffer_update).
pub fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    mut count: ResMut<NpcCount>,
    mut npc_map: ResMut<NpcEntityMap>,
) {
    let mut max_slot = 0usize;
    let mut had_spawns = false;

    for msg in events.read() {
        had_spawns = true;
        let idx = msg.slot_idx;
        if idx + 1 > max_slot {
            max_slot = idx + 1;
        }
        let job = Job::from_i32(msg.job);
        let (r, g, b, a) = job.color();

        // GPU writes via queue — no direct buffer_update()
        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
            queue.push(GpuUpdate::SetPosition { idx, x: msg.x, y: msg.y });
            queue.push(GpuUpdate::SetTarget { idx, x: msg.x, y: msg.y });
            queue.push(GpuUpdate::SetColor { idx, r, g, b, a });
            queue.push(GpuUpdate::SetSpeed { idx, speed: 100.0 });
            queue.push(GpuUpdate::SetFaction { idx, faction: msg.faction });
            queue.push(GpuUpdate::SetHealth { idx, health: 100.0 });
        }

        // Base entity (all NPCs get these)
        let mut ec = commands.spawn((
            NpcIndex(idx),
            job,
            Speed::default(),
            Health::default(),
            Faction::from_i32(msg.faction),
            Home(Vector2::new(msg.home_x, msg.home_y)),
        ));

        // Job template — determines component bundle
        match job {
            Job::Guard => {
                ec.insert(Energy::default());
                ec.insert((AttackStats::default(), AttackTimer(0.0)));
                ec.insert(Guard { town_idx: msg.town_idx as u32 });
                if msg.starting_post >= 0 {
                    let patrol_posts = build_patrol_route(msg.town_idx as u32);
                    ec.insert((
                        PatrolRoute {
                            posts: patrol_posts,
                            current: msg.starting_post as usize,
                        },
                        OnDuty { ticks_waiting: 0 },
                    ));
                }
            }
            Job::Farmer => {
                ec.insert(Energy::default());
                ec.insert(Farmer { town_idx: msg.town_idx as u32 });
                if msg.work_x >= 0.0 {
                    ec.insert((
                        WorkPosition(Vector2::new(msg.work_x, msg.work_y)),
                        GoingToWork,
                    ));
                }
            }
            Job::Raider => {
                ec.insert(Energy::default());
                ec.insert((AttackStats::default(), AttackTimer(0.0)));
                ec.insert(Stealer);
                ec.insert(FleeThreshold { pct: 0.50 });
                ec.insert(LeashRange { distance: 400.0 });
                ec.insert(WoundedThreshold { pct: 0.25 });
            }
        }

        npc_map.0.insert(idx, ec.id());
        count.0 += 1;
    }

    // Update GPU dispatch count so process() includes these NPCs
    if had_spawns {
        if let Ok(mut dc) = GPU_DISPATCH_COUNT.lock() {
            if max_slot > *dc {
                *dc = max_slot;
            }
        }
    }
}

/// Build sorted patrol route from WorldData for a given town.
fn build_patrol_route(town_idx: u32) -> Vec<Vector2> {
    if let Ok(world) = WORLD_DATA.lock() {
        let mut posts: Vec<(u32, Vector2)> = world.guard_posts.iter()
            .filter(|p| p.town_idx == town_idx)
            .map(|p| (p.patrol_order, p.position))
            .collect();
        posts.sort_by_key(|(order, _)| *order);
        posts.into_iter().map(|(_, pos)| pos).collect()
    } else {
        Vec::new()
    }
}
