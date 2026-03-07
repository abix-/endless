//! Built-in LLM player — spawns `claude --print` each cycle to get strategic decisions.
//! Reads ECS resources directly, no HTTP/BRP round-trip.

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{mpsc, Mutex};

use crate::resources::*;
use crate::world::WorldData;

const DEFAULT_CYCLE_SECS: f32 = 20.0;

/// LLM communication state — displayed as a status icon in the HUD.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmStatus {
    /// Timer counting down, no active request.
    Idle,
    /// Spawned claude process, piping state data.
    Sending,
    /// Waiting for claude to respond.
    Thinking,
    /// Got response, executed N actions.
    Done(usize),
}

#[derive(Resource)]
pub struct LlmPlayerState {
    timer: Timer,
    receiver: Option<Mutex<mpsc::Receiver<String>>>,
    prompt: String,
    pub town_idx: usize,
    pub status: LlmStatus,
    /// One-shot topics requested by `query` — included in next cycle only, then cleared.
    pending_queries: Vec<String>,
    /// Persistent topic subscriptions — included in every cycle's state payload.
    subscriptions: Vec<String>,
    /// Last CLI command that was (or will be) executed — for settings panel display.
    pub last_command: String,
    /// Last TOON state payload sent as stdin — for settings panel display.
    pub last_payload: String,
    /// Last raw response from claude — for settings panel display.
    pub last_response: String,
}

impl LlmPlayerState {
    pub fn new(town_idx: usize) -> Self {
        let prompt = load_prompt();
        println!("[LLM] Built-in LLM player initialized for town {town_idx}");
        // Fire immediately on first tick, then repeat every DEFAULT_CYCLE_SECS
        let mut timer = Timer::from_seconds(DEFAULT_CYCLE_SECS, TimerMode::Repeating);
        timer.tick(std::time::Duration::from_secs_f32(DEFAULT_CYCLE_SECS));
        Self {
            timer,
            receiver: None,
            prompt,
            town_idx,
            status: LlmStatus::Idle,
            pending_queries: Vec::new(),
            subscriptions: Vec::new(),
            last_command: String::new(),
            last_payload: String::new(),
            last_response: String::new(),
        }
    }
}

fn load_prompt() -> String {
    for path in [
        "llm-player/prompt_builtin.md",
        "../llm-player/prompt_builtin.md",
    ] {
        if let Ok(content) = std::fs::read_to_string(path) {
            info!("[LLM] Loaded prompt from {path}");
            return content;
        }
    }
    warn!("[LLM] No prompt_builtin.md found, using minimal fallback");
    "You control a town in a real-time strategy game. Respond with a JSON actions array.".into()
}

/// Bundled read-only resources for state serialization.
#[derive(SystemParam)]
pub struct LlmReadState<'w> {
    world_data: Res<'w, WorldData>,
    food: Res<'w, FoodStorage>,
    gold: Res<'w, GoldStorage>,
    game_time: Res<'w, GameTime>,
    faction_stats: Res<'w, FactionStats>,
    entity_map: Res<'w, EntityMap>,
    pop_stats: Res<'w, PopulationStats>,
    town_upgrades: Res<'w, crate::systems::stats::TownUpgrades>,
    reputation: Res<'w, crate::resources::Reputation>,
    world_grid: Res<'w, crate::world::WorldGrid>,
}

/// Bundled mutable resources for executing actions.
#[derive(SystemParam)]
pub struct LlmWriteState<'w> {
    policies: ResMut<'w, TownPolicies>,
    build_q: ResMut<'w, crate::systems::remote::RemoteBuildQueue>,
    destroy_q: ResMut<'w, crate::systems::remote::RemoteDestroyQueue>,
    upgrade_q: ResMut<'w, crate::systems::remote::RemoteUpgradeQueue>,
    combat_log: ResMut<'w, CombatLog>,
    squad_state: ResMut<'w, SquadState>,
    chat_inbox: ResMut<'w, ChatInbox>,
}

