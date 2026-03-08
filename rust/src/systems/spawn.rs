//! Spawn systems - Create Bevy entities from spawn events

use bevy::prelude::*;

use crate::components::*;
use crate::messages::{CombatLogMsg, GpuUpdate, GpuUpdateMsg, SpawnNpcMsg};
use crate::messages::{DirtyWriters, MiningDirtyMsg, SquadsDirtyMsg};
use crate::resources::NextEntityUid;
use crate::resources::{
    CombatEventKind, EntityMap, FactionStats, GameTime, NpcMeta, NpcMetaCache, NpcsByTownCache,
    PopulationStats,
};
use crate::systems::economy::*;
use crate::systems::stats::{CombatConfig, TownUpgrades, resolve_combat_stats};
use crate::world::{BuildingKind, WorldData};

// Name generation word lists
const ADJECTIVES: &[&str] = &[
    "Swift", "Brave", "Calm", "Bold", "Sharp", "Quick", "Stern", "Wise", "Keen", "Strong",
];
const FARMER_NOUNS: &[&str] = &["Tiller", "Sower", "Reaper", "Plower", "Grower"];
const ARCHER_NOUNS: &[&str] = &["Shield", "Sword", "Watcher", "Sentinel", "Defender"];
const RAIDER_NOUNS: &[&str] = &["Blade", "Fang", "Shadow", "Claw", "Storm"];
const MINER_NOUNS: &[&str] = &["Digger", "Pickaxe", "Prospector", "Delver", "Stonecutter"];
const CROSSBOW_NOUNS: &[&str] = &["Bolt", "Marksman", "Sniper", "Hunter", "Striker"];

fn generate_name(job: Job, slot: usize) -> String {
    let adj = ADJECTIVES[slot % ADJECTIVES.len()];
    let noun = match job {
        Job::Farmer => FARMER_NOUNS[(slot / ADJECTIVES.len()) % FARMER_NOUNS.len()],
        Job::Archer => ARCHER_NOUNS[(slot / ADJECTIVES.len()) % ARCHER_NOUNS.len()],
        Job::Raider => RAIDER_NOUNS[(slot / ADJECTIVES.len()) % RAIDER_NOUNS.len()],
        Job::Fighter => "Fighter",
        Job::Miner => MINER_NOUNS[(slot / ADJECTIVES.len()) % MINER_NOUNS.len()],
        Job::Crossbow => CROSSBOW_NOUNS[(slot / ADJECTIVES.len()) % CROSSBOW_NOUNS.len()],
        Job::Boat => "Boat",
    };
    format!("{} {}", adj, noun)
}

/// Generate a random personality with 0-2 spectrum traits.
/// Each of 7 axes has ~20% chance. Magnitude ±0.5 to ±1.5 (sign = pole).
fn generate_personality(slot: usize) -> Personality {
    // Deterministic LCG chain seeded by slot index
    let mut s = slot as u32;
    let mut next = || -> u32 {
        s = s.wrapping_mul(1103515245).wrapping_add(12345);
        (s >> 16) & 0x7fff
    };

    let mut traits = Vec::new();

    // 20% chance per axis, cap at 2
    for &kind in &TraitKind::ALL {
        if traits.len() >= 2 { break; }
        let r = next();
        if r % 100 < 20 {
            let mag = 0.5 + ((r % 1000) as f32 / 1000.0); // 0.5..1.5
            let sign = if next() % 2 == 0 { 1.0 } else { -1.0 };
            traits.push(TraitInstance {
                kind,
                magnitude: sign * mag,
            });
        }
    }

    Personality {
        trait1: traits.first().copied(),
        trait2: traits.get(1).copied(),
    }
}

// ============================================================================
// SHARED SPAWN HELPER — single source of truth for NPC materialization
// ============================================================================

