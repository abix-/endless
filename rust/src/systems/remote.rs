//! Custom BRP endpoints for live game control via HTTP JSON-RPC.
//! Handlers push actions into queue resources; drain systems execute them
//! with proper SystemParams on the next FixedUpdate tick.

use bevy::prelude::*;
use bevy::remote::{BrpError, BrpResult, builtin_methods::parse_some, error_codes};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::components::{Activity, Job, TownId};
use crate::resources::SquadOwner;
use crate::constants::building_cost;
use crate::messages::GpuUpdateMsg;
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::world::{self, BuildingKind, WorldData};

const FORBIDDEN_CODE: i16 = -32001;

// ============================================================================
// QUEUE RESOURCES
// ============================================================================

#[derive(Resource, Default)]
pub struct RemoteBuildQueue(pub Vec<RemoteBuild>);

pub struct RemoteBuild {
    pub town: usize,
    pub kind: BuildingKind,
    pub row: i32,
    pub col: i32,
}

#[derive(Resource, Default)]
pub struct RemoteUpgradeQueue(pub Vec<RemoteUpgrade>);

pub struct RemoteUpgrade {
    pub town: usize,
    pub upgrade_idx: usize,
}

// ============================================================================
// HELPERS
// ============================================================================

fn brp_err(msg: impl Into<String>) -> BrpError {
    BrpError {
        code: error_codes::INVALID_PARAMS,
        message: msg.into(),
        data: None,
    }
}

fn check_town_allowed(world: &World, town: usize) -> Result<(), BrpError> {
    let allowed = world.resource::<RemoteAllowedTowns>();
    if allowed.towns.is_empty() || allowed.towns.contains(&town) {
        Ok(())
    } else {
        Err(BrpError {
            code: FORBIDDEN_CODE,
            message: format!("town {} is not LLM-controlled", town),
            data: None,
        })
    }
}

fn parse_building_kind(s: &str) -> Option<BuildingKind> {
    match s {
        "Fountain" => Some(BuildingKind::Fountain),
        "Bed" => Some(BuildingKind::Bed),
        "Waypoint" => Some(BuildingKind::Waypoint),
        "Farm" => Some(BuildingKind::Farm),
        "FarmerHome" => Some(BuildingKind::FarmerHome),
        "ArcherHome" => Some(BuildingKind::ArcherHome),
        "Tent" => Some(BuildingKind::Tent),
        "GoldMine" => Some(BuildingKind::GoldMine),
        "MinerHome" => Some(BuildingKind::MinerHome),
        "CrossbowHome" => Some(BuildingKind::CrossbowHome),
        "FighterHome" => Some(BuildingKind::FighterHome),
        "Road" => Some(BuildingKind::Road),
        "Wall" => Some(BuildingKind::Wall),
        "Tower" => Some(BuildingKind::Tower),
        "Merchant" => Some(BuildingKind::Merchant),
        "Casino" => Some(BuildingKind::Casino),
        _ => None,
    }
}

// ============================================================================
// HANDLERS
// ============================================================================

// --- endless/summary --------------------------------------------------------

#[derive(Deserialize)]
struct SummaryParams {
    town: Option<usize>,
}

pub fn summary_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let filter_town: Option<usize> = params
        .and_then(|v| serde_json::from_value::<SummaryParams>(v).ok())
        .and_then(|p| p.town);

    // NPC counts via ECS query (must happen first — needs &mut World)
    let mut npc_counts: BTreeMap<String, usize> = BTreeMap::new();
    let mut query = world.query::<(&Job, &TownId, &Activity)>();
    for (job, town_id, activity) in query.iter(world) {
        if let Some(ft) = filter_town {
            if town_id.0 as usize != ft {
                continue;
            }
        }
        let key = format!("t{}_{:?}", town_id.0, job);
        *npc_counts.entry(key).or_default() += 1;
        let act_key = format!("t{}_{:?}_{}", town_id.0, job, activity.name());
        *npc_counts.entry(act_key).or_default() += 1;
    }

    // Now borrow resources immutably
    let game_time = world.resource::<GameTime>();
    let time_json = json!({
        "day": game_time.day(),
        "hour": game_time.hour(),
        "minute": game_time.minute(),
        "paused": game_time.paused,
        "time_scale": game_time.time_scale,
        "total_seconds": game_time.total_seconds,
    });

    let food = world.resource::<FoodStorage>();
    let food_vec: Vec<i32> = food.food.clone();
    let gold = world.resource::<GoldStorage>();
    let gold_vec: Vec<i32> = gold.gold.clone();

    let faction_stats = world.resource::<FactionStats>();
    let fstats: Vec<Value> = faction_stats
        .stats
        .iter()
        .enumerate()
        .map(|(i, s)| {
            json!({"faction": i, "alive": s.alive, "dead": s.dead, "kills": s.kills})
        })
        .collect();

    let entity_map = world.resource::<EntityMap>();
    let world_data = world.resource::<WorldData>();

    let mut towns = Vec::new();
    for (ti, town) in world_data.towns.iter().enumerate() {
        if let Some(ft) = filter_town {
            if ti != ft {
                continue;
            }
        }

        let mut buildings: BTreeMap<&str, usize> = BTreeMap::new();
        for inst in entity_map.iter_instances() {
            if inst.town_idx as usize == ti {
                let label = crate::constants::building_def(inst.kind).label;
                *buildings.entry(label).or_default() += 1;
            }
        }

        towns.push(json!({
            "index": ti,
            "name": town.name,
            "faction": town.faction,
            "center": { "x": town.center.x, "y": town.center.y },
            "food": food_vec.get(ti).copied().unwrap_or(0),
            "gold": gold_vec.get(ti).copied().unwrap_or(0),
            "buildings": buildings,
        }));
    }

    Ok(json!({
        "game_time": time_json,
        "towns": towns,
        "npcs": npc_counts,
        "factions": fstats,
    }))
}