/// Build game state JSON directly from ECS resources.
fn build_state_json(read: &LlmReadState, write: &LlmWriteState, town_idx: usize, queries: &[String]) -> Value {
    let own_center = read.world_data.towns.get(town_idx)
        .map(|t| t.center)
        .unwrap_or_default();
    let own_faction = read.world_data.towns.get(town_idx)
        .map(|t| t.faction)
        .unwrap_or(0);

    let mut towns = Vec::new();
    for (ti, town) in read.world_data.towns.iter().enumerate() {
        let is_own = ti == town_idx;
        let distance = own_center.distance(town.center) as i32;

        // All towns get building counts (not full position lists — scales O(types) not O(buildings))
        let mut counts: std::collections::BTreeMap<&str, i32> = std::collections::BTreeMap::new();
        for inst in read.entity_map.iter_instances() {
            if inst.town_idx as usize == ti {
                let label = crate::constants::building_def(inst.kind).label;
                *counts.entry(label).or_default() += 1;
            }
        }
        let buildings_val = json!(counts);

        let mut squads = Vec::new();
        for (si, squad) in write.squad_state.squads.iter().enumerate() {
            let squad_town = match squad.owner {
                SquadOwner::Player => 0,
                SquadOwner::Town(tdi) => tdi,
            };
            if squad_town == ti {
                squads.push(json!({
                    "index": si,
                    "members": squad.members.len(),
                    "target": squad.target.map(|t| json!({"x": t.x, "y": t.y})),
                }));
            }
        }

        let inbox: Vec<Value> = write.chat_inbox
            .messages
            .iter()
            .filter(|m| m.to_town == ti)
            .map(|m| json!({"from": m.from_town, "message": &m.text}))
            .collect();

        // How YOUR faction feels about this town's faction (negative = they killed your NPCs)
        let rep = read.reputation.get(own_faction, town.faction);

        let mut town_json = json!({
            "index": ti,
            "name": town.name,
            "faction": town.faction,
            "center": {"x": town.center.x, "y": town.center.y},
            "distance": distance,
            "reputation": rep,
            "food": read.food.food.get(ti).copied().unwrap_or(0),
            "gold": read.gold.gold.get(ti).copied().unwrap_or(0),
            "buildings": buildings_val,
            "squads": squads,
            "llm": is_own,
            "inbox": inbox,
        });

        // Own town gets 10 precomputed buildable slots (sampled for spatial spread)
        if is_own {
            let all_empty = crate::world::empty_slots(ti, town.center, &read.world_grid, &read.entity_map);
            let step = if all_empty.len() <= 10 { 1 } else { all_empty.len() / 10 };
            let slots: Vec<Value> = all_empty.iter()
                .step_by(step)
                .take(10)
                .map(|&(r, c)| json!({"row": r, "col": c}))
                .collect();
            town_json["open_slots"] = json!(slots);
        }

        towns.push(town_json);
    }

    let fstats: Vec<Value> = read.faction_stats
        .stats
        .iter()
        .enumerate()
        .map(|(i, s)| json!({"faction": i, "alive": s.alive, "dead": s.dead, "kills": s.kills}))
        .collect();

    let mut root = json!({
        "game_time": {
            "day": read.game_time.day(),
            "hour": read.game_time.hour(),
            "minute": read.game_time.minute(),
        },
        "towns": towns,
        "factions": fstats,
        "your_town": town_idx,
    });

    // Append queried topics
    for topic in queries {
        match topic.as_str() {
            "npcs" => {
                let mut counts: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
                for (&(job_id, town_id), stats) in &read.pop_stats.0 {
                    if town_id as usize == town_idx {
                        let job_name = format!("{:?}", crate::components::Job::from_i32(job_id));
                        counts.insert(job_name, json!({"alive": stats.alive, "working": stats.working, "dead": stats.dead}));
                    }
                }
                root["npcs"] = json!(counts);
            }
            "combat_log" => {
                let entries: Vec<Value> = write.combat_log.iter_all()
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .take(20)
                    .map(|e| json!({"day": e.day, "hour": e.hour, "min": e.minute, "msg": &e.message}))
                    .collect();
                root["combat_log"] = json!(entries);
            }
            "upgrades" => {
                let levels = read.town_upgrades.town_levels(town_idx);
                let registry = &crate::systems::stats::UPGRADES;
                let upgrades: Vec<Value> = registry.nodes.iter().enumerate().map(|(i, node)| {
                    let lv = levels.get(i).copied().unwrap_or(0);
                    let cost_str = node.cost.iter()
                        .map(|(rk, amt)| format!("{} {:?}", amt, rk))
                        .collect::<Vec<_>>()
                        .join(", ");
                    json!({"idx": i, "name": node.label, "level": lv, "cost": cost_str})
                }).collect();
                root["upgrades"] = json!(upgrades);
            }
            "policies" => {
                if let Some(policy) = write.policies.policies.get(town_idx) {
                    root["policies"] = json!(policy);
                }
            }
            _ => {}
        }
    }

    root
}

