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
    ManualTarget, NpcEquipment, NpcFlags, NpcWorkState, PatrolRoute, Personality, Speed, SquadId,
    TownId,
};
use crate::constants::building_cost;
use crate::messages::{CombatLogMsg, GpuUpdateMsg};
use crate::resources::SquadOwner;
use crate::resources::*;
use crate::systemparams::WorldState;
use crate::world::{BuildingKind, WorldData};

fn queue_llm_log(world: &mut World, town: usize, message: String, location: Option<Vec2>) {
    let gt = world.resource::<GameTime>();
    let (day, hour, minute) = (gt.day(), gt.hour(), gt.minute());
    let wd = world.resource::<WorldData>();
    let name = wd.towns.get(town).map(|t| t.name.as_str()).unwrap_or("?");
    let faction = wd.towns.get(town).map(|t| t.faction).unwrap_or(-1);
    let msg = format!("[{name}] {message}");
    world
        .resource_mut::<RemoteLlmLogQueue>()
        .0
        .push(CombatLogMsg {
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
    pub col: usize,
    pub row: usize,
}

#[derive(Resource, Default)]
pub struct RemoteDestroyQueue(pub Vec<RemoteDestroy>);

pub struct RemoteDestroy {
    pub town: usize,
    pub col: usize,
    pub row: usize,
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

/// Round f32 to 2 decimal places via f64 to avoid precision artifacts.
fn r2(v: f32) -> f64 {
    (v as f64 * 100.0).round() / 100.0
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
    buildings: Vec<(String, usize, usize)>,
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
                crate::constants::ResourceKind::Wood => "w",
                crate::constants::ResourceKind::Stone => "s",
            };
            format!("{amt}{suffix}")
        })
        .collect::<Vec<_>>()
        .join("+")
}