/// Optional overrides for save-loaded NPCs. Fresh spawns pass all None.
#[derive(Default)]
pub struct NpcSpawnOverrides {
    pub health: Option<f32>,
    pub energy: Option<f32>,
    pub activity: Option<Activity>,
    pub combat_state: Option<CombatState>,
    pub personality: Option<Personality>,
    pub name: Option<String>,
    pub level: Option<i32>,
    pub xp: Option<i32>,
    pub equipment: NpcEquipment,
    pub carried_food: Option<i32>,
    pub carried_gold: Option<i32>,
    pub carried_equipment: Vec<crate::constants::LootItem>,
    pub squad_id: Option<i32>,
    /// Explicit UID for save/load. None = allocate fresh from NextEntityUid.
    pub uid_override: Option<EntityUid>,
}


/// Shared NPC spawn: creates entity, emits GPU updates, registers in tracking caches.
/// Used by both spawn_npc_system (fresh spawn) and spawn_npcs_from_save (load).
pub fn materialize_npc(
    slot_idx: usize,
    x: f32,
    y: f32,
    job_id: i32,
    faction_id: i32,
    town_idx: i32,
    home: [f32; 2],
    work_pos: Option<[f32; 2]>,
    starting_post: i32,
    attack_type_id: i32,
    overrides: &NpcSpawnOverrides,
    commands: &mut Commands,
    entity_map: &mut EntityMap,
    pop_stats: &mut PopulationStats,
    npc_meta: &mut NpcMetaCache,
    npcs_by_town: &mut NpcsByTownCache,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    _world_data: &WorldData,
    combat_config: &CombatConfig,
    upgrades: &TownUpgrades,
    uid_alloc: &mut NextEntityUid,
) {
    let idx = slot_idx;
    let job = Job::from_i32(job_id);
    let attack_type = if attack_type_id == 1 {
        BaseAttackType::Ranged
    } else {
        BaseAttackType::Melee
    };
    let personality = overrides
        .personality
        .clone()
        .unwrap_or_else(|| generate_personality(idx));
    let level = overrides.level.unwrap_or(0);

    let cached = resolve_combat_stats(
        job,
        attack_type,
        town_idx,
        level,
        &personality,
        combat_config,
        upgrades,
        overrides.equipment.total_weapon_bonus(),
        overrides.equipment.total_armor_bonus(),
    );

    // GPU init
    let health = overrides.health.unwrap_or(cached.max_health);
    // Fresh spawns should not start with a work target; behavior assigns work later.
    // Save/restore path (activity override) may restore explicit work targets.
    let restore_work_state = overrides.activity.is_some();
    let (target_x, target_y) = if let (true, Job::Farmer, Some(wp)) = (restore_work_state, job, work_pos.as_ref()) {
        (wp[0], wp[1])
    } else {
        (x, y)
    };
    let def = crate::constants::npc_def(job);
    let (sprite_col, sprite_row) = def.sprite;

    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx, x, y }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget {
        idx,
        x: target_x,
        y: target_y,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed {
        idx,
        speed: cached.speed,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction {
        idx,
        faction: faction_id,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetMaxHealth { idx, max_health: cached.max_health }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame {
        idx,
        col: sprite_col,
        row: sprite_row,
        atlas: def.atlas,
    }));
    let combat_flags = if job.is_military() { 1u32 } else { 0u32 };
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFlags {
        idx,
        flags: combat_flags,
    }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHalfSize {
        idx,
        half_w: crate::constants::NPC_HITBOX_HALF[0],
        half_h: crate::constants::NPC_HITBOX_HALF[1],
    }));

    // Resolve spawn data
    let activity = overrides.activity.clone().unwrap_or_default();
    let combat_state = overrides.combat_state.clone().unwrap_or_default();

    let trait_display = personality.trait_summary();

    // Patrol route
    let patrol_route = if def.is_patrol_unit && starting_post >= 0 {
        let patrol_posts = build_patrol_route(entity_map, town_idx as u32);
        if !patrol_posts.is_empty() {
            Some(PatrolRoute {
                posts: patrol_posts,
                current: starting_post as usize,
            })
        } else {
            None
        }
    } else {
        None
    };

    // Work position slot
    let work_slot = work_pos.and_then(|wp| entity_map.slot_at_position(Vec2::new(wp[0], wp[1])));
    let initial_work_target = if restore_work_state {
        work_slot.and_then(|s| entity_map.uid_for_slot(s))
    } else {
        None
    };

    // Resolve final activity (patrol/work overrides)
    let activity = if overrides.activity.is_some() {
        activity // save-loaded activity takes precedence
    } else if patrol_route.is_some() {
        Activity::OnDuty { ticks_waiting: 0 }
    } else if initial_work_target.is_some() {
        Activity::GoingToWork
    } else {
        activity
    };

    // Equipment
    let npc_equipment = overrides.equipment.clone();

    // Spawn ECS entity with all NPC components
    let energy_val = if def.has_energy {
        overrides.energy.unwrap_or(100.0)
    } else {
        0.0
    };
    let home_vec = Vec2::new(home[0], home[1]);
    let mut ecmds = commands.spawn((
        // Identity
        (GpuSlot(idx), job, Faction(faction_id), TownId(town_idx)),
        // State
        (
            NpcFlags::default(),
            activity.clone(),
            Position { x, y },
            Home(home_vec),
        ),
        // Combat
        (
            Health(health),
            Energy(energy_val),
            Speed(cached.speed),
            combat_state.clone(),
        ),
        // Stats
        (cached.clone(), attack_type, AttackTimer(0.0), personality),
        // Economy
        CarriedLoot {
            food: overrides.carried_food.unwrap_or(0),
            gold: overrides.carried_gold.unwrap_or(0),
            equipment: overrides.carried_equipment.clone(),
        },
        // Work state (always present)
        NpcWorkState {
            worksite: initial_work_target,
        },
        // Pathfinding (empty until first path request)
        NpcPath::default(),
    ));
    if let Some(sq) = overrides.squad_id {
        ecmds.insert(SquadId(sq));
    }
    if let Some(pr) = patrol_route {
        ecmds.insert(pr);
    }
    ecmds.insert(npc_equipment);
    if let Some(lr) = def.leash_range {
        ecmds.insert(LeashRange(lr));
    }
    if def.stealer {
        ecmds.insert(Stealer);
    }
    if def.has_energy {
        ecmds.insert(HasEnergy);
    }
    let uid = overrides.uid_override.unwrap_or_else(|| uid_alloc.alloc());
    ecmds.insert(uid);
    let entity = ecmds.id();

    entity_map.register_npc(idx, entity, job, faction_id, town_idx);
    entity_map.register_uid(idx, uid, entity);
    pop_inc_alive(pop_stats, job, town_idx);

    if idx < npc_meta.0.len() {
        npc_meta.0[idx] = NpcMeta {
            name: overrides
                .name
                .clone()
                .unwrap_or_else(|| generate_name(job, idx)),
            level,
            xp: overrides.xp.unwrap_or(0),
            trait_display,
            town_id: town_idx,
            job: job_id,
        };
    }

    if town_idx >= 0 {
        let ti = town_idx as usize;
        if ti < npcs_by_town.0.len() {
            npcs_by_town.0[ti].push(idx);
        }
    }
}

