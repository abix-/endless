//! Custom BRP endpoints for live game control via HTTP JSON-RPC.
//! Handlers push actions into queue resources; drain systems execute them
//! with proper SystemParams on the next FixedUpdate tick.

use bevy::prelude::*;
use bevy::remote::{BrpError, BrpResult, builtin_methods::parse_some, error_codes};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;

use crate::components::{
    Activity, CachedStats, CarriedLoot, CombatState, Energy, Faction, GpuSlot, Health, Home, Job,
    ManualTarget, NpcEquipment, NpcFlags, NpcWorkState, Personality, PatrolRoute, Speed, SquadId,
    TownId,
};
use crate::resources::SquadOwner;
use crate::constants::building_cost;
use crate::messages::{CombatLogMsg, GpuUpdateMsg};
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::world::{self, BuildingKind, WorldData};

fn queue_llm_log(world: &mut World, town: usize, message: String, location: Option<Vec2>) {
    let gt = world.resource::<GameTime>();
    let (day, hour, minute) = (gt.day(), gt.hour(), gt.minute());
    let wd = world.resource::<WorldData>();
    let name = wd.towns.get(town).map(|t| t.name.as_str()).unwrap_or("?");
    let faction = wd.towns.get(town).map(|t| t.faction).unwrap_or(-1);
    let msg = format!("[{name}] {message}");
    world.resource_mut::<RemoteLlmLogQueue>().0.push(CombatLogMsg {
        kind: CombatEventKind::Llm,
        faction,
        day,
        hour,
        minute,
        message: msg,
        location,
    });
}

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
pub struct RemoteDestroyQueue(pub Vec<RemoteDestroy>);

pub struct RemoteDestroy {
    pub town: usize,
    pub row: i32,
    pub col: i32,
}

#[derive(Resource, Default)]
pub struct RemoteUpgradeQueue(pub Vec<RemoteUpgrade>);

pub struct RemoteUpgrade {
    pub town: usize,
    pub upgrade_idx: usize,
}

#[derive(Resource, Default)]
pub struct RemoteLlmLogQueue(pub Vec<CombatLogMsg>);

/// Ring buffer of recent combat events for LLM summary (last 20).
#[derive(Resource, Default)]
pub struct RemoteCombatLogRing {
    pub events: std::collections::VecDeque<CombatLogMsg>,
}

impl RemoteCombatLogRing {
    const CAP: usize = 20;

    pub fn push(&mut self, msg: CombatLogMsg) {
        if self.events.len() >= Self::CAP {
            self.events.pop_front();
        }
        self.events.push_back(msg);
    }
}

// ============================================================================
// HELPERS
// ============================================================================

/// Wrap any serde_json::Value as a TOON string for BRP response.
fn toon_ok(value: Value) -> BrpResult {
    let toon = serde_toon2::to_string(&value).unwrap_or_default();
    Ok(json!(toon))
}

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