/// Collapse NPC counts like t1_Archer_Patrolling into "Archer: 8 (Patrolling:5 Idle:3)"
fn compact_npc_counts(
    raw: &BTreeMap<String, usize>,
    town_prefix: &str,
) -> BTreeMap<String, String> {
    let mut job_totals: BTreeMap<String, usize> = BTreeMap::new();
    let mut job_activities: BTreeMap<String, Vec<(String, usize)>> = BTreeMap::new();

    for (key, &count) in raw {
        let Some(rest) = key.strip_prefix(town_prefix) else {
            continue;
        };
        let parts: Vec<&str> = rest.splitn(2, '_').collect();
        let job = parts[0].to_string();
        if parts.len() == 1 {
            // Job total (e.g. t1_Archer)
            job_totals.insert(job, count);
        } else {
            // Activity (e.g. t1_Archer_Patrolling)
            job_activities
                .entry(job)
                .or_default()
                .push((parts[1].to_string(), count));
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

    // Read chat inbox without draining — messages must persist for HUD display
    let mut inbox_by_town: std::collections::HashMap<usize, Vec<(usize, String, i32, i32, i32)>> =
        std::collections::HashMap::new();
    {
        let inbox = world.resource::<ChatInbox>();
        for msg in inbox.messages.iter() {
            let dominated_by_filter = filter_town.is_some_and(|ft| ft != msg.to_town);
            if !dominated_by_filter {
                inbox_by_town.entry(msg.to_town).or_default().push((
                    msg.from_town,
                    msg.text.clone(),
                    msg.day,
                    msg.hour,
                    msg.minute,
                ));
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

    let faction_stats = world.resource::<FactionStats>();
    let entity_map = world.resource::<EntityMap>();
    let grid = world.resource::<crate::world::WorldGrid>();
    let world_data = world.resource::<WorldData>();
    let allowed = world.resource::<RemoteAllowedTowns>();
    let squad_state = world.resource::<SquadState>();
    let town_index = world.resource::<crate::resources::TownIndex>();
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
    let town_entity = town_index.0.get(&(target_town as i32)).copied();
    let town_food = town_entity
        .and_then(|e| world.get::<crate::components::FoodStore>(e))
        .map(|f| f.0)
        .unwrap_or(0);
    let town_gold = town_entity
        .and_then(|e| world.get::<crate::components::GoldStore>(e))
        .map(|g| g.0)
        .unwrap_or(0);

    // Buildings
    let buildings: Vec<(String, usize, usize)> = if let Some(_t) = town {
        entity_map
            .iter_instances()
            .filter(|inst| inst.town_idx as usize == target_town)
            .map(|inst| {
                let label = crate::constants::building_def(inst.kind).label.to_string();
                let (col, row) = grid.world_to_grid(inst.position);
                (label, col, row)
            })
            .collect()
    } else {
        Vec::new()
    };

    // Squads
    let squads: Vec<(usize, usize, Option<i32>, Option<i32>)> = squad_state
        .squads
        .iter()
        .enumerate()
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
    let levels = town_entity
        .and_then(|e| world.get::<crate::components::TownUpgradeLevel>(e))
        .map(|u| u.0.clone())
        .unwrap_or_default();
    let upgrade_nodes = &crate::systems::stats::UPGRADES.nodes;
    let upgrades: Vec<(usize, String, u8, String, String)> = upgrade_nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let level = levels.get(i).copied().unwrap_or(0);
            let pct = format!("{:.0}%", node.pct * 100.0);
            let cost = format_cost(node.cost);
            (i, node.label.to_string(), level, pct, cost)
        })
        .collect();

    // Combat log — filter to events relevant to this town's faction
    let combat_log: Vec<(i32, i32, i32, String)> = log_ring
        .events
        .iter()
        .filter(|e| e.faction == town_faction || e.kind == CombatEventKind::Llm)
        .map(|e| (e.day, e.hour, e.minute, e.message.clone()))
        .collect();

    // Inbox
    let inbox = inbox_by_town.remove(&target_town).unwrap_or_default();

    // Compact NPC counts
    let town_prefix = format!("t{}_", target_town);
    let npcs = compact_npc_counts(&npc_counts, &town_prefix);

    let response = SummaryResponse {
        day,
        hour,
        minute,
        paused,
        time_scale,
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
    col: usize,
    row: usize,
}

pub fn build_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: BuildParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;
    let kind = parse_building_kind(&p.kind)
        .ok_or_else(|| brp_err(format!("unknown building kind: {}", p.kind)))?;

    // Validate town exists
    let town_count = world.resource::<WorldData>().towns.len();
    if p.town >= town_count {
        return Err(brp_err(format!(
            "town {} out of range (max {})",
            p.town,
            town_count - 1
        )));
    }

    let pos = world
        .resource::<crate::world::WorldGrid>()
        .grid_to_world(p.col, p.row);
    queue_llm_log(
        world,
        p.town,
        format!("build {} at ({},{})", p.kind, p.col, p.row),
        Some(pos),
    );

    world
        .resource_mut::<RemoteBuildQueue>()
        .0
        .push(RemoteBuild {
            town: p.town,
            kind,
            col: p.col,
            row: p.row,
        });

    toon_ok(json!({"status": "queued", "kind": p.kind, "town": p.town, "col": p.col, "row": p.row}))
}

// --- endless/destroy --------------------------------------------------------

#[derive(Deserialize)]
struct DestroyParams {
    town: usize,
    col: usize,
    row: usize,
}

pub fn destroy_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: DestroyParams = parse_some(params)?;
    check_town_allowed(world, p.town)?;

    let town_count = world.resource::<WorldData>().towns.len();
    if p.town >= town_count {
        return Err(brp_err(format!("town {} out of range", p.town)));
    }

    let pos = world
        .resource::<crate::world::WorldGrid>()
        .grid_to_world(p.col, p.row);
    queue_llm_log(
        world,
        p.town,
        format!("destroy at ({},{})", p.col, p.row),
        Some(pos),
    );

    world
        .resource_mut::<RemoteDestroyQueue>()
        .0
        .push(RemoteDestroy {
            town: p.town,
            col: p.col,
            row: p.row,
        });

    toon_ok(json!({"status": "queued", "town": p.town, "col": p.col, "row": p.row}))
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

    let parts = {
        let town_entity = world
            .resource::<crate::resources::TownIndex>()
            .0
            .get(&(p.town as i32))
            .copied()
            .ok_or_else(|| brp_err(format!("town {} out of range", p.town)))?;
        let policy = &mut world
            .get_mut::<crate::components::TownPolicy>(town_entity)
            .ok_or_else(|| brp_err(format!("town {} missing TownPolicy", p.town)))?
            .0;

        // Diff: only log fields that actually change
        let mut parts = Vec::new();
        if let Some(v) = p.eat_food {
            if v != policy.eat_food {
                parts.push(format!("eat_food={v}"));
            }
            policy.eat_food = v;
        }
        if let Some(v) = p.archer_aggressive {
            if v != policy.archer_aggressive {
                parts.push(format!("archer_aggressive={v}"));
            }
            policy.archer_aggressive = v;
        }
        if let Some(v) = p.archer_leash {
            if v != policy.archer_leash {
                parts.push(format!("archer_leash={v}"));
            }
            policy.archer_leash = v;
        }
        if let Some(v) = p.farmer_fight_back {
            if v != policy.farmer_fight_back {
                parts.push(format!("farmer_fight_back={v}"));
            }
            policy.farmer_fight_back = v;
        }
        if let Some(v) = p.prioritize_healing {
            if v != policy.prioritize_healing {
                parts.push(format!("prioritize_healing={v}"));
            }
            policy.prioritize_healing = v;
        }
        if let Some(v) = p.farmer_flee_hp {
            let v = v.clamp(0.0, 1.0);
            if (v - policy.farmer_flee_hp).abs() > f32::EPSILON {
                parts.push(format!("farmer_flee_hp={v:.1}"));
            }
            policy.farmer_flee_hp = v;
        }
        if let Some(v) = p.archer_flee_hp {
            let v = v.clamp(0.0, 1.0);
            if (v - policy.archer_flee_hp).abs() > f32::EPSILON {
                parts.push(format!("archer_flee_hp={v:.1}"));
            }
            policy.archer_flee_hp = v;
        }
        if let Some(v) = p.recovery_hp {
            let v = v.clamp(0.0, 1.0);
            if (v - policy.recovery_hp).abs() > f32::EPSILON {
                parts.push(format!("recovery_hp={v:.1}"));
            }
            policy.recovery_hp = v;
        }
        if let Some(v) = p.mining_radius {
            let v = v.clamp(0.0, 5000.0);
            if (v - policy.mining_radius).abs() > f32::EPSILON {
                parts.push(format!("mining_radius={v:.0}"));
            }
            policy.mining_radius = v;
        }
        parts
    };
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
        let squad = state
            .squads
            .get(p.squad)
            .ok_or_else(|| brp_err(format!("squad {} out of range", p.squad)))?;
        town = match squad.owner {
            SquadOwner::Player => 0,
            SquadOwner::Town(tdi) => tdi,
        };
        check_town_allowed(world, town)?;
    }

    queue_llm_log(
        world,
        town,
        format!("squad {} target ({:.0},{:.0})", p.squad, p.x, p.y),
        Some(Vec2::new(p.x, p.y)),
    );

    let mut state = world.resource_mut::<SquadState>();
    let squad = state
        .squads
        .get_mut(p.squad)
        .ok_or_else(|| brp_err(format!("squad {} out of range", p.squad)))?;
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
    if let Some(v) = p.active {
        parts.push(format!("active={v}"));
    }
    if let Some(ref s) = p.personality {
        parts.push(format!("personality={s}"));
    }
    if let Some(v) = p.build_enabled {
        parts.push(format!("build={v}"));
    }
    if let Some(v) = p.upgrade_enabled {
        parts.push(format!("upgrade={v}"));
    }
    if let Some(ref s) = p.road_style {
        parts.push(format!("roads={s}"));
    }
    let msg = if parts.is_empty() {
        "ai_manager query".to_string()
    } else {
        format!("ai_manager: {}", parts.join(", "))
    };
    queue_llm_log(world, p.town, msg, None);

    let mut ai_state = world.resource_mut::<crate::systems::AiPlayerState>();
    let player = ai_state
        .players
        .iter_mut()
        .find(|pl| pl.town_data_idx == p.town)
        .ok_or_else(|| brp_err(format!("no AI player for town {}", p.town)))?;

    if let Some(v) = p.active {
        player.active = v;
    }
    if let Some(v) = p.build_enabled {
        player.build_enabled = v;
    }
    if let Some(v) = p.upgrade_enabled {
        player.upgrade_enabled = v;
    }
    if let Some(ref s) = p.personality {
        player.personality =
            parse_personality(s).ok_or_else(|| brp_err(format!("unknown personality: {s}")))?;
    }
    if let Some(ref s) = p.road_style {
        player.road_style =
            parse_road_style(s).ok_or_else(|| brp_err(format!("unknown road_style: {s}")))?;
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

    let from_name = world
        .resource::<WorldData>()
        .towns
        .get(p.town)
        .map(|t| t.name.clone())
        .unwrap_or_default();
    queue_llm_log(
        world,
        p.town,
        format!("[chat to F{}] {}", p.to, p.message),
        None,
    );

    world.resource_mut::<ChatInbox>().push(ChatMessage {
        from_town: p.town,
        to_town: p.to,
        text: p.message.clone(),
        day,
        hour,
        minute,
        sent_to_llm: false,
        has_reply: false,
    });

    toon_ok(json!({"status": "ok", "from": from_name, "message": p.message}))
}

// --- endless/debug -----------------------------------------------------------

#[derive(Deserialize)]
struct DebugParams {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    entity: Option<String>,
    #[serde(default)]
    index: Option<usize>,
}

/// Parse `"489v9"` entity string → Bevy Entity.
fn parse_entity_str(s: &str) -> Result<Entity, BrpError> {
    let (idx_str, gen_str) = s
        .split_once('v')
        .ok_or_else(|| brp_err(format!("invalid entity '{s}' — use format '489v9'")))?;
    let index: u32 = idx_str
        .parse()
        .map_err(|_| brp_err(format!("invalid entity index '{idx_str}'")))?;
    let generation: u32 = gen_str
        .parse()
        .map_err(|_| brp_err(format!("invalid entity generation '{gen_str}'")))?;
    let ei = bevy::ecs::entity::EntityIndex::from_raw_u32(index)
        .ok_or_else(|| brp_err(format!("invalid entity index '{idx_str}'")))?;
    Ok(Entity::from_index_and_generation(
        ei,
        bevy::ecs::entity::EntityGeneration::from_bits(generation),
    ))
}

pub fn debug_handler(In(params): In<Option<Value>>, world: &mut World) -> BrpResult {
    let p: DebugParams = parse_some(params)?;

    // Entity string provided: parse "489v9", resolve to slot, auto-detect NPC vs building
    if let Some(ref s) = p.entity {
        let entity = parse_entity_str(s)?;
        let entity_map = world.resource::<EntityMap>();
        let slot = entity_map
            .slot_for_entity(entity)
            .ok_or_else(|| brp_err(format!("no entity for {entity:?}")))?;
        let is_npc = entity_map.get_npc(slot).is_some();
        return if is_npc {
            debug_npc(world, entity, slot)
        } else {
            debug_building(world, entity, slot)
        };
    }

    // Kind + index for resource-based lookups
    let kind = p.kind.as_deref().ok_or_else(|| {
        brp_err("provide 'entity' (npc/building) or 'kind'+'index' (squad/town/policy)")
    })?;
    let idx = p
        .index
        .ok_or_else(|| brp_err(format!("'index' required for kind '{kind}'")))?;
    match kind {
        "squad" => debug_squad(world, idx),
        "town" => debug_town(world, idx),
        "policy" => debug_policy(world, idx),
        _ => Err(brp_err(format!(
            "unknown kind: {kind} (use squad/town/policy, or pass 'entity' for npc/building)"
        ))),
    }
}

fn debug_npc(world: &mut World, target_entity: Entity, slot: usize) -> BrpResult {
    // ECS query — split into nested tuples to stay under Bevy's 15-element QueryData limit
    let mut query = world.query::<(
        Entity,
        &GpuSlot,
        &Job,
        &Activity,
        &Health,
        &Energy,
        &Speed,
        &Home,
        &TownId,
        &Faction,
        &CarriedLoot,
        &NpcWorkState,
        &NpcFlags,
        &CombatState,
        (
            &Personality,
            &CachedStats,
            &NpcEquipment,
            Option<&ManualTarget>,
            Option<&SquadId>,
            Option<&PatrolRoute>,
        ),
    )>();

    let mut npc_data: Option<Value> = None;
    for (
        entity,
        _gpu_slot,
        job,
        activity,
        health,
        energy,
        _speed,
        home,
        town_id,
        faction,
        carried_loot,
        work_state,
        flags,
        combat_state,
        (personality, stats, equipment, manual_target, squad_id, patrol_route),
    ) in query.iter(world)
    {
        if entity != target_entity {
            continue;
        }

        // Equipment slots — uniform array for TOON CSV table
        let mut equip: Vec<Value> = Vec::new();
        let slots: &[(&str, &Option<crate::constants::LootItem>)] = &[
            ("weapon", &equipment.weapon),
            ("helm", &equipment.helm),
            ("armor", &equipment.armor),
            ("shield", &equipment.shield),
            ("gloves", &equipment.gloves),
            ("boots", &equipment.boots),
            ("belt", &equipment.belt),
            ("amulet", &equipment.amulet),
            ("ring1", &equipment.ring1),
            ("ring2", &equipment.ring2),
        ];
        for &(label, item_opt) in slots {
            if let Some(item) = item_opt {
                equip.push(json!({
                    "slot": label, "name": item.name,
                    "rarity": item.rarity.label(),
                    "bonus": format!("{:.0}%", item.stat_bonus * 100.0),
                }));
            }
        }

        // Flags
        let mut flag_list = Vec::new();
        if flags.healing {
            flag_list.push("healing");
        }
        if flags.starving {
            flag_list.push("starving");
        }
        if flags.direct_control {
            flag_list.push("direct_control");
        }
        if flags.migrating {
            flag_list.push("migrating");
        }
        if flags.at_destination {
            flag_list.push("at_dest");
        }

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
            "entity": target_entity.to_bits(),
            "slot": slot,
            "job": format!("{:?}", job),
            "activity": activity.name(),
            "activity_phase": format!("{:?}", activity.phase),
            "activity_target": format!("{:?}", activity.target),
            "transition_reason": activity.reason,
            "last_transition_frame": activity.last_frame,
            "combat_state": combat_state.name(),
            "hp": health.0,
            "max_hp": stats.max_health,
            "energy": ((energy.0 as f64 * 10.0).round() / 10.0),
            "speed": stats.speed,
            "home_x": home.0.x as i32,
            "home_y": home.0.y as i32,
            "town_id": town_id.0,
            "faction": faction.0,
            "loot_food": carried_loot.food,
            "loot_gold": carried_loot.gold,
            "loot_equipment": carried_loot.equipment.iter().map(|i| json!({
                "name": &i.name, "rarity": i.rarity.label(),
            })).collect::<Vec<_>>(),
            "worksite": work_state.worksite.map(|e| e.to_bits()),
            "flags": flag_list,
            "manual_target": mt_str,
            "squad_id": squad_id.map(|s| s.0),
            "patrol_current": patrol_route.map(|r| r.current),
            "patrol_posts": patrol_route.map(|r| r.posts.len()),
            "personality": trait_str,
            "stats_damage": stats.damage,
            "stats_range": r2(stats.range),
            "stats_cooldown": r2(stats.cooldown),
            "stats_hp_regen": r2(stats.hp_regen),
            "stats_stamina": stats.stamina,
            "stats_berserk": stats.berserk_bonus,
            "equipment": equip,
        }));
        break;
    }

    let Some(mut data) = npc_data else {
        return Err(brp_err(format!("no NPC for entity {target_entity:?}")));
    };

    // Resource-based data (immutable borrows after query)
    let entity_map = world.resource::<EntityMap>();
    let npc_logs = world.resource::<NpcLogCache>();
    let thrash = world.resource::<NpcTargetThrashDebug>();
    let gpu_state = world.resource::<GpuReadState>();
    let world_data = world.resource::<WorldData>();
    let town_index = world.resource::<crate::resources::TownIndex>();
    let squad_state = world.resource::<SquadState>();
    let game_time = world.resource::<GameTime>();

    // NpcStats (name, xp → level)
    if let Some(npc) = entity_map.get_npc(slot) {
        if let Some(stats) = world.get::<crate::components::NpcStats>(npc.entity) {
            let level = crate::systems::stats::level_from_xp(stats.xp);
            let xp_next = (level + 1) * (level + 1) * 100;
            data["name"] = json!(stats.name);
            data["level"] = json!(level);
            data["xp"] = json!(stats.xp);
            data["xp_next"] = json!(xp_next);
        }
    }

    // Town name + faction name
    let town_id = data["town_id"].as_i64().unwrap_or(-1) as i32;
    if town_id >= 0 {
        if let Some(town) = world_data.towns.get(town_id as usize) {
            data["town_name"] = json!(town.name);
            data["faction_name"] = json!(format!("{} (F{})", town.name, town.faction));
        }
        let p_opt = town_index
            .0
            .get(&town_id)
            .and_then(|&e| world.get::<crate::components::TownPolicy>(e))
            .map(|tp| tp.0.clone());
        if let Some(p) = p_opt {
            data["policy_eat_food"] = json!(p.eat_food);
            data["policy_aggressive"] = json!(p.archer_aggressive);
            data["policy_leash"] = json!(p.archer_leash);
            data["policy_archer_flee"] = json!(r2(p.archer_flee_hp));
            data["policy_farmer_flee"] = json!(r2(p.farmer_flee_hp));
            data["policy_heal_prio"] = json!(p.prioritize_healing);
            data["policy_recovery"] = json!(r2(p.recovery_hp));
        }
    }

    // Readback: actual rendered position, movement target, combat target
    let i2 = slot * 2;
    if i2 + 1 < gpu_state.positions.len() {
        data["world_x"] = json!(gpu_state.positions[i2] as i32);
        data["world_y"] = json!(gpu_state.positions[i2 + 1] as i32);
    }
    let gpu_data = world.resource::<crate::gpu::EntityGpuState>();
    if i2 + 1 < gpu_data.targets.len() {
        data["target_x"] = json!(gpu_data.targets[i2] as i32);
        data["target_y"] = json!(gpu_data.targets[i2 + 1] as i32);
    }
    let ct = gpu_state.combat_targets.get(slot).copied().unwrap_or(-1);
    data["combat_target"] = json!(ct);
    if ct >= 0 {
        let ti = ct as usize;
        if let Some(inst) = entity_map.get_instance(ti) {
            data["combat_info"] = json!(format!(
                "building {} F{} @{},{}",
                format!("{:?}", inst.kind),
                inst.faction,
                inst.position.x as i32,
                inst.position.y as i32
            ));
        } else if let Some(tnpc) = entity_map.get_npc(ti) {
            let tx = gpu_state.positions.get(ti * 2).copied().unwrap_or(-9999.0);
            let ty = gpu_state
                .positions
                .get(ti * 2 + 1)
                .copied()
                .unwrap_or(-9999.0);
            data["combat_info"] = json!(format!(
                "npc F{} @{},{}{}",
                tnpc.faction,
                tx as i32,
                ty as i32,
                if tnpc.dead { " dead" } else { "" }
            ));
        }
    }

    // Worksite detail
    if let Some(uid_val) = data["worksite"].as_u64() {
        let ws_entity = Entity::from_bits(uid_val);
        if let Some(ws_slot) = entity_map.slot_for_entity(ws_entity) {
            if let Some(inst) = entity_map.get_instance(ws_slot) {
                let max_occ = crate::constants::building_def(inst.kind)
                    .worksite
                    .map_or(0, |w| w.max_occupants);
                data["worksite_kind"] = json!(format!("{:?}", inst.kind));
                data["worksite_occupants"] = json!(entity_map.occupant_count(ws_slot));
                data["worksite_max_occ"] = json!(max_occ);
                let growth_pct = entity_map
                    .entities
                    .get(&ws_slot)
                    .and_then(|&e| world.get::<crate::components::ProductionState>(e))
                    .map(|ps| ps.progress * 100.0)
                    .unwrap_or(0.0);
                data["worksite_growth"] = json!(format!("{:.0}%", growth_pct));
            }
        }
    }

    // Squad detail
    if let Some(sq_val) = data["squad_id"].as_i64() {
        let sq = sq_val as usize;
        if sq < squad_state.squads.len() {
            let s = &squad_state.squads[sq];
            data["squad_members"] = json!(s.members.len());
            data["squad_target_x"] = json!(s.target.map(|v| v.x as i32));
            data["squad_target_y"] = json!(s.target.map(|v| v.y as i32));
            data["squad_hold_fire"] = json!(s.hold_fire);
            data["squad_patrol"] = json!(s.patrol_enabled);
            data["squad_rest"] = json!(s.rest_when_tired);
        }
    }

    // Target thrash diagnostics
    data["thrash_changes"] = json!(
        thrash
            .sink_target_changes_this_minute
            .get(slot)
            .copied()
            .unwrap_or(0)
    );
    data["thrash_ping_pong"] = json!(
        thrash
            .sink_ping_pong_this_minute
            .get(slot)
            .copied()
            .unwrap_or(0)
    );
    data["thrash_writes"] = json!(
        thrash
            .sink_writes_this_minute
            .get(slot)
            .copied()
            .unwrap_or(0)
    );
    data["thrash_flips"] = json!(
        thrash
            .reason_flips_this_minute
            .get(slot)
            .copied()
            .unwrap_or(0)
    );
    data["thrash_reason"] = json!(
        thrash
            .last_reason
            .get(slot)
            .map(String::as_str)
            .unwrap_or("-")
    );

    // Sprite indices from EntityGpuState (CPU-side visual state)
    let si = slot * 4;
    if si + 3 < gpu_data.sprite_indices.len() {
        data["sprite_col"] = json!(gpu_data.sprite_indices[si]);
        data["sprite_row"] = json!(gpu_data.sprite_indices[si + 1]);
        data["sprite_atlas"] = json!(gpu_data.sprite_indices[si + 2]);
    }

    // Visual upload data (what actually gets sent to GPU)
    let visual_upload = world.resource::<crate::gpu::NpcVisualUpload>();
    let vb = slot * 8;
    if vb + 7 < visual_upload.visual_data.len() {
        data["visual_col"] = json!(visual_upload.visual_data[vb]);
        data["visual_row"] = json!(visual_upload.visual_data[vb + 1]);
        data["visual_atlas"] = json!(visual_upload.visual_data[vb + 2]);
        data["visual_flash"] = json!(visual_upload.visual_data[vb + 3]);
    }

    // Active projectiles fired by this NPC
    let proj_writes = world.resource::<crate::gpu::ProjBufferWrites>();
    let proj_pos_state = world.resource::<ProjPositionState>();
    let mut projs = Vec::new();
    for &pi in &proj_writes.active_set {
        if proj_writes.shooters[pi] == slot as i32 {
            let pi2 = pi * 2;
            let (px, py) = if pi2 + 1 < proj_pos_state.0.len() {
                (proj_pos_state.0[pi2], proj_pos_state.0[pi2 + 1])
            } else {
                (
                    proj_writes.positions.get(pi2).copied().unwrap_or(0.0),
                    proj_writes.positions.get(pi2 + 1).copied().unwrap_or(0.0),
                )
            };
            projs.push(json!({
                "slot": pi,
                "x": px as i32,
                "y": py as i32,
                "vx": proj_writes.velocities[pi2] as i32,
                "vy": proj_writes.velocities[pi2 + 1] as i32,
                "damage": proj_writes.damages[pi] as i32,
                "faction": proj_writes.factions[pi],
                "lifetime": (proj_writes.lifetimes[pi] * 10.0).round() / 10.0,
            }));
        }
    }
    data["projectiles"] = json!(projs);

    // NPC activity log (last 20 entries)
    if slot < npc_logs.logs.len() {
        let entries: Vec<Value> = npc_logs.logs[slot]
            .iter()
            .rev()
            .take(20)
            .map(|e| {
                json!(format!(
                    "D{}:{:02}:{:02} {}",
                    e.day, e.hour, e.minute, e.message
                ))
            })
            .collect();
        data["log"] = json!(entries);
    }

    // Timestamp
    data["day"] = json!(game_time.day());
    data["hour"] = json!(game_time.hour());
    data["minute"] = json!(game_time.minute());

    toon_ok(data)
}

