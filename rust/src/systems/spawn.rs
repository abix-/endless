//! Spawn systems - Create Bevy entities from spawn messages

use godot_bevy::prelude::bevy_ecs_prelude::*;
use godot_bevy::prelude::godot_prelude::*;

use crate::components::*;
use crate::constants::*;
use crate::messages::*;
use crate::resources::*;
use crate::systems::economy::*;
use crate::world::WORLD_DATA;

// Name generation word lists
const ADJECTIVES: &[&str] = &["Swift", "Brave", "Calm", "Bold", "Sharp", "Quick", "Stern", "Wise", "Keen", "Strong"];
const FARMER_NOUNS: &[&str] = &["Tiller", "Sower", "Reaper", "Plower", "Grower"];
const GUARD_NOUNS: &[&str] = &["Shield", "Sword", "Watcher", "Sentinel", "Defender"];
const RAIDER_NOUNS: &[&str] = &["Blade", "Fang", "Shadow", "Claw", "Storm"];

fn generate_name(job: Job, slot: usize) -> String {
    let adj = ADJECTIVES[slot % ADJECTIVES.len()];
    let noun = match job {
        Job::Farmer => FARMER_NOUNS[(slot / ADJECTIVES.len()) % FARMER_NOUNS.len()],
        Job::Guard => GUARD_NOUNS[(slot / ADJECTIVES.len()) % GUARD_NOUNS.len()],
        Job::Raider => RAIDER_NOUNS[(slot / ADJECTIVES.len()) % RAIDER_NOUNS.len()],
        Job::Fighter => "Fighter",
    };
    format!("{} {}", adj, noun)
}

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
    mut pop_stats: ResMut<PopulationStats>,
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
        // Target defaults to spawn position; overridden below for jobs with initial destinations
        let (target_x, target_y) = if job == Job::Farmer && msg.work_x >= 0.0 {
            (msg.work_x, msg.work_y)
        } else {
            (msg.x, msg.y)
        };
        // Get sprite frame for this job
        let (sprite_col, sprite_row) = match job {
            Job::Farmer => SPRITE_FARMER,
            Job::Guard => SPRITE_GUARD,
            Job::Raider => SPRITE_RAIDER,
            Job::Fighter => SPRITE_FIGHTER,
        };

        if let Ok(mut queue) = GPU_UPDATE_QUEUE.lock() {
            queue.push(GpuUpdate::SetPosition { idx, x: msg.x, y: msg.y });
            queue.push(GpuUpdate::SetTarget { idx, x: target_x, y: target_y });
            queue.push(GpuUpdate::SetColor { idx, r, g, b, a });
            queue.push(GpuUpdate::SetSpeed { idx, speed: 100.0 });
            queue.push(GpuUpdate::SetFaction { idx, faction: msg.faction });
            queue.push(GpuUpdate::SetHealth { idx, health: 100.0 });
            queue.push(GpuUpdate::SetSpriteFrame { idx, col: sprite_col, row: sprite_row });
        }

        // Base entity (all NPCs get these)
        let mut ec = commands.spawn((
            NpcIndex(idx),
            job,
            TownId(msg.town_idx),
            Speed::default(),
            Health::default(),
            Faction::from_i32(msg.faction),
            Home(Vector2::new(msg.home_x, msg.home_y)),
        ));

        // Job template — determines component bundle
        match job {
            Job::Guard => {
                ec.insert(Energy::default());
                ec.insert((AttackStats::melee(), AttackTimer(0.0)));
                ec.insert(Guard);
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
                ec.insert(Farmer);
                if msg.work_x >= 0.0 {
                    ec.insert((
                        WorkPosition(Vector2::new(msg.work_x, msg.work_y)),
                        GoingToWork,
                    ));
                }
            }
            Job::Raider => {
                ec.insert(Energy::default());
                ec.insert((AttackStats::melee(), AttackTimer(0.0)));
                ec.insert(Stealer);
                ec.insert(FleeThreshold { pct: 0.50 });
                ec.insert(LeashRange { distance: 400.0 });
                ec.insert(WoundedThreshold { pct: 0.25 });
            }
            Job::Fighter => {
                let stats = if msg.attack_type == 1 { AttackStats::ranged() } else { AttackStats::melee() };
                ec.insert((stats, AttackTimer(0.0)));
            }
        }

        npc_map.0.insert(idx, ec.id());
        count.0 += 1;
        pop_inc_alive(&mut pop_stats, job, msg.town_idx);

        // Initialize NPC metadata for UI queries
        if let Ok(mut meta) = NPC_META.lock() {
            if idx < meta.len() {
                meta[idx] = NpcMeta {
                    name: generate_name(job, idx),
                    level: 1,
                    xp: 0,
                    trait_id: (idx % 5) as i32,  // Simple trait assignment (0-4)
                    town_id: msg.town_idx,
                    job: msg.job,
                };
            }
        }

        // Set initial state for UI
        let initial_state = match job {
            Job::Guard => if msg.starting_post >= 0 { STATE_ON_DUTY } else { STATE_IDLE },
            Job::Farmer => if msg.work_x >= 0.0 { STATE_GOING_TO_WORK } else { STATE_IDLE },
            Job::Raider => STATE_IDLE,  // Will be set by steal_decision_system
            Job::Fighter => STATE_IDLE,
        };
        if let Ok(mut states) = NPC_STATES.lock() {
            if idx < states.len() {
                states[idx] = initial_state;
            }
        }

        // Add to per-town NPC list
        if msg.town_idx >= 0 {
            if let Ok(mut by_town) = NPCS_BY_TOWN.lock() {
                let town_idx = msg.town_idx as usize;
                if town_idx < by_town.len() {
                    by_town[town_idx].push(idx);
                }
            }
        }
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