/// Generic spawn system. Job determines the component template.
/// All GPU writes go through GpuUpdateMsg messages (collected at end of frame).
pub fn spawn_npc_system(
    mut commands: Commands,
    mut events: MessageReader<SpawnNpcMsg>,
    mut entity_map: ResMut<EntityMap>,
    mut pop_stats: ResMut<PopulationStats>,
    mut faction_stats: ResMut<FactionStats>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    world_data: Res<WorldData>,
    game_time: Res<GameTime>,
    mut npc_meta: ResMut<NpcMetaCache>,
    mut npcs_by_town: ResMut<NpcsByTownCache>,
    mut combat_log: MessageWriter<CombatLogMsg>,
    combat_config: Res<CombatConfig>,
    upgrades: Res<TownUpgrades>,
    mut dirty_writers: DirtyWriters,
    mut uid_alloc: ResMut<NextEntityUid>,
) {
    for msg in events.read() {
        let work_pos = if msg.work_x >= 0.0 {
            Some([msg.work_x, msg.work_y])
        } else {
            None
        };

        let overrides = if msg.uid_override.is_some() {
            NpcSpawnOverrides { uid_override: msg.uid_override, ..Default::default() }
        } else {
            NpcSpawnOverrides::default()
        };
        materialize_npc(
            msg.slot_idx,
            msg.x,
            msg.y,
            msg.job,
            msg.faction,
            msg.town_idx,
            [msg.home_x, msg.home_y],
            work_pos,
            msg.starting_post,
            msg.attack_type,
            &overrides,
            &mut commands,
            &mut entity_map,
            &mut pop_stats,
            &mut npc_meta,
            &mut npcs_by_town,
            &mut gpu_updates,
            &world_data,
            &combat_config,
            &upgrades,
            &mut uid_alloc,
        );

        // Spawn-only bookkeeping (not needed for save-load)
        let job = Job::from_i32(msg.job);
        faction_stats.inc_alive(msg.faction);
        if job == Job::Miner {
            dirty_writers.mining.write(MiningDirtyMsg);
        }
        if crate::constants::npc_def(job).is_military {
            dirty_writers.squads.write(SquadsDirtyMsg);
        }

        if game_time.total_hours() > 0 {
            let job_str = crate::job_name(msg.job);
            combat_log.write(CombatLogMsg {
                kind: CombatEventKind::Spawn,
                faction: msg.faction,
                day: game_time.day(),
                hour: game_time.hour(),
                minute: game_time.minute(),
                message: format!("{} #{} spawned", job_str, msg.slot_idx),
                location: None,
            });
        }
    }
}