/// Parse CSV-line response — one action per line: `method, key:value, key:value, ...`
fn parse_actions(response: &str) -> Vec<LlmAction> {
    let text = response.trim();
    if text.is_empty() || text.eq_ignore_ascii_case("none") {
        return Vec::new();
    }
    text.lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() { return None; }
            let mut parts = line.splitn(2, ", ");
            let method = parts.next()?.trim().to_string();
            let mut params = HashMap::new();
            if let Some(rest) = parts.next() {
                // Special handling: message: grabs everything after it (may contain commas)
                if let Some(msg_idx) = rest.find("message:") {
                    let message_val = rest[msg_idx + 8..].trim().to_string();
                    params.insert("message".to_string(), message_val);
                    let prefix = &rest[..msg_idx];
                    for arg in prefix.split(", ") {
                        let arg = arg.trim();
                        if let Some((key, val)) = arg.split_once(':') {
                            params.insert(key.trim().to_string(), val.trim().to_string());
                        }
                    }
                } else {
                    for arg in rest.split(", ") {
                        let arg = arg.trim();
                        if let Some((key, val)) = arg.split_once(':') {
                            params.insert(key.trim().to_string(), val.trim().to_string());
                        }
                    }
                }
            }
            Some(LlmAction { method, params })
        })
        .collect()
}

#[derive(Debug)]
struct LlmAction {
    method: String,
    params: HashMap<String, String>,
}