// --- endless/build ----------------------------------------------------------

#[derive(Deserialize)]
struct BuildParams {
    town: usize,
    kind: String,
    row: i32,
    col: i32,
}

pub fn build_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: BuildParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;
    let kind = parse_building_kind(&p.kind).ok_or_else(|| brp_err(format!("unknown building kind: {}", p.kind)))?;

    // Validate town exists
    let town_count = world.resource::<WorldData>().towns.len();
    if p.town >= town_count {
        return Err(brp_err(format!("town {} out of range (max {})", p.town, town_count - 1)));
    }

    world
        .resource_mut::<RemoteBuildQueue>()
        .0
        .push(RemoteBuild {
            town: p.town,
            kind,
            row: p.row,
            col: p.col,
        });

    Ok(json!({"status": "queued", "kind": p.kind, "town": p.town, "row": p.row, "col": p.col}))
}

// --- endless/upgrade --------------------------------------------------------

#[derive(Deserialize)]
struct UpgradeParams {
    town: usize,
    upgrade_idx: usize,
}

pub fn upgrade_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: UpgradeParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;

    let town_count = world.resource::<WorldData>().towns.len();
    if p.town >= town_count {
        return Err(brp_err(format!("town {} out of range", p.town)));
    }

    world
        .resource_mut::<RemoteUpgradeQueue>()
        .0
        .push(RemoteUpgrade {
            town: p.town,
            upgrade_idx: p.upgrade_idx,
        });

    Ok(json!({"status": "queued", "town": p.town, "upgrade_idx": p.upgrade_idx}))
}

// --- endless/policy ---------------------------------------------------------

#[derive(Deserialize)]
struct PolicyParams {
    town: usize,
    #[serde(default)]
    eat_food: Option<bool>,
    #[serde(default)]
    archer_aggressive: Option<bool>,
    #[serde(default)]
    archer_leash: Option<bool>,
    #[serde(default)]
    farmer_fight_back: Option<bool>,
    #[serde(default)]
    prioritize_healing: Option<bool>,
    #[serde(default)]
    farmer_flee_hp: Option<f32>,
    #[serde(default)]
    archer_flee_hp: Option<f32>,
    #[serde(default)]
    recovery_hp: Option<f32>,
    #[serde(default)]
    mining_radius: Option<f32>,
}

pub fn policy_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: PolicyParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;

    let mut policies = world.resource_mut::<TownPolicies>();
    let policy = policies.policies.get_mut(p.town).ok_or_else(|| brp_err(format!("town {} out of range", p.town)))?;

    if let Some(v) = p.eat_food { policy.eat_food = v; }
    if let Some(v) = p.archer_aggressive { policy.archer_aggressive = v; }
    if let Some(v) = p.archer_leash { policy.archer_leash = v; }
    if let Some(v) = p.farmer_fight_back { policy.farmer_fight_back = v; }
    if let Some(v) = p.prioritize_healing { policy.prioritize_healing = v; }
    if let Some(v) = p.farmer_flee_hp { policy.farmer_flee_hp = v; }
    if let Some(v) = p.archer_flee_hp { policy.archer_flee_hp = v; }
    if let Some(v) = p.recovery_hp { policy.recovery_hp = v; }
    if let Some(v) = p.mining_radius { policy.mining_radius = v; }

    Ok(json!({"status": "ok", "town": p.town}))
}

// --- endless/time -----------------------------------------------------------

#[derive(Deserialize)]
struct TimeParams {
    #[serde(default)]
    paused: Option<bool>,
    #[serde(default)]
    time_scale: Option<f32>,
}

pub fn time_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: TimeParams = parse_some(params)?;

    let mut gt = world.resource_mut::<GameTime>();
    if let Some(v) = p.paused {
        gt.paused = v;
    }
    if let Some(v) = p.time_scale {
        gt.time_scale = v.clamp(0.0, 20.0);
    }

    Ok(json!({
        "status": "ok",
        "paused": gt.paused,
        "time_scale": gt.time_scale,
    }))
}