/// Build sorted patrol route from EntityMap for a given town.
pub(crate) fn build_patrol_route(entity_map: &EntityMap, town_idx: u32) -> Vec<Vec2> {
    let mut posts: Vec<(u32, Vec2)> = entity_map
        .iter_kind_for_town(BuildingKind::Waypoint, town_idx)
        .map(|inst| (inst.patrol_order, inst.position))
        .collect();
    posts.sort_by_key(|(order, _)| *order);
    posts.into_iter().map(|(_, pos)| pos).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_name_deterministic() {
        let a = generate_name(Job::Archer, 42);
        let b = generate_name(Job::Archer, 42);
        assert_eq!(a, b, "same job+slot should produce same name");
    }

    #[test]
    fn generate_name_different_slots() {
        let names: Vec<String> = (0..50).map(|s| generate_name(Job::Archer, s)).collect();
        let unique: std::collections::HashSet<&String> = names.iter().collect();
        assert!(unique.len() > 1, "different slots should produce different names");
    }

    #[test]
    fn generate_name_all_jobs() {
        let jobs = [Job::Farmer, Job::Archer, Job::Raider, Job::Fighter, Job::Miner, Job::Crossbow, Job::Boat];
        for job in jobs {
            let name = generate_name(job, 0);
            assert!(!name.is_empty(), "job {:?} should produce a non-empty name", job);
            assert!(name.contains(' '), "name should be 'Adj Noun': {name}");
        }
    }

    #[test]
    fn generate_personality_deterministic() {
        let a = generate_personality(42);
        let b = generate_personality(42);
        assert_eq!(a.trait1.map(|t| (t.kind, t.magnitude.to_bits())),
                   b.trait1.map(|t| (t.kind, t.magnitude.to_bits())),
                   "same slot should produce same personality");
    }

    #[test]
    fn generate_personality_some_have_traits() {
        let with_traits = (0..100)
            .filter(|&s| generate_personality(s).trait1.is_some())
            .count();
        assert!(with_traits > 0, "at least some personalities should have traits");
        assert!(with_traits < 100, "not all personalities should have traits (20% chance per axis)");
    }

    #[test]
    fn generate_personality_max_two_traits() {
        for slot in 0..1000 {
            let p = generate_personality(slot);
            if p.trait2.is_some() {
                assert!(p.trait1.is_some(), "trait2 without trait1 at slot {slot}");
            }
            // No trait3 field exists — struct enforces max 2 by design
        }
    }
}