/// Main system — timer-driven, spawns claude --print in background, polls results.
pub fn llm_player_system(
    mut state: ResMut<LlmPlayerState>,
    time: Res<Time>,
    read: LlmReadState,
    mut write: LlmWriteState,
    settings: Res<crate::settings::UserSettings>,
) {
    let town = state.town_idx;

    // Sync timer duration from settings so slider changes take effect live
    state.timer.set_duration(std::time::Duration::from_secs_f32(settings.llm_interval));

    // Poll pending result
    enum PollResult { None, Response(String), Waiting, Disconnected }
    let poll = if let Some(ref receiver_mutex) = state.receiver {
        let receiver = receiver_mutex.lock().unwrap();
        match receiver.try_recv() {
            Ok(response) => PollResult::Response(response),
            Err(mpsc::TryRecvError::Empty) => PollResult::Waiting,
            Err(mpsc::TryRecvError::Disconnected) => PollResult::Disconnected,
        }
    } else {
        PollResult::None
    };

    match poll {
        PollResult::Response(response) => {
            state.receiver = None;
            state.last_response = response.clone();
            let actions = parse_actions(&response);
            state.status = LlmStatus::Done(actions.len());
            execute_actions(&actions, town, &read, &mut write, &mut state);
            // Drain inbox after LLM has responded — not on send, so messages survive crashes
            write.chat_inbox.messages.retain(|m| m.to_town != town);
        }
        PollResult::Waiting => {
            state.status = LlmStatus::Thinking;
        }
        PollResult::Disconnected => {
            warn!("[LLM] Background thread disconnected");
            state.receiver = None;
            state.status = LlmStatus::Idle;
        }
        PollResult::None => {}
    }

    // Tick timer
    state.timer.tick(time.delta());
    if !state.timer.just_finished() {
        return;
    }
    if state.receiver.is_some() {
        return; // still waiting for previous response
    }
    if read.game_time.paused {
        return;
    }

    // Merge persistent subscriptions + one-shot queries (deduplicated)
    let one_shot = std::mem::take(&mut state.pending_queries);
    let mut topics: Vec<String> = state.subscriptions.clone();
    for t in one_shot {
        if !topics.contains(&t) {
            topics.push(t);
        }
    }
    let state_json = build_state_json(&read, &write, town, &topics);

    let prompt = state.prompt.clone();
    let toon_state = serde_toon2::to_string(&state_json).unwrap_or_default();
    let message = format!(
        "Current game state:\n\n{}\n\nRespond with one action per line: method, key:value, key:value. Or NONE if no action needed.",
        toon_state
    );

    // Store command + payload for the settings panel inspector
    state.last_command = "claude --print --model claude-haiku-4-5-20251001 --output-format text --system-prompt <prompt_builtin.md> --dangerously-skip-permissions".into();
    state.last_payload = message.clone();

    let (tx, rx) = mpsc::channel();
    state.receiver = Some(Mutex::new(rx));
    state.status = LlmStatus::Sending;

    std::thread::spawn(move || {
        use std::io::Write;
        #[cfg(target_os = "windows")]
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        let mut cmd = std::process::Command::new("claude");
        cmd.args([
                "--print",
                "--model", "claude-haiku-4-5-20251001",
                "--output-format", "text",
                "--system-prompt", &prompt,
                "--dangerously-skip-permissions",
            ])
            .env_remove("CLAUDECODE")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        #[cfg(target_os = "windows")]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }

        let child = cmd.spawn();

        match child {
            Ok(mut proc) => {
                if let Some(mut stdin) = proc.stdin.take() {
                    let _ = stdin.write_all(message.as_bytes());
                    // drop closes stdin, signaling EOF
                }
                match proc.wait_with_output() {
                    Ok(output) => {
                        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
                        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
                        if !output.status.success() || stdout.is_empty() {
                            let msg = format!("ERR exit={} stderr={}", output.status, &stderr[..stderr.len().min(300)]);
                            let _ = tx.send(msg);
                        } else {
                            let _ = tx.send(stdout);
                        }
                    }
                    Err(e) => { let _ = tx.send(format!("ERR wait: {e}")); }
                }
            }
            Err(e) => {
                let _ = tx.send(format!("ERR spawn: {e}"));
            }
        }
    });
}

/// Extract topic names from params — `topics:npcs,upgrades` → `["npcs", "upgrades"]`
fn extract_topics(params: &HashMap<String, String>) -> Vec<String> {
    params.get("topics")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default()
}