fn debug_building(world: &mut World, _entity: Entity, slot: usize) -> BrpResult {
    let (inst, bld_entity, occupants) = {
        let entity_map = world.resource::<EntityMap>();
        let inst = entity_map
            .get_instance(slot)
            .ok_or_else(|| brp_err(format!("no building at slot {slot}")))?
            .clone();
        let bld_entity = entity_map.entities.get(&slot).copied();
        let occupants = entity_map.occupant_count(slot);
        (inst, bld_entity, occupants)
    };
    let world_data = world.resource::<WorldData>();
    let game_time = world.resource::<GameTime>();

    let def = crate::constants::building_def(inst.kind);
    let town_name = world_data
        .towns
        .get(inst.town_idx as usize)
        .map(|t| t.name.as_str())
        .unwrap_or("?");
    let faction_str = world_data
        .towns
        .get(inst.town_idx as usize)
        .map(|t| format!("{} (F{})", t.name, t.faction))
        .unwrap_or_else(|| {
            if inst.kind == BuildingKind::GoldMine {
                "Unowned".into()
            } else {
                "?".into()
            }
        });

    // Grid coords (world grid)
    let grid = world.resource::<crate::world::WorldGrid>();
    let (col, row) = grid.world_to_grid(inst.position);

    // HP from entity
    let hp = bld_entity
        .and_then(|e| world.get::<Health>(e))
        .map(|h| h.0)
        .unwrap_or(0.0);

    let mut data = json!({
        "entity": _entity.to_bits(),
        "slot": slot,
        "kind": format!("{:?}", inst.kind),
        "label": def.label,
        "town": town_name,
        "faction": faction_str,
        "world_x": inst.position.x as i32,
        "world_y": inst.position.y as i32,
        "grid_col": col,
        "grid_row": row,
        "hp": hp,
        "max_hp": def.hp,
        "town_idx": inst.town_idx,
        "occupants": occupants,
        "growth": "0%",
        "under_construction": 0.0f32,
        "npc_uid": serde_json::Value::Null,
        "respawn_timer": 0.0f32,
    });

    // Sprite/visual data
    let gpu_data = world.resource::<crate::gpu::EntityGpuState>();
    let si = slot * 4;
    if si + 3 < gpu_data.sprite_indices.len() {
        data["sprite_col"] = json!(gpu_data.sprite_indices[si]);
        data["sprite_row"] = json!(gpu_data.sprite_indices[si + 1]);
        data["sprite_atlas"] = json!(gpu_data.sprite_indices[si + 2]);
    }

    // ECS component data
    if let Some(e) = bld_entity {
        if let Some(ps) = world.get::<crate::components::ProductionState>(e) {
            data["growth"] = json!(format!("{:.0}%", ps.progress * 100.0));
        }
        if let Some(cp) = world.get::<crate::components::ConstructionProgress>(e) {
            data["under_construction"] = json!(cp.0);
        }
        if let Some(ss) = world.get::<crate::components::SpawnerState>(e) {
            data["npc_slot"] = json!(ss.npc_slot);
            data["respawn_timer"] = json!(ss.respawn_timer);
        }
        if let Some(mc) = world.get::<crate::components::MinerHomeConfig>(e) {
            data["assigned_mine_x"] = json!(mc.assigned_mine.map(|v| v.x as i32));
            data["assigned_mine_y"] = json!(mc.assigned_mine.map(|v| v.y as i32));
            data["manual_mine"] = json!(mc.manual_mine);
        }
        if let Some(wl) = world.get::<crate::components::WallLevel>(e) {
            data["wall_level"] = json!(wl.0);
        }
    }

    // Worksite info
    if let Some(ws) = def.worksite {
        data["ws_max_occ"] = json!(ws.max_occupants);
        data["ws_drift"] = json!(ws.drift_radius);
        data["ws_harvest"] = json!(format!("{:?}", ws.harvest_item));
        data["ws_town_scoped"] = json!(ws.town_scoped);
    }

    data["day"] = json!(game_time.day());
    data["hour"] = json!(game_time.hour());
    data["minute"] = json!(game_time.minute());

    toon_ok(data)
}

