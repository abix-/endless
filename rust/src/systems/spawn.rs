//! Spawn systems - Create Bevy entities from spawn events

use bevy::prelude::*;

use crate::components::*;
use crate::constants::*;
use crate::messages::{SpawnNpcMsg, GpuUpdate, GpuUpdateMsg};
use crate::resources::{
    NpcEntityMap, PopulationStats, NpcMetaCache, NpcMeta,
    NpcsByTownCache, FactionStats, GameTime, CombatLog, CombatEventKind, SystemTimings, DirtyFlags,
};
use crate::systems::stats::{CombatConfig, TownUpgrades, resolve_combat_stats};
use crate::systems::economy::*;
use crate::world::WorldData;

// Name generation word lists
const ADJECTIVES: &[&str] = &["Swift", "Brave", "Calm", "Bold", "Sharp", "Quick", "Stern", "Wise", "Keen", "Strong"];
const FARMER_NOUNS: &[&str] = &["Tiller", "Sower", "Reaper", "Plower", "Grower"];
const ARCHER_NOUNS: &[&str] = &["Shield", "Sword", "Watcher", "Sentinel", "Defender"];
const RAIDER_NOUNS: &[&str] = &["Blade", "Fang", "Shadow", "Claw", "Storm"];
const MINER_NOUNS: &[&str] = &["Digger", "Pickaxe", "Prospector", "Delver", "Stonecutter"];


fn generate_name(job: Job, slot: usize) -> String {
    let adj = ADJECTIVES[slot % ADJECTIVES.len()];
    let noun = match job {
        Job::Farmer => FARMER_NOUNS[(slot / ADJECTIVES.len()) % FARMER_NOUNS.len()],
        Job::Archer => ARCHER_NOUNS[(slot / ADJECTIVES.len()) % ARCHER_NOUNS.len()],
        Job::Raider => RAIDER_NOUNS[(slot / ADJECTIVES.len()) % RAIDER_NOUNS.len()],
        Job::Fighter => "Fighter",
        Job::Miner => MINER_NOUNS[(slot / ADJECTIVES.len()) % MINER_NOUNS.len()],
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
    mut npc_map: ResMut<NpcEntityMap>,
    mut pop_stats: ResMut<PopulationStats>,
    mut faction_stats: ResMut<FactionStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    world_data: Res<WorldData>,
    game_time: Res<GameTime>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut npcs_by_town: ResMut<NpcsByTownCache>,
    mut combat_log: ResMut<CombatLog>,
    combat_config: Res<CombatConfig>,
    upgrades: Res<TownUpgrades>,
    timings: Res<SystemTimings>,
    mut dirty: ResMut<DirtyFlags>,
) {
    let _t = timings.scope("spawn_npc");
    for msg in events.read() {
        let idx = msg.slot_idx;
        let job = Job::from_i32(msg.job);

        // Determine attack type (farmers default to Melee — stats exist but unused)
        let attack_type = if msg.attack_type == 1 { BaseAttackType::Ranged } else { BaseAttackType::Melee };

        // Generate personality for this NPC
        let personality = generate_personality(idx);

        // Resolve stats from config
        let cached = resolve_combat_stats(
            job, attack_type, msg.town_idx, 0, &personality, &combat_config, &upgrades,
        );

        // GPU writes via messages — collected at end of frame
        let (target_x, target_y) = if job == Job::Farmer && msg.work_x >= 0.0 {
            (msg.work_x, msg.work_y)
        } else {
            (msg.x, msg.y)
        };
        let (sprite_col, sprite_row) = match job {
            Job::Farmer => SPRITE_FARMER,
            Job::Archer => SPRITE_ARCHER,
            Job::Raider => SPRITE_RAIDER,
            Job::Fighter => SPRITE_FIGHTER,
            Job::Miner => SPRITE_MINER,
        };

        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx, x: msg.x, y: msg.y }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: target_x, y: target_y }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx, faction: msg.faction }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health: cached.max_health }));
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx, col: sprite_col, row: sprite_row, atlas: 0.0 }));
        // Combat scan flag: fighters need full 81-cell scan, others get reduced threat-only scan
        let combat_flags = if matches!(job, Job::Archer | Job::Raider | Job::Fighter) { 1u32 } else { 0u32 };
        gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFlags { idx, flags: combat_flags }));

        // Base entity (all NPCs get these)
        let mut ec = commands.spawn((
            NpcIndex(idx),
            Position::new(msg.x, msg.y),
            job,
            TownId(msg.town_idx),
            Speed(cached.speed),
            Health(cached.max_health),
            cached,
            attack_type,
            Faction::from_i32(msg.faction),
            Home(Vec2::new(msg.home_x, msg.home_y)),
            personality,
            Activity::default(),
            CombatState::default(),
        ));

        // Job template — determines component bundle
        match job {
            Job::Archer => {
                dirty.squads = true;
                ec.insert(Energy::default());
                ec.insert(AttackTimer(0.0));
                ec.insert(Archer);
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
                ec.insert(AttackTimer(0.0));
                ec.insert(Stealer);
                ec.insert(EquippedWeapon(EQUIP_SWORD.0, EQUIP_SWORD.1));
                ec.insert(LeashRange { distance: 400.0 });
            }
            Job::Miner => {
                dirty.mining = true;
                ec.insert(Energy::default());
                ec.insert(Miner);
            }
            Job::Fighter => {
                ec.insert(AttackTimer(0.0));
            }
        }

        npc_map.0.insert(idx, ec.id());
        pop_inc_alive(&mut pop_stats, job, msg.town_idx);
        faction_stats.inc_alive(msg.faction);

        // Initialize NPC metadata for UI queries
        if idx < npc_meta.0.len() {
            npc_meta.0[idx] = NpcMeta {
                name: generate_name(job, idx),
                level: 0,
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
            combat_log.push(CombatEventKind::Spawn, msg.faction,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} #{} spawned", job_str, idx));
        }
    }

}

/// Build sorted patrol route from WorldData for a given town.
pub(crate) fn build_patrol_route(world: &WorldData, town_idx: u32) -> Vec<Vec2> {
    let mut posts: Vec<(u32, Vec2)> = world.waypoints.iter()
        .filter(|p| p.town_idx == town_idx && crate::world::is_alive(p.position))
        .map(|p| (p.patrol_order, p.position))
        .collect();
    posts.sort_by_key(|(order, _)| *order);
    posts.into_iter().map(|(_, pos)| pos).collect()
}
