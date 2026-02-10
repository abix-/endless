//! Spawn systems - Create Bevy entities from spawn events

use bevy::prelude::*;

use crate::components::*;
use crate::constants::*;
use crate::messages::{SpawnNpcMsg, GpuUpdate, GpuUpdateMsg, GPU_DISPATCH_COUNT};
use crate::resources::{
    NpcCount, NpcEntityMap, PopulationStats, GpuDispatchCount, NpcMetaCache, NpcMeta,
    NpcsByTownCache, FactionStats, GameTime, CombatLog, CombatEventKind, ReassignQueue,
};
use crate::systems::economy::*;
use crate::world::{WorldData, FarmOccupancy, LocationKind, find_nearest_location, pos_to_key};

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

/// Generate a random personality with 0-2 traits.
/// Each trait has 30% chance, magnitude 0.5-1.5.
fn generate_personality(slot: usize) -> Personality {
    // Simple deterministic "random" based on slot for reproducibility
    let seed = slot as u32;
    let r1 = ((seed.wrapping_mul(1103515245).wrapping_add(12345)) >> 16) & 0x7fff;
    let r2 = ((seed.wrapping_mul(1103515245).wrapping_add(12345).wrapping_mul(1103515245).wrapping_add(12345)) >> 16) & 0x7fff;
    let r3 = (r1.wrapping_mul(1103515245).wrapping_add(12345) >> 16) & 0x7fff;
    let r4 = (r2.wrapping_mul(1103515245).wrapping_add(12345) >> 16) & 0x7fff;

    let mut traits = Vec::new();

    // 30% chance for each trait
    let kinds = [TraitKind::Brave, TraitKind::Tough, TraitKind::Swift, TraitKind::Focused];
    let randoms = [r1, r2, r3, r4];

    for (i, &kind) in kinds.iter().enumerate() {
        if randoms[i] % 100 < 30 {
            // Magnitude 0.5 to 1.5
            let mag = 0.5 + ((randoms[i] % 1000) as f32 / 1000.0);
            traits.push(TraitInstance { kind, magnitude: mag });
        }
    }

    // Keep at most 2
    Personality {
        trait1: traits.get(0).copied(),
        trait2: traits.get(1).copied(),
    }
}

/// Generic spawn system. Job determines the component template.
/// All GPU writes go through GpuUpdateMsg messages (collected at end of frame).
pub fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    mut count: ResMut<NpcCount>,
    mut npc_map: ResMut<NpcEntityMap>,
    mut pop_stats: ResMut<PopulationStats>,
    mut faction_stats: ResMut<FactionStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    world_data: Res<WorldData>,
    game_time: Res<GameTime>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut npcs_by_town: ResMut<NpcsByTownCache>,
    mut gpu_dispatch: ResMut<GpuDispatchCount>,
    mut combat_log: ResMut<CombatLog>,
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

        // GPU writes via messages — collected at end of frame
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

        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx, x: msg.x, y: msg.y }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: target_x, y: target_y }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: 100.0 }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx, faction: msg.faction }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: 100.0 }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx, col: sprite_col, row: sprite_row }));

        // Generate personality for this NPC
        let personality = generate_personality(idx);

        // Base entity (all NPCs get these)
        let current_hour = game_time.total_hours();
        let mut ec = commands.spawn((
            NpcIndex(idx),
            Position::new(msg.x, msg.y),
            job,
            TownId(msg.town_idx),
            Speed::default(),
            Health::default(),
            MaxHealth::default(),
            Faction::from_i32(msg.faction),
            Home(Vec2::new(msg.home_x, msg.home_y)),
            personality,
            LastAteHour(current_hour),
            Activity::default(),
            CombatState::default(),
        ));

        // Job template — determines component bundle
        match job {
            Job::Guard => {
                ec.insert(Energy::default());
                ec.insert((AttackStats::melee(), AttackTimer(0.0)));
                ec.insert(Guard);
                ec.insert((EquippedWeapon(EQUIP_SWORD.0, EQUIP_SWORD.1), EquippedHelmet(EQUIP_HELMET.0, EQUIP_HELMET.1)));
                if msg.starting_post >= 0 {
                    let patrol_posts = build_patrol_route(&world_data, msg.town_idx as u32);
                    ec.insert(PatrolRoute {
                        posts: patrol_posts,
                        current: msg.starting_post as usize,
                    });
                    ec.insert(Activity::OnDuty { ticks_waiting: 0 });
                }
            }
            Job::Farmer => {
                ec.insert(Energy::default());
                ec.insert(Farmer);
                if msg.work_x >= 0.0 {
                    ec.insert(WorkPosition(Vec2::new(msg.work_x, msg.work_y)));
                    ec.insert(Activity::GoingToWork);
                }
            }
            Job::Raider => {
                ec.insert(Energy::default());
                ec.insert((AttackStats::melee(), AttackTimer(0.0)));
                ec.insert(Stealer);
                ec.insert(EquippedWeapon(EQUIP_SWORD.0, EQUIP_SWORD.1));
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
        faction_stats.inc_alive(msg.faction);

        // Initialize NPC metadata for UI queries
        if idx < npc_meta.0.len() {
            npc_meta.0[idx] = NpcMeta {
                name: generate_name(job, idx),
                level: 1,
                xp: 0,
                trait_id: (idx % 5) as i32,  // Simple trait assignment (0-4)
                town_id: msg.town_idx,
                job: msg.job,
            };
        }

        // Add to per-town NPC list
        if msg.town_idx >= 0 {
            let town_idx = msg.town_idx as usize;
            if town_idx < npcs_by_town.0.len() {
                npcs_by_town.0[town_idx].push(idx);
            }
        }

        // Combat log: spawn event (only after initial startup, day > 1 or hour > 6)
        if game_time.total_hours() > 0 {
            let job_str = crate::job_name(msg.job);
            combat_log.push(CombatEventKind::Spawn,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} #{} spawned", job_str, idx));
        }
    }

    // Update GPU dispatch count so process() includes these NPCs
    if had_spawns && max_slot > gpu_dispatch.0 {
        gpu_dispatch.0 = max_slot;
        // Also update static for lib.rs process() to read
        if let Ok(mut c) = GPU_DISPATCH_COUNT.lock() {
            if max_slot > *c {
                *c = max_slot;
            }
        }
    }
}