// --- debug: squad, town, policy ----------------------------------------------

fn debug_squad(world: &mut World, idx: usize) -> BrpResult {
    let squad_state = world.resource::<SquadState>();
    let squad = squad_state
        .squads
        .get(idx)
        .ok_or_else(|| {
            brp_err(format!(
                "no squad at index {idx} (max {})",
                squad_state.squads.len()
            ))
        })?
        .clone();
    let game_time = world.resource::<GameTime>();
    let (day, hour, minute) = (game_time.day(), game_time.hour(), game_time.minute());

    // Resolve member details
    let entity_map = world.resource::<EntityMap>();
    let member_slots: Vec<(u64, Option<usize>, String)> = squad
        .members
        .iter()
        .map(|e| {
            let slot = entity_map.slot_for_entity(*e);
            let name = world
                .get::<crate::components::NpcStats>(*e)
                .map(|s| s.name.clone())
                .unwrap_or_default();
            (e.to_bits(), slot, name)
        })
        .collect();

    // ECS query for member stats — uniform fields for TOON CSV table
    let mut members_json = Vec::new();
    for (uid, slot, name) in &member_slots {
        let (mut job, mut dead, mut activity, mut energy, mut hp, mut max_hp) = (
            "".to_string(),
            false,
            "".to_string(),
            0.0_f32,
            0.0_f32,
            0.0_f32,
        );
        if let Some(s) = slot {
            if let Some(npc) = world.resource::<EntityMap>().get_npc(*s) {
                let entity = npc.entity;
                job = format!("{:?}", npc.job);
                dead = npc.dead;
                if let Some(a) = world.get::<Activity>(entity) {
                    activity = a.name().to_string();
                }
                if let Some(e) = world.get::<Energy>(entity) {
                    energy = e.0;
                }
                if let Some(h) = world.get::<Health>(entity) {
                    hp = h.0;
                }
                if let Some(s) = world.get::<CachedStats>(entity) {
                    max_hp = s.max_health;
                }
            }
        }
        members_json.push(json!({
            "uid": uid, "name": name, "job": job, "dead": dead,
            "activity": activity, "energy": r2(energy), "hp": hp, "max_hp": max_hp,
        }));
    }

    let data = json!({
        "squad_index": idx,
        "members": members_json,
        "member_count": squad.members.len(),
        "target_x": squad.target.map(|v| v.x as i32),
        "target_y": squad.target.map(|v| v.y as i32),
        "target_size": squad.target_size,
        "patrol_enabled": squad.patrol_enabled,
        "rest_when_tired": squad.rest_when_tired,
        "loot_threshold": squad.loot_threshold,
        "wave_active": squad.wave_active,
        "wave_start_count": squad.wave_start_count,
        "wave_min_start": squad.wave_min_start,
        "wave_retreat_below_pct": squad.wave_retreat_below_pct,
        "owner": format!("{:?}", squad.owner),
        "hold_fire": squad.hold_fire,
        "day": day, "hour": hour, "minute": minute,
    });
    toon_ok(data)
}