/// Execute parsed actions directly against ECS resources.
fn execute_actions(
    actions: &[LlmAction],
    town: usize,
    read: &LlmReadState,
    write: &mut LlmWriteState,
    state: &mut LlmPlayerState,
) {
    let faction = -1i32;
    let day = read.game_time.day();
    let hour = read.game_time.hour();
    let minute = read.game_time.minute();

    for action in actions {
        let p = &action.params;
        match action.method.as_str() {
            "policy" => {
                if let Some(policy) = write.policies.policies.get_mut(town) {
                    for (key, val) in p {
                        match key.as_str() {
                            "eat_food" => { if let Ok(v) = val.parse::<bool>() { policy.eat_food = v; } }
                            "archer_aggressive" => { if let Ok(v) = val.parse::<bool>() { policy.archer_aggressive = v; } }
                            "archer_leash" => { if let Ok(v) = val.parse::<bool>() { policy.archer_leash = v; } }
                            "farmer_fight_back" => { if let Ok(v) = val.parse::<bool>() { policy.farmer_fight_back = v; } }
                            "prioritize_healing" => { if let Ok(v) = val.parse::<bool>() { policy.prioritize_healing = v; } }
                            "farmer_flee_hp" => { if let Ok(v) = val.parse::<f32>() { policy.farmer_flee_hp = v.clamp(0.0, 1.0); } }
                            "archer_flee_hp" => { if let Ok(v) = val.parse::<f32>() { policy.archer_flee_hp = v.clamp(0.0, 1.0); } }
                            "recovery_hp" => { if let Ok(v) = val.parse::<f32>() { policy.recovery_hp = v.clamp(0.0, 1.0); } }
                            "mining_radius" => { if let Ok(v) = val.parse::<f32>() { policy.mining_radius = v.clamp(0.0, 5000.0); } }
                            _ => {}
                        }
                    }
                    write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                        format!("[llm] policy: {:?}", p));
                }
            }
            "build" => {
                let kind_str = p.get("kind").map(|s| s.as_str()).unwrap_or("");
                let row = p.get("row").and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
                let col = p.get("col").and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
                if let Some(kind) = crate::systems::remote::parse_building_kind(kind_str) {
                    write.build_q.0.push(crate::systems::remote::RemoteBuild {
                        town, kind, row, col,
                    });
                    write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                        format!("[llm] build {} at ({},{})", kind_str, row, col));
                }
            }
            "destroy" => {
                let row = p.get("row").and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
                let col = p.get("col").and_then(|s| s.parse::<i32>().ok()).unwrap_or(0);
                write.destroy_q.0.push(crate::systems::remote::RemoteDestroy { town, row, col });
                write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                    format!("[llm] destroy at ({},{})", row, col));
            }
            "upgrade" => {
                let idx = p.get("upgrade_idx").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                write.upgrade_q.0.push(crate::systems::remote::RemoteUpgrade {
                    town, upgrade_idx: idx,
                });
                write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                    format!("[llm] upgrade idx {}", idx));
            }
            "squad_target" => {
                let squad_idx = p.get("squad").and_then(|s| s.parse::<usize>().ok());
                let x = p.get("x").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                let y = p.get("y").and_then(|s| s.parse::<f32>().ok()).unwrap_or(0.0);
                if let Some(si) = squad_idx {
                    let owned = write.squad_state.squads.get(si).map(|sq| match sq.owner {
                        SquadOwner::Player => 0,
                        SquadOwner::Town(tdi) => tdi,
                    } == town).unwrap_or(false);
                    if owned {
                        write.squad_state.squads[si].target = Some(bevy::math::Vec2::new(x, y));
                        write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                            format!("[llm] squad_target squad={} x={} y={}", si, x, y));
                    }
                }
            }
            "chat" => {
                let to = p.get("to").and_then(|s| s.parse::<usize>().ok()).unwrap_or(0);
                let message = p.get("message").cloned().unwrap_or_default();
                if !message.is_empty() {
                    write.chat_inbox.messages.push(ChatMessage {
                        from_town: town,
                        to_town: to,
                        text: message.clone(),
                        day, hour, minute,
                    });
                    write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                        format!("[llm] chat to town {}: {}", to, message));
                }
            }
            "query" => {
                let topics = extract_topics(p);
                write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                    format!("[llm] query: {:?}", topics));
                state.pending_queries.extend(topics);
            }
            "subscribe" => {
                let topics = extract_topics(p);
                for t in &topics {
                    if !state.subscriptions.contains(t) {
                        state.subscriptions.push(t.clone());
                    }
                }
                write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                    format!("[llm] subscribed: {:?}", state.subscriptions));
            }
            "unsubscribe" => {
                let topics = extract_topics(p);
                state.subscriptions.retain(|s| !topics.contains(s));
                write.combat_log.push(CombatEventKind::Llm, faction, day, hour, minute,
                    format!("[llm] unsubscribed, remaining: {:?}", state.subscriptions));
            }
            other => {
                warn!("[LLM] Unknown action: {other}");
            }
        }
    }
}