pub fn parse_building_kind(s: &str) -> Option<BuildingKind> {
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
        "Road" | "DirtRoad" => Some(BuildingKind::Road),
        "StoneRoad" => Some(BuildingKind::StoneRoad),
        "MetalRoad" => Some(BuildingKind::MetalRoad),
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

#[derive(Serialize)]
struct SummaryResponse {
    day: i32,
    hour: i32,
    minute: i32,
    paused: bool,
    time_scale: f32,
    town_idx: usize,
    town_name: String,
    faction: i32,
    food: i32,
    gold: i32,
    factions: Vec<(usize, i32, i32, i32)>,
    buildings: Vec<(String, i32, i32)>,
    squads: Vec<(usize, usize, Option<i32>, Option<i32>)>,
    upgrades: Vec<(usize, String, u8, String, String)>,
    combat_log: Vec<(i32, i32, i32, String)>,
    inbox: Vec<(usize, String, i32, i32, i32)>,
    npcs: BTreeMap<String, String>,
}

/// Format cost slice as compact string like "50g" or "30f+10g"
fn format_cost(cost: &[(crate::constants::ResourceKind, i32)]) -> String {
    cost.iter()
        .map(|(kind, amt)| {
            let suffix = match kind {
                crate::constants::ResourceKind::Food => "f",
                crate::constants::ResourceKind::Gold => "g",
            };
            format!("{amt}{suffix}")
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Collapse NPC counts like t1_Archer_Patrolling into "Archer: 8 (Patrolling:5 Idle:3)"
fn compact_npc_counts(raw: &BTreeMap<String, usize>, town_prefix: &str) -> BTreeMap<String, String> {
    let mut job_totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut job_activities: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();

    for (key, &count) in raw {
        let Some(rest) = key.strip_prefix(town_prefix) else { continue };
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        let job = parts[0].to_string();
        if parts.len() == 1 {
            // Job total (e.g. t1_Archer)
            job_totals.insert(job, count);
        } else {
            // Activity (e.g. t1_Archer_Patrolling)
            job_activities.entry(job).or_default().push((parts[1].to_string(), count));
        }
    }

    let mut result = BTreeMap::new();
    for (job, total) in &job_totals {
        let mut s = format!("{total}");
        if let Some(activities) = job_activities.get(job) {
            let parts: Vec<String> = activities.iter().map(|(a, c)| format!("{a}:{c}")).collect();
            s.push_str(&format!(" ({})", parts.join(" ")));
        }
        result.insert(job.clone(), s);
    }
    result
}

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

    // Drain chat inbox before immutable borrows
    let mut inbox_by_town: std::collections::HashMap<usize, Vec<(usize, String, i32, i32, i32)>> = std::collections::HashMap::new();
    {
        let mut inbox = world.resource_mut::<ChatInbox>();
        let messages = std::mem::take(&mut inbox.messages);
        for msg in messages {
            let dominated_by_filter = filter_town.is_some_and(|ft| ft != msg.to_town);
            if dominated_by_filter {
                inbox.messages.push(msg);
            } else {
                inbox_by_town.entry(msg.to_town).or_default().push(
                    (msg.from_town, msg.text, msg.day, msg.hour, msg.minute)
                );
            }
        }
    }

    // Now borrow resources immutably
    let game_time = world.resource::<GameTime>();
    let day = game_time.day();
    let hour = game_time.hour();
    let minute = game_time.minute();
    let paused = game_time.paused;
    let time_scale = game_time.time_scale;

    let food = world.resource::<FoodStorage>();
    let gold = world.resource::<GoldStorage>();
    let faction_stats = world.resource::<FactionStats>();
    let entity_map = world.resource::<EntityMap>();
    let world_data = world.resource::<WorldData>();
    let allowed = world.resource::<RemoteAllowedTowns>();
    let squad_state = world.resource::<SquadState>();
    let town_upgrades = world.resource::<crate::systems::stats::TownUpgrades>();
    let log_ring = world.resource::<RemoteCombatLogRing>();

    let factions: Vec<(usize, i32, i32, i32)> = faction_stats
        .stats
        .iter()
        .enumerate()
        .map(|(i, s)| (i, s.alive, s.dead, s.kills))
        .collect();

    // Find the LLM town (first allowed, or filter_town, or first town)
    let target_town = filter_town
        .or_else(|| allowed.towns.first().copied())
        .unwrap_or(0);

    let town = world_data.towns.get(target_town);
    let town_name = town.map(|t| t.name.clone()).unwrap_or_default();
    let town_faction = town.map(|t| t.faction).unwrap_or(0);
    let town_food = food.food.get(target_town).copied().unwrap_or(0);
    let town_gold = gold.gold.get(target_town).copied().unwrap_or(0);

    // Buildings
    let buildings: Vec<(String, i32, i32)> = if let Some(t) = town {
        entity_map.iter_instances()
            .filter(|inst| inst.town_idx as usize == target_town)
            .map(|inst| {
                let label = crate::constants::building_def(inst.kind).label.to_string();
                let (row, col) = world::world_to_town_grid(t.center, inst.position);
                (label, row, col)
            })
            .collect()
    } else {
        Vec::new()
    };

    // Squads
    let squads: Vec<(usize, usize, Option<i32>, Option<i32>)> = squad_state.squads.iter().enumerate()
        .filter(|(_, squad)| {
            let squad_town = match squad.owner {
                SquadOwner::Player => 0,
                SquadOwner::Town(tdi) => tdi,
            };
            squad_town == target_town
        })
        .map(|(si, squad)| {
            let (tx, ty) = match squad.target {
                Some(t) => (Some(t.x as i32), Some(t.y as i32)),
                None => (None, None),
            };
            (si, squad.members.len(), tx, ty)
        })
        .collect();

    // Upgrades
    let levels = town_upgrades.town_levels(target_town);
    let upgrade_nodes = &crate::systems::stats::UPGRADES.nodes;
    let upgrades: Vec<(usize, String, u8, String, String)> = upgrade_nodes.iter().enumerate()
        .map(|(i, node)| {
            let level = levels.get(i).copied().unwrap_or(0);
            let pct = format!("{:.0}%", node.pct * 100.0);
            let cost = format_cost(node.cost);
            (i, node.label.to_string(), level, pct, cost)
        })
        .collect();

    // Combat log — filter to events relevant to this town's faction
    let combat_log: Vec<(i32, i32, i32, String)> = log_ring.events.iter()
        .filter(|e| e.faction == town_faction || e.kind == CombatEventKind::Llm)
        .map(|e| (e.day, e.hour, e.minute, e.message.clone()))
        .collect();

    // Inbox
    let inbox = inbox_by_town.remove(&target_town).unwrap_or_default();

    // Compact NPC counts
    let town_prefix = format!("t{}_", target_town);
    let npcs = compact_npc_counts(&npc_counts, &town_prefix);

    let response = SummaryResponse {
        day, hour, minute, paused, time_scale,
        town_idx: target_town,
        town_name,
        faction: town_faction,
        food: town_food,
        gold: town_gold,
        factions,
        buildings,
        squads,
        upgrades,
        combat_log,
        inbox,
        npcs,
    };

    let toon = serde_toon2::to_string(&response).unwrap_or_default();
    Ok(json!(toon))
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

    let center = world.resource::<WorldData>().towns.get(p.town).map(|t| t.center).unwrap_or_default();
    let pos = world::town_grid_to_world(center, p.row, p.col);
    queue_llm_log(world, p.town, format!("build {} at ({},{})", p.kind, p.row, p.col), Some(pos));

    world
        .resource_mut::<RemoteBuildQueue>()
        .0
        .push(RemoteBuild {
            town: p.town,
            kind,
            row: p.row,
            col: p.col,
        });

    toon_ok(json!({"status": "queued", "kind": p.kind, "town": p.town, "row": p.row, "col": p.col}))
}

// --- endless/destroy --------------------------------------------------------

#[derive(Deserialize)]
struct DestroyParams {
    town: usize,
    row: i32,
    col: i32,
}

pub fn destroy_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: DestroyParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;

    let town_count = world.resource::<WorldData>().towns.len();
    if p.town >= town_count {
        return Err(brp_err(format!("town {} out of range", p.town)));
    }

    let center = world.resource::<WorldData>().towns.get(p.town).map(|t| t.center).unwrap_or_default();
    let pos = world::town_grid_to_world(center, p.row, p.col);
    queue_llm_log(world, p.town, format!("destroy at ({},{})", p.row, p.col), Some(pos));

    world
        .resource_mut::<RemoteDestroyQueue>()
        .0
        .push(RemoteDestroy {
            town: p.town,
            row: p.row,
            col: p.col,
        });

    toon_ok(json!({"status": "queued", "town": p.town, "row": p.row, "col": p.col}))
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

    let upgrade_label = crate::systems::stats::UPGRADES
        .nodes
        .get(p.upgrade_idx)
        .map(|n| n.label)
        .unwrap_or("unknown");
    queue_llm_log(world, p.town, format!("upgrade: {}", upgrade_label), None);

    world
        .resource_mut::<RemoteUpgradeQueue>()
        .0
        .push(RemoteUpgrade {
            town: p.town,
            upgrade_idx: p.upgrade_idx,
        });

    toon_ok(json!({"status": "queued", "town": p.town, "upgrade_idx": p.upgrade_idx}))
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

    // Diff: only log fields that actually change
    let mut parts = Vec::new();
    if let Some(v) = p.eat_food { if v != policy.eat_food { parts.push(format!("eat_food={v}")); } policy.eat_food = v; }
    if let Some(v) = p.archer_aggressive { if v != policy.archer_aggressive { parts.push(format!("archer_aggressive={v}")); } policy.archer_aggressive = v; }
    if let Some(v) = p.archer_leash { if v != policy.archer_leash { parts.push(format!("archer_leash={v}")); } policy.archer_leash = v; }
    if let Some(v) = p.farmer_fight_back { if v != policy.farmer_fight_back { parts.push(format!("farmer_fight_back={v}")); } policy.farmer_fight_back = v; }
    if let Some(v) = p.prioritize_healing { if v != policy.prioritize_healing { parts.push(format!("prioritize_healing={v}")); } policy.prioritize_healing = v; }
    if let Some(v) = p.farmer_flee_hp { let v = v.clamp(0.0, 1.0); if (v - policy.farmer_flee_hp).abs() > f32::EPSILON { parts.push(format!("farmer_flee_hp={v:.1}")); } policy.farmer_flee_hp = v; }
    if let Some(v) = p.archer_flee_hp { let v = v.clamp(0.0, 1.0); if (v - policy.archer_flee_hp).abs() > f32::EPSILON { parts.push(format!("archer_flee_hp={v:.1}")); } policy.archer_flee_hp = v; }
    if let Some(v) = p.recovery_hp { let v = v.clamp(0.0, 1.0); if (v - policy.recovery_hp).abs() > f32::EPSILON { parts.push(format!("recovery_hp={v:.1}")); } policy.recovery_hp = v; }
    if let Some(v) = p.mining_radius { let v = v.clamp(0.0, 5000.0); if (v - policy.mining_radius).abs() > f32::EPSILON { parts.push(format!("mining_radius={v:.0}")); } policy.mining_radius = v; }
    drop(policies);

    if !parts.is_empty() {
        queue_llm_log(world, p.town, format!("policy: {}", parts.join(", ")), None);
    }

    toon_ok(json!({"status": "ok", "town": p.town}))
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

    toon_ok(json!({
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
    let town;
    {
        let state = world.resource::<SquadState>();
        let squad = state.squads.get(p.squad).ok_or_else(|| brp_err(format!("squad {} out of range", p.squad)))?;
        town = match squad.owner {
            SquadOwner::Player => 0,
            SquadOwner::Town(tdi) => tdi,
        };
        check_town_allowed(world, town)?;
    }

    queue_llm_log(world, town, format!("squad {} target ({:.0},{:.0})", p.squad, p.x, p.y), Some(Vec2::new(p.x, p.y)));

    let mut state = world.resource_mut::<SquadState>();
    let squad = state.squads.get_mut(p.squad).ok_or_else(|| brp_err(format!("squad {} out of range", p.squad)))?;
    squad.target = Some(Vec2::new(p.x, p.y));

    toon_ok(json!({"status": "ok", "squad": p.squad, "target_x": p.x, "target_y": p.y}))
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

    let mut parts = Vec::new();
    if let Some(v) = p.active { parts.push(format!("active={v}")); }
    if let Some(ref s) = p.personality { parts.push(format!("personality={s}")); }
    if let Some(v) = p.build_enabled { parts.push(format!("build={v}")); }
    if let Some(v) = p.upgrade_enabled { parts.push(format!("upgrade={v}")); }
    if let Some(ref s) = p.road_style { parts.push(format!("roads={s}")); }
    let msg = if parts.is_empty() { "ai_manager query".to_string() } else { format!("ai_manager: {}", parts.join(", ")) };
    queue_llm_log(world, p.town, msg, None);

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

    toon_ok(json!({
        "status": "ok",
        "town": p.town,
        "active": player.active,
        "build_enabled": player.build_enabled,
        "upgrade_enabled": player.upgrade_enabled,
        "personality": player.personality.name(),
        "road_style": format!("{:?}", player.road_style),
    }))
}

// --- endless/chat ------------------------------------------------------------

#[derive(Deserialize)]
struct ChatParams {
    town: usize,
    to: usize,
    message: String,
}

pub fn chat_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: ChatParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;

    let gt = world.resource::<GameTime>();
    let (day, hour, minute) = (gt.day(), gt.hour(), gt.minute());

    let from_name = world.resource::<WorldData>().towns.get(p.town).map(|t| t.name.clone()).unwrap_or_default();
    queue_llm_log(world, p.town, format!("[chat to F{}] {}", p.to, p.message), None);

    world.resource_mut::<ChatInbox>().messages.push(ChatMessage {
        from_town: p.town,
        to_town: p.to,
        text: p.message.clone(),
        day, hour, minute,
    });

    toon_ok(json!({"status": "ok", "from": from_name, "message": p.message}))
}

// --- endless/debug -----------------------------------------------------------

#[derive(Deserialize)]
struct DebugParams {
    kind: String,
    slot: usize,
}

pub fn debug_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: DebugParams = parse_some(params)?;
    match p.kind.as_str() {
        "npc" => debug_npc(world, p.slot),
        "building" => debug_building(world, p.slot),
        _ => Err(brp_err(format!("unknown debug kind: {} (use 'npc' or 'building')", p.kind))),
    }
}

fn debug_npc(world: &mut World, slot: usize) -> BrpResult {
    // ECS query — split into nested tuples to stay under Bevy's 15-element QueryData limit
    let mut query = world.query::<(
        &GpuSlot, &Job, &Activity, &Health, &Energy, &Speed, &Home, &TownId,
        &Faction, &CarriedLoot, &NpcWorkState, &NpcFlags, &CombatState,
        (&Personality, &CachedStats, &NpcEquipment,
         Option<&ManualTarget>, Option<&SquadId>, Option<&PatrolRoute>),
    )>();

    let mut npc_data: Option<Value> = None;
    for (
        gpu_slot, job, activity, health, energy, _speed, home, town_id,
        faction, carried_loot, work_state, flags, combat_state,
        (personality, stats, equipment, manual_target, squad_id, patrol_route),
    ) in query.iter(world) {
        if gpu_slot.0 != slot { continue; }

        // Equipment slots
        let mut equip = json!({});
        let slots: &[(&str, &Option<crate::constants::LootItem>)] = &[
            ("weapon", &equipment.weapon), ("helm", &equipment.helm),
            ("armor", &equipment.armor), ("shield", &equipment.shield),
            ("gloves", &equipment.gloves), ("boots", &equipment.boots),
            ("belt", &equipment.belt), ("amulet", &equipment.amulet),
            ("ring1", &equipment.ring1), ("ring2", &equipment.ring2),
        ];
        for &(label, item_opt) in slots {
            if let Some(item) = item_opt {
                equip[label] = json!({
                    "name": item.name,
                    "rarity": item.rarity.label(),
                    "stat_bonus": format!("{:.0}%", item.stat_bonus * 100.0),
                });
            }
        }

        // Flags
        let mut flag_list = Vec::new();
        if flags.healing { flag_list.push("healing"); }
        if flags.starving { flag_list.push("starving"); }
        if flags.direct_control { flag_list.push("direct_control"); }
        if flags.migrating { flag_list.push("migrating"); }
        if flags.at_destination { flag_list.push("at_dest"); }

        // ManualTarget
        let mt_str = match manual_target {
            Some(ManualTarget::Npc(s)) => format!("Npc({})", s),
            Some(ManualTarget::Building(pos)) => format!("Building({:.0},{:.0})", pos.x, pos.y),
            Some(ManualTarget::Position(pos)) => format!("Position({:.0},{:.0})", pos.x, pos.y),
            None => "None".to_string(),
        };

        // Personality traits
        let trait_str = personality.trait_summary();

        npc_data = Some(json!({
            "slot": slot,
            "job": format!("{:?}", job),
            "activity": activity.name(),
            "activity_debug": format!("{:?}", activity),
            "combat_state": combat_state.name(),
            "hp": health.0,
            "max_hp": stats.max_health,
            "energy": energy.0,
            "speed": stats.speed,
            "home": [home.0.x, home.0.y],
            "town_id": town_id.0,
            "faction": faction.0,
            "carried_loot": {
                "food": carried_loot.food,
                "gold": carried_loot.gold,
                "equipment": carried_loot.equipment.iter().map(|i| json!({
                    "name": &i.name, "rarity": i.rarity.label(),
                })).collect::<Vec<_>>(),
            },
            "work_state": { "worksite": work_state.worksite.map(|u| u.0) },
            "flags": flag_list,
            "manual_target": mt_str,
            "squad_id": squad_id.map(|s| s.0),
            "patrol": patrol_route.map(|r| json!({
                "current": r.current, "posts": r.posts.len(),
            })),
            "personality": trait_str,
            "stats": {
                "damage": stats.damage,
                "range": stats.range,
                "cooldown": stats.cooldown,
                "hp_regen": stats.hp_regen,
                "stamina": stats.stamina,
                "berserk_bonus": stats.berserk_bonus,
            },
            "equipment": equip,
        }));
        break;
    }

    let Some(mut data) = npc_data else {
        return Err(brp_err(format!("no NPC at slot {}", slot)));
    };

    // Resource-based data (immutable borrows after query)
    let entity_map = world.resource::<EntityMap>();
    let meta_cache = world.resource::<NpcMetaCache>();
    let npc_logs = world.resource::<NpcLogCache>();
    let thrash = world.resource::<NpcTargetThrashDebug>();
    let gpu_state = world.resource::<GpuReadState>();
    let world_data = world.resource::<WorldData>();
    let policies = world.resource::<TownPolicies>();
    let squad_state = world.resource::<SquadState>();
    let game_time = world.resource::<GameTime>();

    // NpcMeta (name, level, xp)
    if let Some(meta) = meta_cache.0.get(slot) {
        let xp_next = (meta.level + 1) * (meta.level + 1) * 100;
        data["name"] = json!(meta.name);
        data["level"] = json!(meta.level);
        data["xp"] = json!(meta.xp);
        data["xp_next"] = json!(xp_next);
    }

    // Town name + faction name
    let town_id = data["town_id"].as_i64().unwrap_or(-1) as i32;
    if town_id >= 0 {
        if let Some(town) = world_data.towns.get(town_id as usize) {
            data["town_name"] = json!(town.name);
            data["faction_name"] = json!(format!("{} (F{})", town.name, town.faction));
        }
        if let Some(p) = policies.policies.get(town_id as usize) {
            data["policy"] = json!({
                "eat_food": p.eat_food,
                "archer_aggressive": p.archer_aggressive,
                "archer_leash": p.archer_leash,
                "archer_flee_hp": p.archer_flee_hp,
                "farmer_flee_hp": p.farmer_flee_hp,
                "prioritize_healing": p.prioritize_healing,
                "recovery_hp": p.recovery_hp,
            });
        }
    }

    // GPU readback: position, target, combat_target
    let i2 = slot * 2;
    if i2 + 1 < gpu_state.positions.len() {
        data["gpu_pos"] = json!([gpu_state.positions[i2], gpu_state.positions[i2 + 1]]);
    }
    // GPU target from EntityGpuState (CPU-side target buffer)
    let gpu_data = world.resource::<crate::gpu::EntityGpuState>();
    if i2 + 1 < gpu_data.targets.len() {
        data["gpu_target"] = json!([gpu_data.targets[i2], gpu_data.targets[i2 + 1]]);
    }
    let ct = gpu_state.combat_targets.get(slot).copied().unwrap_or(-1);
    data["gpu_combat_target"] = json!(ct);
    if ct >= 0 {
        let ti = ct as usize;
        if let Some(inst) = entity_map.get_instance(ti) {
            data["gpu_target_resolved"] = json!({
                "type": "building",
                "kind": format!("{:?}", inst.kind),
                "faction": inst.faction,
                "pos": [inst.position.x, inst.position.y],
            });
        } else if let Some(tnpc) = entity_map.get_npc(ti) {
            let tx = gpu_state.positions.get(ti * 2).copied().unwrap_or(-9999.0);
            let ty = gpu_state.positions.get(ti * 2 + 1).copied().unwrap_or(-9999.0);
            data["gpu_target_resolved"] = json!({
                "type": "npc",
                "slot": ti,
                "faction": tnpc.faction,
                "pos": [tx, ty],
                "dead": tnpc.dead,
            });
        }
    }

    // Worksite detail
    if let Some(uid_val) = data["work_state"]["worksite"].as_u64() {
        let uid = crate::components::EntityUid(uid_val);
        if let Some(ws_slot) = entity_map.slot_for_uid(uid) {
            if let Some(inst) = entity_map.get_instance(ws_slot) {
                let max_occ = crate::constants::building_def(inst.kind)
                    .worksite.map_or(0, |w| w.max_occupants);
                data["worksite_detail"] = json!({
                    "slot": ws_slot,
                    "kind": format!("{:?}", inst.kind),
                    "occupants": inst.occupants,
                    "max_occupants": max_occ,
                    "growth": format!("{:.0}%", inst.growth_progress * 100.0),
                    "pos": [inst.position.x, inst.position.y],
                });
            }
        }
    }

    // Squad detail
    if let Some(sq_val) = data["squad_id"].as_i64() {
        let sq = sq_val as usize;
        if sq < squad_state.squads.len() {
            let s = &squad_state.squads[sq];
            data["squad"] = json!({
                "id": sq,
                "members": s.members.len(),
                "target": s.target.map(|v| json!([v.x, v.y])),
                "hold_fire": s.hold_fire,
                "patrol_enabled": s.patrol_enabled,
                "rest_when_tired": s.rest_when_tired,
            });
        }
    }

    // Target thrash diagnostics
    let sink_changes = thrash.sink_target_changes_this_minute.get(slot).copied().unwrap_or(0);
    let sink_ping_pong = thrash.sink_ping_pong_this_minute.get(slot).copied().unwrap_or(0);
    let sink_writes = thrash.sink_writes_this_minute.get(slot).copied().unwrap_or(0);
    let reason_flips = thrash.reason_flips_this_minute.get(slot).copied().unwrap_or(0);
    let last_reason = thrash.last_reason.get(slot).map(String::as_str).unwrap_or("-");
    let prev_target = thrash.sink_prev_target.get(slot).copied().unwrap_or((0.0, 0.0));
    let last_target = thrash.sink_last_target.get(slot).copied().unwrap_or((0.0, 0.0));
    data["thrash"] = json!({
        "sink_changes": sink_changes,
        "sink_ping_pong": sink_ping_pong,
        "sink_writes": sink_writes,
        "reason_flips": reason_flips,
        "last_reason": last_reason,
        "prev_target": [prev_target.0, prev_target.1],
        "last_target": [last_target.0, last_target.1],
    });

    // NPC activity log (last 20 entries)
    if slot < npc_logs.logs.len() {
        let entries: Vec<Value> = npc_logs.logs[slot].iter().rev().take(20).map(|e| {
            json!(format!("D{}:{:02}:{:02} {}", e.day, e.hour, e.minute, e.message))
        }).collect();
        data["log"] = json!(entries);
    }

    // Timestamp
    data["day"] = json!(game_time.day());
    data["hour"] = json!(game_time.hour());
    data["minute"] = json!(game_time.minute());

    toon_ok(data)
}

fn debug_building(world: &mut World, slot: usize) -> BrpResult {
    let entity_map = world.resource::<EntityMap>();
    let gpu_state = world.resource::<GpuReadState>();
    let world_data = world.resource::<WorldData>();
    let game_time = world.resource::<GameTime>();

    let inst = entity_map.get_instance(slot)
        .ok_or_else(|| brp_err(format!("no building at slot {}", slot)))?
        .clone();

    let def = crate::constants::building_def(inst.kind);
    let town_name = world_data.towns.get(inst.town_idx as usize)
        .map(|t| t.name.as_str()).unwrap_or("?");
    let faction_str = world_data.towns.get(inst.town_idx as usize)
        .map(|t| format!("{} (F{})", t.name, t.faction))
        .unwrap_or_else(|| if inst.kind == BuildingKind::GoldMine { "Unowned".into() } else { "?".into() });

    // Grid coords
    let (row, col) = world_data.towns.get(inst.town_idx as usize)
        .map(|t| world::world_to_town_grid(t.center, inst.position))
        .unwrap_or((0, 0));

    // HP from entity
    let hp = entity_map.entities.get(&slot)
        .and_then(|&e| world.get::<Health>(e))
        .map(|h| h.0)
        .unwrap_or(0.0);

    // GPU position
    let i2 = slot * 2;
    let gpu_pos = if i2 + 1 < gpu_state.positions.len() {
        Some([gpu_state.positions[i2], gpu_state.positions[i2 + 1]])
    } else {
        None
    };

    let mut data = json!({
        "slot": slot,
        "kind": format!("{:?}", inst.kind),
        "label": def.label,
        "town": town_name,
        "faction": faction_str,
        "pos": [inst.position.x, inst.position.y],
        "grid": [row, col],
        "hp": hp,
        "max_hp": def.hp,
        "town_idx": inst.town_idx,
        "occupants": inst.occupants,
        "growth": format!("{:.0}%", inst.growth_progress * 100.0),
        "under_construction": inst.under_construction,
        "npc_uid": inst.npc_uid.map(|u| u.0),
        "respawn_timer": inst.respawn_timer,
    });

    if let Some(gp) = gpu_pos {
        data["gpu_pos"] = json!(gp);
    }

    // Worksite info
    if let Some(ws) = def.worksite {
        data["worksite"] = json!({
            "max_occupants": ws.max_occupants,
            "drift_radius": ws.drift_radius,
            "harvest_item": format!("{:?}", ws.harvest_item),
            "town_scoped": ws.town_scoped,
        });
    }

    // Miner home extras
    if inst.kind == BuildingKind::MinerHome {
        data["assigned_mine"] = json!(inst.assigned_mine.map(|v| [v.x, v.y]));
        data["manual_mine"] = json!(inst.manual_mine);
    }

    // Wall level
    if inst.kind == BuildingKind::Wall {
        data["wall_level"] = json!(inst.wall_level);
    }

    data["day"] = json!(game_time.day());
    data["hour"] = json!(game_time.hour());
    data["minute"] = json!(game_time.minute());

    toon_ok(data)
}

// ============================================================================
// DRAIN SYSTEM
// ============================================================================

pub fn drain_remote_queues(
    mut build_q: ResMut<RemoteBuildQueue>,
    mut destroy_q: ResMut<RemoteDestroyQueue>,
    mut upgrade_q: ResMut<RemoteUpgradeQueue>,
    mut llm_log_q: ResMut<RemoteLlmLogQueue>,
    mut log_ring: ResMut<RemoteCombatLogRing>,
    mut world_state: WorldState,
    mut food_storage: ResMut<FoodStorage>,
    mut gpu_updates: MessageWriter<GpuUpdateMsg>,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    mut damage_writer: MessageWriter<crate::messages::DamageMsg>,
    mut commands: Commands,
    mut upgrade_writer: MessageWriter<crate::systems::stats::UpgradeMsg>,
    game_time: Res<GameTime>,
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

    // Drain destroy queue
    let destroys: Vec<RemoteDestroy> = destroy_q.0.drain(..).collect();
    for destroy in destroys {
        let Some(town) = world_state.world_data.towns.get(destroy.town) else {
            continue;
        };
        let center = town.center;
        let town_name = town.name.clone();
        let pos = world::town_grid_to_world(center, destroy.row, destroy.col);
        let (gc, gr) = world_state.grid.world_to_grid(pos);

        // Look up building at grid cell
        let Some(inst) = world_state.entity_map.get_at_grid(gc as i32, gr as i32) else {
            continue;
        };
        // Validate: not Fountain/GoldMine, belongs to requesting town
        if matches!(inst.kind, BuildingKind::Fountain | BuildingKind::GoldMine) {
            continue;
        }
        if inst.town_idx as usize != destroy.town {
            continue;
        }
        let bld_kind = inst.kind;
        let slot = inst.slot;

        // Send lethal damage so death_system handles entity despawn
        let Some(uid) = world_state.entity_map.uid_for_slot(slot) else {
            continue;
        };
        damage_writer.write(crate::messages::DamageMsg {
            target: uid,
            amount: f32::MAX,
            attacker: -1,
            attacker_faction: 0,
        });

        let _ = world_state.destroy_building(
            &mut combat_log,
            &game_time,
            destroy.row,
            destroy.col,
            center,
            &format!("Destroyed building in {}", town_name),
            &mut gpu_updates,
        );
        world_state.dirty_writers.mark_building_changed(bld_kind);
    }

    // Drain upgrade queue
    for upgrade in upgrade_q.0.drain(..) {
        upgrade_writer.write(crate::systems::stats::UpgradeMsg {
            town_idx: upgrade.town,
            upgrade_idx: upgrade.upgrade_idx,
        });
    }

    // Drain LLM log queue — write to both combat log and ring buffer
    for msg in llm_log_q.0.drain(..) {
        log_ring.push(msg.clone());
        combat_log.write(msg);
    }
}
