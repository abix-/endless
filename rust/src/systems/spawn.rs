//! Spawn systems - Create Bevy entities from spawn events

use bevy::prelude::*;

use crate::components::*;
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

// ============================================================================
// SHARED SPAWN HELPER — single source of truth for NPC materialization
// ============================================================================

/// Optional overrides for save-loaded NPCs. Fresh spawns pass all None.
pub struct NpcSpawnOverrides {
    pub health: Option<f32>,
    pub energy: Option<f32>,
    pub activity: Option<Activity>,
    pub combat_state: Option<CombatState>,
    pub personality: Option<Personality>,
    pub name: Option<String>,
    pub level: Option<i32>,
    pub xp: Option<i32>,
    pub weapon: Option<[f32; 2]>,
    pub helmet: Option<[f32; 2]>,
    pub armor: Option<[f32; 2]>,
    pub carried_gold: Option<i32>,
    pub squad_id: Option<i32>,
}

impl Default for NpcSpawnOverrides {
    fn default() -> Self {
        Self {
            health: None, energy: None, activity: None, combat_state: None,
            personality: None, name: None, level: None, xp: None,
            weapon: None, helmet: None, armor: None, carried_gold: None, squad_id: None,
        }
    }
}

/// Shared NPC spawn: creates entity, emits GPU updates, registers in tracking caches.
/// Used by both spawn_npc_system (fresh spawn) and spawn_npcs_from_save (load).
pub fn materialize_npc(
    slot_idx: usize,
    x: f32, y: f32,
    job_id: i32,
    faction_id: i32,
    town_idx: i32,
    home: [f32; 2],
    work_pos: Option<[f32; 2]>,
    starting_post: i32,
    attack_type_id: i32,
    overrides: &NpcSpawnOverrides,
    commands: &mut Commands,
    npc_map: &mut NpcEntityMap,
    pop_stats: &mut PopulationStats,
    npc_meta: &mut NpcMetaCache,
    npcs_by_town: &mut NpcsByTownCache,
    gpu_updates: &mut MessageWriter<GpuUpdateMsg>,
    world_data: &WorldData,
    combat_config: &CombatConfig,
    upgrades: &TownUpgrades,
) {
    let idx = slot_idx;
    let job = Job::from_i32(job_id);
    let attack_type = if attack_type_id == 1 { BaseAttackType::Ranged } else { BaseAttackType::Melee };
    let personality = overrides.personality.clone().unwrap_or_else(|| generate_personality(idx));
    let level = overrides.level.unwrap_or(0);

    let cached = resolve_combat_stats(
        job, attack_type, town_idx, level, &personality, combat_config, upgrades,
    );

    // GPU init
    let health = overrides.health.unwrap_or(cached.max_health);
    let (target_x, target_y) = if job == Job::Farmer && work_pos.is_some() {
        let wp = work_pos.unwrap();
        (wp[0], wp[1])
    } else {
        (x, y)
    };
    let def = crate::constants::npc_def(job);
    let (sprite_col, sprite_row) = def.sprite;

    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetPosition { idx, x, y }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetTarget { idx, x: target_x, y: target_y }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpeed { idx, speed: cached.speed }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFaction { idx, faction: faction_id }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetHealth { idx, health }));
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetSpriteFrame { idx, col: sprite_col, row: sprite_row, atlas: 0.0 }));
    let combat_flags = if job.is_military() { 1u32 } else { 0u32 };
    gpu_updates.write(GpuUpdateMsg(GpuUpdate::SetFlags { idx, flags: combat_flags }));

    // Entity with base components
    let activity = overrides.activity.clone().unwrap_or_default();
    let combat_state = overrides.combat_state.clone().unwrap_or_default();

    let mut ec = commands.spawn((
        NpcIndex(idx),
        Position::new(x, y),
        job,
        TownId(town_idx),
        Speed(cached.speed),
        Health(health),
        cached,
        attack_type,
        Faction::from_i32(faction_id),
        Home(Vec2::new(home[0], home[1])),
        personality,
        activity,
        combat_state,
    ));

    // Data-driven components from NPC registry
    if def.has_energy {
        ec.insert(Energy(overrides.energy.unwrap_or(100.0)));
    }
    if def.has_attack_timer { ec.insert(AttackTimer(0.0)); }
    if let Some(w_default) = def.weapon {
        let w = overrides.weapon.unwrap_or([w_default.0, w_default.1]);
        ec.insert(EquippedWeapon(w[0], w[1]));
    }
    if let Some(h_default) = def.helmet {
        let h = overrides.helmet.unwrap_or([h_default.0, h_default.1]);
        ec.insert(EquippedHelmet(h[0], h[1]));
    }
    if def.stealer { ec.insert(Stealer); }
    if let Some(d) = def.leash_range { ec.insert(LeashRange { distance: d }); }

    // Marker components
    match job {
        Job::Archer => { ec.insert(Archer); }
        Job::Farmer => { ec.insert(Farmer); }
        Job::Miner => { ec.insert(Miner); }
        Job::Crossbow => { ec.insert(Crossbow); }
        _ => {}
    }
    if def.is_military { ec.insert(SquadUnit); }

    // Save-specific optional components
    if let Some(a) = overrides.armor { ec.insert(EquippedArmor(a[0], a[1])); }
    if let Some(cg) = overrides.carried_gold { ec.insert(CarriedGold(cg)); }
    if let Some(sq) = overrides.squad_id { ec.insert(SquadId(sq)); }

    // Patrol route
    if def.is_patrol_unit && starting_post >= 0 {
        let patrol_posts = build_patrol_route(world_data, town_idx as u32);
        if !patrol_posts.is_empty() {
            ec.insert(PatrolRoute { posts: patrol_posts, current: starting_post as usize });
            // Only set OnDuty for fresh spawns (save restores activity via override)
            if overrides.activity.is_none() {
                ec.insert(Activity::OnDuty { ticks_waiting: 0 });
            }
        }
    }

    // Work position
    if let Some(wp) = work_pos {
        ec.insert(WorkPosition(Vec2::new(wp[0], wp[1])));
        if overrides.activity.is_none() {
            ec.insert(Activity::GoingToWork);
        }
    }

    // Register in tracking caches
    npc_map.0.insert(idx, ec.id());
    pop_inc_alive(pop_stats, job, town_idx);

    if idx < npc_meta.0.len() {
        npc_meta.0[idx] = NpcMeta {
            name: overrides.name.clone().unwrap_or_else(|| generate_name(job, idx)),
            level,
            xp: overrides.xp.unwrap_or(0),
            trait_id: (idx % 5) as i32,
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
        let work_pos = if msg.work_x >= 0.0 { Some([msg.work_x, msg.work_y]) } else { None };

        materialize_npc(
            msg.slot_idx, msg.x, msg.y, msg.job, msg.faction, msg.town_idx,
            [msg.home_x, msg.home_y], work_pos, msg.starting_post, msg.attack_type,
            &NpcSpawnOverrides::default(),
            &mut commands, &mut npc_map, &mut pop_stats, &mut npc_meta,
            &mut npcs_by_town, &mut gpu_updates, &world_data, &combat_config, &upgrades,
        );

        // Spawn-only bookkeeping (not needed for save-load)
        let job = Job::from_i32(msg.job);
        faction_stats.inc_alive(msg.faction);
        if job == Job::Miner { dirty.mining = true; }
        if crate::constants::npc_def(job).is_military { dirty.squads = true; }

        if game_time.total_hours() > 0 {
            let job_str = crate::job_name(msg.job);
            combat_log.push(CombatEventKind::Spawn, msg.faction,
                game_time.day(), game_time.hour(), game_time.minute(),
                format!("{} #{} spawned", job_str, msg.slot_idx));
        }
    }

}

/// Build sorted patrol route from WorldData for a given town.
pub(crate) fn build_patrol_route(world: &WorldData, town_idx: u32) -> Vec<Vec2> {
    let mut posts: Vec<(u32, Vec2)> = world.waypoints().iter()
        .filter(|p| p.town_idx == town_idx && crate::world::is_alive(p.position))
        .map(|p| (p.patrol_order, p.position))
        .collect();
    posts.sort_by_key(|(order, _)| *order);
    posts.into_iter().map(|(_, pos)| pos).collect()
}