fn debug_town(world: &mut World, idx: usize) -> BrpResult {
    let world_data = world.resource::<WorldData>();
    let town = world_data.towns.get(idx).ok_or_else(|| {
        brp_err(format!(
            "no town at index {idx} (max {})",
            world_data.towns.len()
        ))
    })?;
    let name = town.name.clone();
    let faction = town.faction;
    let center = [town.center.x, town.center.y];

    let town_index = world.resource::<crate::resources::TownIndex>();
    let town_entity = town_index.0.get(&(idx as i32)).copied();
    let area_level = town_entity
        .and_then(|e| world.get::<crate::components::TownAreaLevel>(e))
        .map(|a| a.0)
        .unwrap_or(0);
    let food = town_entity
        .and_then(|e| world.get::<crate::components::FoodStore>(e))
        .map(|f| f.0)
        .unwrap_or(0);
    let gold = town_entity
        .and_then(|e| world.get::<crate::components::GoldStore>(e))
        .map(|g| g.0)
        .unwrap_or(0);
    let faction_stat = world
        .resource::<FactionStats>()
        .stats
        .get(faction as usize)
        .cloned();
    let game_time = world.resource::<GameTime>();
    let (day, hour, minute) = (game_time.day(), game_time.hour(), game_time.minute());

    // Policies inline
    let policy = town_entity
        .and_then(|e| world.get::<crate::components::TownPolicy>(e))
        .map(|tp| tp.0.clone());

    // Squads belonging to this town
    let squad_state = world.resource::<SquadState>();
    let squads: Vec<Value> = squad_state
        .squads
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let belongs = match s.owner {
                SquadOwner::Player => faction == crate::constants::FACTION_PLAYER,
                SquadOwner::Town(t) => t == idx,
            };
            if !belongs || s.members.is_empty() {
                return None;
            }
            Some(json!({
                "index": i,
                "members": s.members.len(),
                "target_x": s.target.map(|v| v.x as i32),
                "target_y": s.target.map(|v| v.y as i32),
                "rest": s.rest_when_tired,
                "loot_threshold": s.loot_threshold,
            }))
        })
        .collect();

    // NPC counts by job for this town
    let entity_map = world.resource::<EntityMap>();
    let mut job_counts: BTreeMap<String, i32> = BTreeMap::new();
    for npc in entity_map.iter_npcs() {
        if npc.town_idx as usize == idx && !npc.dead {
            *job_counts.entry(format!("{:?}", npc.job)).or_default() += 1;
        }
    }

    // Building counts by kind
    let mut building_counts: BTreeMap<String, i32> = BTreeMap::new();
    for inst in entity_map.iter_instances() {
        if inst.town_idx as usize == idx {
            *building_counts
                .entry(format!("{:?}", inst.kind))
                .or_default() += 1;
        }
    }

    // Compact strings for npcs/buildings
    let npcs_str = job_counts
        .iter()
        .map(|(k, v)| format!("{}:{}", k, v))
        .collect::<Vec<_>>()
        .join(",");
    let buildings_str = building_counts
        .iter()
        .map(|(k, v)| format!("{}:{}", k, v))
        .collect::<Vec<_>>()
        .join(",");

    let mut data = json!({
        "town_index": idx,
        "name": name,
        "faction": faction,
        "center_x": center[0] as i32,
        "center_y": center[1] as i32,
        "area_level": area_level,
        "food": food,
        "gold": gold,
        "npcs": npcs_str,
        "buildings": buildings_str,
        "squads": squads,
        "day": day, "hour": hour, "minute": minute,
    });
    if let Some(fs) = faction_stat {
        data["faction_alive"] = json!(fs.alive);
        data["faction_dead"] = json!(fs.dead);
        data["faction_kills"] = json!(fs.kills);
    }
    if let Some(p) = policy {
        data["policy_eat_food"] = json!(p.eat_food);
        data["policy_aggressive"] = json!(p.archer_aggressive);
        data["policy_leash"] = json!(p.archer_leash);
        data["policy_archer_flee"] = json!(r2(p.archer_flee_hp));
        data["policy_farmer_flee"] = json!(r2(p.farmer_flee_hp));
        data["policy_fight_back"] = json!(p.farmer_fight_back);
        data["policy_heal_prio"] = json!(p.prioritize_healing);
        data["policy_recovery"] = json!(r2(p.recovery_hp));
    }
    toon_ok(data)
}