// --- endless/squad_target ---------------------------------------------------

#[derive(Deserialize)]
struct SquadTargetParams {
    squad: usize,
    x: f32,
    y: f32,
}

pub fn squad_target_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: SquadTargetParams = parse_some(params)?;

    // Resolve squad owner to town index for access control
    {
        let state = world.resource::<SquadState>();
        let squad = state.squads.get(p.squad).ok_or_else(|| brp_err(format!("squad {} out of range", p.squad)))?;
        let town = match squad.owner {
            SquadOwner::Player => 0,
            SquadOwner::Town(tdi) => tdi,
        };
        check_town_allowed(world, town)?;
    }

    let mut state = world.resource_mut::<SquadState>();
    let squad = state.squads.get_mut(p.squad).ok_or_else(|| brp_err(format!("squad {} out of range", p.squad)))?;
    squad.target = Some(Vec2::new(p.x, p.y));

    Ok(json!({"status": "ok", "squad": p.squad, "target": {"x": p.x, "y": p.y}}))
}

// --- endless/ai_manager -----------------------------------------------------

#[derive(Deserialize)]
struct AiManagerParams {
    town: usize,
    #[serde(default)]
    active: Option<bool>,
    #[serde(default)]
    build_enabled: Option<bool>,
    #[serde(default)]
    upgrade_enabled: Option<bool>,
    #[serde(default)]
    personality: Option<String>,
    #[serde(default)]
    road_style: Option<String>,
}

fn parse_personality(s: &str) -> Option<crate::systems::AiPersonality> {
    match s {
        "Aggressive" => Some(crate::systems::AiPersonality::Aggressive),
        "Balanced" => Some(crate::systems::AiPersonality::Balanced),
        "Economic" => Some(crate::systems::AiPersonality::Economic),
        _ => None,
    }
}

fn parse_road_style(s: &str) -> Option<crate::systems::ai_player::RoadStyle> {
    match s {
        "None" => Some(crate::systems::ai_player::RoadStyle::None),
        "Cardinal" => Some(crate::systems::ai_player::RoadStyle::Cardinal),
        "Grid4" => Some(crate::systems::ai_player::RoadStyle::Grid4),
        "Grid5" => Some(crate::systems::ai_player::RoadStyle::Grid5),
        _ => None,
    }
}

pub fn ai_manager_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: AiManagerParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;

    let mut ai_state = world.resource_mut::<crate::systems::AiPlayerState>();
    let player = ai_state
        .players
        .iter_mut()
        .find(|pl| pl.town_data_idx == p.town)
        .ok_or_else(|| brp_err(format!("no AI player for town {}", p.town)))?;

    if let Some(v) = p.active { player.active = v; }
    if let Some(v) = p.build_enabled { player.build_enabled = v; }
    if let Some(v) = p.upgrade_enabled { player.upgrade_enabled = v; }
    if let Some(ref s) = p.personality {
        player.personality = parse_personality(s).ok_or_else(|| brp_err(format!("unknown personality: {s}")))?;
    }
    if let Some(ref s) = p.road_style {
        player.road_style = parse_road_style(s).ok_or_else(|| brp_err(format!("unknown road_style: {s}")))?;
    }

    Ok(json!({
        "status": "ok",
        "town": p.town,
        "active": player.active,
        "build_enabled": player.build_enabled,
        "upgrade_enabled": player.upgrade_enabled,
        "personality": player.personality.name(),
        "road_style": format!("{:?}", player.road_style),
    }))
}

// ============================================================================
// DRAIN SYSTEM
// ============================================================================

pub fn drain_remote_queues(
    mut build_q: ResMut<RemoteBuildQueue>,
    mut upgrade_q: ResMut<RemoteUpgradeQueue>,
    mut world_state: WorldState,
    mut food_storage: ResMut<FoodStorage>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut commands: Commands,
    mut upgrade_writer: MessageWriter<crate::systems::stats::UpgradeMsg>,
) {
    // Drain build queue
    let builds: Vec<RemoteBuild> = build_q.0.drain(..).collect();
    for build in builds {
        let Some(town) = world_state.world_data.towns.get(build.town) else {
            continue;
        };
        let center = town.center;
        let pos = world::town_grid_to_world(center, build.row, build.col);
        let cost = building_cost(build.kind);

        let _ = world_state.place_building(
            &mut food_storage,
            build.kind,
            build.town,
            pos,
            cost,
            &mut gpu_updates,
            &mut commands,
        );
    }

    // Drain upgrade queue
    for upgrade in upgrade_q.0.drain(..) {
        upgrade_writer.write(crate::systems::stats::UpgradeMsg {
            town_idx: upgrade.town,
            upgrade_idx: upgrade.upgrade_idx,
        });
    }
}