/// Build sorted patrol route from WorldData for a given town.
fn build_patrol_route(world: &WorldData, town_idx: u32) -> Vec<Vec2> {
    let mut posts: Vec<(u32, Vec2)> = world.guard_posts.iter()
        .filter(|p| p.town_idx == town_idx)
        .map(|p| (p.patrol_order, p.position))
        .collect();
    posts.sort_by_key(|(order, _)| *order);
    posts.into_iter().map(|(_, pos)| pos).collect()
}

/// Process role reassignment requests (Farmer <-> Guard).
/// Drains ReassignQueue, swaps job components, updates GPU sprite + population stats.
pub fn reassign_npc_system(
    mut commands: Commands,
    mut queue: ResMut<ReassignQueue>,
    npc_map: Res<NpcEntityMap>,
    mut pop_stats: ResMut<PopulationStats>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    world_data: Res<WorldData>,
    mut farm_occupancy: ResMut<FarmOccupancy>,
    game_time: Res<GameTime>,
    mut combat_log: ResMut<CombatLog>,
    query: Query<(&Job, &TownId, &NpcIndex, &Position, Option<&AssignedFarm>), Without<Dead>>,
) {
    for (slot, new_job) in queue.0.drain(..) {
        let Some(&entity) = npc_map.0.get(&slot) else { continue };
        let Ok((job, town_id, npc_idx, position, assigned_farm)) = query.get(entity) else { continue };

        let idx = npc_idx.0;
        let town = town_id.0;
        let old_job = *job;
        let pos = Vec2::new(position.x, position.y);

        match (old_job, new_job) {
            (Job::Farmer, 1) => {
                // Farmer → Guard
                // Release farm occupancy
                if let Some(farm) = assigned_farm {
                    let key = pos_to_key(farm.0);
                    if let Some(count) = farm_occupancy.occupants.get_mut(&key) {
                        *count = (*count - 1).max(0);
                    }
                }

                // Remove farmer components, insert guard components
                commands.entity(entity)
                    .remove::<Farmer>()
                    .remove::<WorkPosition>()
                    .remove::<AssignedFarm>()
                    .insert(Guard)
                    .insert(Job::Guard)
                    .insert((AttackStats::melee(), AttackTimer(0.0)))
                    .insert((EquippedWeapon(EQUIP_SWORD.0, EQUIP_SWORD.1), EquippedHelmet(EQUIP_HELMET.0, EQUIP_HELMET.1)))
                    .insert(CombatState::None);

                // Build patrol route and set activity
                let patrol = build_patrol_route(&world_data, town as u32);
                if !patrol.is_empty() {
                    commands.entity(entity)
                        .insert(PatrolRoute { posts: patrol, current: 0 })
                        .insert(Activity::OnDuty { ticks_waiting: 0 });
                } else {
                    commands.entity(entity).insert(Activity::Idle);
                }

                // GPU sprite
                let (col, row) = SPRITE_GUARD;
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx, col, row }));

                // Population stats
                pop_dec_alive(&mut pop_stats, Job::Farmer, town);
                pop_inc_alive(&mut pop_stats, Job::Guard, town);

                // Meta cache
                if idx < npc_meta.0.len() {
                    npc_meta.0[idx].job = 1;
                }

                let name = npc_meta.0[idx].name.clone();
                combat_log.push(CombatEventKind::Spawn,
                    game_time.day(), game_time.hour(), game_time.minute(),
                    format!("{} reassigned: Farmer → Guard", name));
            }
            (Job::Guard, 0) => {
                // Guard → Farmer
                // Remove guard components, insert farmer
                commands.entity(entity)
                    .remove::<Guard>()
                    .remove::<AttackStats>()
                    .remove::<AttackTimer>()
                    .remove::<EquippedWeapon>()
                    .remove::<EquippedHelmet>()
                    .remove::<PatrolRoute>()
                    .insert(Farmer)
                    .insert(Job::Farmer)
                    .insert(CombatState::None);

                // Find nearest farm for work assignment
                if let Some(farm_pos) = find_nearest_location(pos, &world_data, LocationKind::Farm) {
                    commands.entity(entity)
                        .insert(WorkPosition(farm_pos))
                        .insert(Activity::GoingToWork);
                } else {
                    commands.entity(entity).insert(Activity::Idle);
                }

                // GPU sprite
                let (col, row) = SPRITE_FARMER;
                gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx, col, row }));

                // Population stats
                pop_dec_alive(&mut pop_stats, Job::Guard, town);
                pop_inc_alive(&mut pop_stats, Job::Farmer, town);

                // Meta cache
                if idx < npc_meta.0.len() {
                    npc_meta.0[idx].job = 0;
                }

                let name = npc_meta.0[idx].name.clone();
                combat_log.push(CombatEventKind::Spawn,
                    game_time.day(), game_time.hour(), game_time.minute(),
                    format!("{} reassigned: Guard → Farmer", name));
            }
            _ => {} // Invalid reassignment — skip
        }
    }
}