fn debug_policy(world: &mut World, idx: usize) -> BrpResult {
    let town_index = world.resource::<crate::resources::TownIndex>();
    let town_entity = town_index
        .0
        .get(&(idx as i32))
        .copied()
        .ok_or_else(|| brp_err(format!("no town at index {idx}")))?;
    let p = world
        .get::<crate::components::TownPolicy>(town_entity)
        .ok_or_else(|| brp_err(format!("town {idx} missing TownPolicy")))?
        .0
        .clone();
    let world_data = world.resource::<WorldData>();
    let town_name = world_data
        .towns
        .get(idx)
        .map(|t| t.name.as_str())
        .unwrap_or("?");
    let game_time = world.resource::<GameTime>();
    let data = json!({
        "town_index": idx,
        "town_name": town_name,
        "eat_food": p.eat_food,
        "archer_aggressive": p.archer_aggressive,
        "archer_leash": p.archer_leash,
        "archer_flee_hp": r2(p.archer_flee_hp),
        "farmer_flee_hp": r2(p.farmer_flee_hp),
        "farmer_fight_back": p.farmer_fight_back,
        "prioritize_healing": p.prioritize_healing,
        "recovery_hp": r2(p.recovery_hp),
        "day": game_time.day(), "hour": game_time.hour(), "minute": game_time.minute(),
    });
    toon_ok(data)
}

// --- endless/perf ------------------------------------------------------------

pub fn perf_handler(In(_params): In<Option<Value>>, world: &World) -> BrpResult {
    let timings = world.resource::<crate::resources::SystemTimings>();
    let ups = world.resource::<crate::resources::UpsCounter>();
    let faction_stats = world.resource::<FactionStats>();

    let frame_ms = timings.frame_ms.lock().map(|v| *v).unwrap_or(0.0);
    let fps = if frame_ms > 0.0 {
        1000.0 / frame_ms
    } else {
        0.0
    };
    let npc_count: i32 = faction_stats.stats.iter().map(|s| s.alive).sum();
    let entity_count = world.entities().len() as usize;

    let mut response = json!({
        "fps": (fps * 10.0).round() / 10.0,
        "frame_ms": (frame_ms * 100.0).round() / 100.0,
        "ups": ups.display_ups,
        "npc_count": npc_count,
        "entity_count": entity_count,
    });

    // Include per-system timings if profiling is enabled
    if timings.enabled {
        let system_timings = timings.get_timings();
        let traced_timings = timings.get_traced_timings();
        let mut all: std::collections::BTreeMap<String, f64> = std::collections::BTreeMap::new();
        for (k, v) in system_timings {
            all.insert(k.to_string(), (v as f64 * 100.0).round() / 100.0);
        }
        for (k, v) in traced_timings {
            all.insert(k, (v as f64 * 100.0).round() / 100.0);
        }
        response["timings"] = json!(all);
    }

    toon_ok(response)
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
    mut town_access: crate::systemparams::TownAccess,
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
        if build.town >= world_state.world_data.towns.len() {
            continue;
        }
        let pos = world_state.grid.grid_to_world(build.col, build.row);
        let cost = building_cost(build.kind);

        let mut food_val = town_access.food(build.town as i32);
        let _ = world_state.place_building(
            &mut food_val,
            build.kind,
            build.town,
            pos,
            cost,
            &mut gpu_updates,
            &mut commands,
        );
        if let Some(mut f) = town_access.food_mut(build.town as i32) {
            f.0 = food_val;
        }
    }

    // Drain destroy queue
    let destroys: Vec<RemoteDestroy> = destroy_q.0.drain(..).collect();
    for destroy in destroys {
        let Some(town) = world_state.world_data.towns.get(destroy.town) else {
            continue;
        };
        let town_name = town.name.clone();

        // Look up building at grid cell
        let Some(inst) = world_state
            .entity_map
            .get_at_grid(destroy.col as i32, destroy.row as i32)
        else {
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
        let Some(&target_entity) = world_state.entity_map.entities.get(&slot) else {
            continue;
        };
        damage_writer.write(crate::messages::DamageMsg {
            target: target_entity,
            amount: f32::MAX,
            attacker: -1,
            attacker_faction: 0,
        });

        let _ = world_state.destroy_building(
            &mut combat_log,
            &game_time,
            destroy.col,
            destroy.row,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{
        Activity, ActivityKind, ActivityPhase, ActivityTarget, CachedStats, CarriedLoot,
        CombatState, Faction, GpuSlot, Health, Home, Job, NpcEquipment, NpcFlags, NpcWorkState,
        Personality, Speed, TownId,
    };
    use crate::gpu::{EntityGpuState, NpcVisualUpload, ProjBufferWrites};
    use crate::resources::{
        EntityMap, GameTime, GpuReadState, NpcLogCache, NpcTargetThrashDebug, ProjPositionState,
        SquadState, TownIndex,
    };
    use crate::world::WorldData;

    fn decode_toon(response: Value) -> Value {
        let encoded = response
            .as_str()
            .expect("debug response should be TOON text");
        serde_toon2::from_str(encoded).expect("debug response should decode from TOON")
    }

    fn setup_debug_world(activity: Activity) -> (World, Entity, usize) {
        let mut world = World::default();
        world.insert_resource(EntityMap::default());
        world.insert_resource(NpcLogCache::default());
        world.insert_resource(NpcTargetThrashDebug::default());
        world.insert_resource(GpuReadState::default());
        world.insert_resource(WorldData::default());
        world.insert_resource(TownIndex::default());
        world.insert_resource(SquadState::default());
        world.insert_resource(GameTime::default());
        world.insert_resource(EntityGpuState::default());
        world.insert_resource(NpcVisualUpload::default());
        world.insert_resource(ProjBufferWrites::default());
        world.insert_resource(ProjPositionState::default());

        let slot = 7;
        let entity = world
            .spawn_empty()
            .insert((
                GpuSlot(slot),
                Job::Archer,
                activity,
                Health(42.0),
                crate::components::Energy(81.5),
                Speed(123.0),
                Home(Vec2::new(96.0, 64.0)),
                TownId(-1),
            ))
            .insert((
                Faction(3),
                CarriedLoot::default(),
                NpcWorkState::default(),
                NpcFlags::default(),
                CombatState::None,
                Personality::default(),
                CachedStats {
                    damage: 9.0,
                    range: 120.0,
                    cooldown: 1.5,
                    projectile_speed: 0.0,
                    projectile_lifetime: 0.0,
                    max_health: 50.0,
                    speed: 123.0,
                    stamina: 1.0,
                    hp_regen: 0.25,
                    berserk_bonus: 0.0,
                },
                NpcEquipment::default(),
            ))
            .id();

        world
            .resource_mut::<EntityMap>()
            .register_npc(slot, entity, Job::Archer, 3, -1);
        (world, entity, slot)
    }

    #[test]
    fn debug_npc_includes_activity_transition_fields() {
        let activity = Activity {
            kind: ActivityKind::Patrol,
            phase: ActivityPhase::Holding,
            target: ActivityTarget::PatrolPost { route: 2, index: 5 },
            ticks_waiting: 11,
            recover_until: 0.0,
            reason: "unit-test",
            last_frame: 77,
        };
        let (mut world, entity, slot) = setup_debug_world(activity);

        let response = debug_npc(&mut world, entity, slot).expect("debug_npc should succeed");
        let data = decode_toon(response);

        assert_eq!(data["activity"], "Patrol");
        assert_eq!(data["activity_phase"], "Holding");
        assert_eq!(data["activity_target"], "PatrolPost { route: 2, index: 5 }");
        assert_eq!(data["transition_reason"], "unit-test");
        assert_eq!(data["last_transition_frame"], 77);
    }
}
