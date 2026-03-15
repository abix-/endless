//! AI squad commander -- wave-based attack cycle for both Builder and Raider AIs.

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use rand::Rng;

use crate::components::{Building, Dead, Job, TownId};
use crate::constants::*;
use crate::resources::*;
use crate::world::{BuildingKind, WorldData};

use super::{
    AGGRESSIVE_RETARGET_COOLDOWN, AI_ATTACK_SEARCH_RADIUS, AiKind, AiPersonality, AiPlayer,
    AiPlayerState, AiSquadCmdState, BALANCED_RETARGET_COOLDOWN, ECONOMIC_RETARGET_COOLDOWN,
    RETARGET_JITTER, SquadRole,
};

// ============================================================================
// AI SQUAD COMMANDER
// ============================================================================

/// Resolve a building's position by slot. Returns None if slot has no instance (dead/freed).
fn resolve_building_pos(entity_map: &EntityMap, uid: Entity) -> Option<Vec2> {
    entity_map.instance_by_entity(uid).map(|inst| inst.position)
}

impl AiPersonality {
    fn retarget_cooldown(self) -> f32 {
        match self {
            Self::Aggressive => AGGRESSIVE_RETARGET_COOLDOWN,
            Self::Balanced => BALANCED_RETARGET_COOLDOWN,
            Self::Economic => ECONOMIC_RETARGET_COOLDOWN,
        }
    }

    fn attack_squad_count(self) -> usize {
        match self {
            Self::Aggressive => 2,
            Self::Balanced => 1,
            Self::Economic => 1,
        }
    }

    fn desired_squad_count(self) -> usize {
        1 + self.attack_squad_count()
    }

    /// Percent of town archers kept in squad[0] as patrol/defense.
    fn defense_share_pct(self) -> usize {
        match self {
            Self::Aggressive => 25,
            Self::Balanced => 45,
            Self::Economic => 65,
        }
    }

    /// Relative split for each attack squad (index within attack squads only).
    fn attack_split_weight(self, attack_idx: usize) -> usize {
        match self {
            Self::Aggressive => {
                if attack_idx == 0 {
                    55
                } else {
                    45
                }
            }
            Self::Balanced => 100,
            Self::Economic => 100,
        }
    }

    /// Preferred building kinds to attack, by personality and squad role.
    fn attack_kinds(self, role: SquadRole) -> &'static [BuildingKind] {
        match role {
            SquadRole::Reserve | SquadRole::Idle => &[], // non-attack squads don't attack
            SquadRole::Attack => match self {
                Self::Aggressive => &[
                    BuildingKind::Farm,
                    BuildingKind::FarmerHome,
                    BuildingKind::ArcherHome,
                    BuildingKind::CrossbowHome,
                    BuildingKind::Waypoint,
                    BuildingKind::Tent,
                    BuildingKind::MinerHome,
                ],
                Self::Balanced => &[
                    BuildingKind::ArcherHome,
                    BuildingKind::CrossbowHome,
                    BuildingKind::Waypoint,
                ],
                Self::Economic => &[BuildingKind::Farm],
            },
        }
    }

    /// Broad fallback set when preferred kinds yield no target.
    /// Fountain last priority -- destroy the base after clearing defenses.
    fn fallback_attack_kinds() -> &'static [BuildingKind] {
        &[
            BuildingKind::Farm,
            BuildingKind::FarmerHome,
            BuildingKind::ArcherHome,
            BuildingKind::CrossbowHome,
            BuildingKind::Waypoint,
            BuildingKind::Tent,
            BuildingKind::MinerHome,
            BuildingKind::Fountain,
        ]
    }

    /// Minimum members before a wave can start.
    fn wave_min_start(self, kind: AiKind) -> usize {
        match kind {
            AiKind::Raider => RAID_GROUP_SIZE as usize,
            AiKind::Builder => match self {
                Self::Aggressive => 3,
                Self::Balanced => 5,
                Self::Economic => 8,
            },
        }
    }

    /// Loss threshold percent -- end wave when alive drops below this % of wave_start_count.
    fn wave_retreat_pct(self, kind: AiKind) -> usize {
        match kind {
            AiKind::Raider => 30,
            AiKind::Builder => match self {
                Self::Aggressive => 25,
                Self::Balanced => 40,
                Self::Economic => 60,
            },
        }
    }
}

/// Pick nearest enemy farm as raider squad target.
fn pick_raider_farm_target(
    entity_map: &EntityMap,
    center: Vec2,
    faction: i32,
) -> Option<(BuildingKind, Entity, Vec2)> {
    let mut best_d2 = f32::MAX;
    let mut result: Option<(BuildingKind, Entity, Vec2)> = None;
    let r2 = AI_ATTACK_SEARCH_RADIUS * AI_ATTACK_SEARCH_RADIUS;
    entity_map.for_each_nearby_kind(
        center,
        AI_ATTACK_SEARCH_RADIUS,
        BuildingKind::Farm,
        |inst, _| {
            if inst.faction == faction || inst.faction == crate::constants::FACTION_NEUTRAL {
                return;
            }
            let Some(&uid) = entity_map.entities.get(&inst.slot) else {
                return;
            };
            let dx = inst.position.x - center.x;
            let dy = inst.position.y - center.y;
            let d2 = dx * dx + dy * dy;
            if d2 <= r2 && d2 < best_d2 {
                best_d2 = d2;
                result = Some((inst.kind, uid, inst.position));
            }
        },
    );
    result
}

fn pick_ai_target_unclaimed(
    entity_map: &EntityMap,
    center: Vec2,
    faction: i32,
    personality: AiPersonality,
    role: SquadRole,
    claimed: &HashSet<Entity>,
) -> Option<(BuildingKind, Entity, Vec2)> {
    if role != SquadRole::Attack {
        return None;
    }

    let find_nearest_unclaimed =
        |allowed_kinds: &[BuildingKind]| -> Option<(BuildingKind, Entity, Vec2)> {
            let mut best_d2 = f32::MAX;
            let mut result: Option<(BuildingKind, Entity, Vec2)> = None;
            let r2 = AI_ATTACK_SEARCH_RADIUS * AI_ATTACK_SEARCH_RADIUS;
            for &kind in allowed_kinds {
                entity_map.for_each_nearby_kind(
                    center,
                    AI_ATTACK_SEARCH_RADIUS,
                    kind,
                    |inst, _| {
                        if inst.faction == faction
                            || inst.faction == crate::constants::FACTION_NEUTRAL
                        {
                            return;
                        }
                        let Some(&uid) = entity_map.entities.get(&inst.slot) else {
                            return;
                        };
                        if claimed.contains(&uid) {
                            return;
                        }
                        let dx = inst.position.x - center.x;
                        let dy = inst.position.y - center.y;
                        let d2 = dx * dx + dy * dy;
                        if d2 <= r2 && d2 < best_d2 {
                            best_d2 = d2;
                            result = Some((inst.kind, uid, inst.position));
                        }
                    },
                );
            }
            result
        };

    let preferred = personality.attack_kinds(role);
    find_nearest_unclaimed(preferred)
        .or_else(|| find_nearest_unclaimed(AiPersonality::fallback_attack_kinds()))
}

/// Rebuild squad_indices for one AI player by scanning SquadState ownership.
pub fn rebuild_squad_indices(player: &mut AiPlayer, squads: &[Squad]) {
    player.squad_indices.clear();
    for (i, s) in squads.iter().enumerate() {
        if s.owner == SquadOwner::Town(player.town_data_idx) {
            player.squad_indices.push(i);
        }
    }
}

/// AI squad commander -- wave-based attack cycle for both Builder and Raider AIs.
/// Sets shared squad knobs: target, target_size, patrol_enabled, rest_when_tired.
/// Wave model: gather -> threshold -> dispatch -> detect end -> reset.
pub fn ai_squad_commander_system(
    time: Res<Time>,
    mut ai_state: ResMut<AiPlayerState>,
    mut squad_state: ResMut<SquadState>,
    world_data: Res<WorldData>,
    entity_map: Res<EntityMap>,
    mut combat_log: MessageWriter<crate::messages::CombatLogMsg>,
    game_time: Res<GameTime>,
    mut squads_dirty_w: MessageWriter<crate::messages::SquadsDirtyMsg>,
    mut timer: Local<f32>,
    military_q: Query<(&Job, &TownId), (Without<Building>, Without<Dead>)>,
) {
    const AI_SQUAD_HEARTBEAT: f32 = 2.0;
    let dt = game_time.delta(&time);
    *timer += dt;
    if *timer < AI_SQUAD_HEARTBEAT {
        return;
    }
    let elapsed = *timer;
    *timer = 0.0;

    // Count alive military units per town.
    let mut units_by_town: HashMap<i32, usize> = HashMap::new();
    for (job, town_id) in military_q.iter() {
        if !job.is_military() {
            continue;
        }
        *units_by_town.entry(town_id.0).or_default() += 1;
    }

    for pi in 0..ai_state.players.len() {
        let player = &ai_state.players[pi];
        if !player.active {
            continue;
        }

        let tdi = player.town_data_idx;
        let personality = player.personality;
        let kind = player.kind;
        let Some(town) = world_data.towns.get(tdi) else {
            continue;
        };
        let center = town.center;
        let faction = town.faction;

        // --- Self-healing squad allocation ---
        let desired = match kind {
            AiKind::Builder => personality.desired_squad_count(),
            AiKind::Raider => 1, // single attack squad for raider towns
        };
        let owned: usize = squad_state
            .squads
            .iter()
            .filter(|s| s.owner == SquadOwner::Town(tdi))
            .count();
        if owned < desired {
            for _ in owned..desired {
                let idx = squad_state.alloc_squad(SquadOwner::Town(tdi));
                let base_cd = personality.retarget_cooldown();
                let jitter = rand::rng().random_range(0.3..1.0);
                let sq = squad_state
                    .squads
                    .get_mut(idx)
                    .expect("squad just allocated");
                sq.wave_min_start = personality.wave_min_start(kind);
                sq.wave_retreat_below_pct = personality.wave_retreat_pct(kind);
                ai_state.players[pi].squad_cmd.insert(
                    idx,
                    AiSquadCmdState {
                        building_uid: None,
                        cooldown: base_cd * jitter,
                    },
                );
            }
        }

        // Rebuild squad_indices from ownership scan.
        rebuild_squad_indices(&mut ai_state.players[pi], &squad_state.squads);
        let squad_indices = ai_state.players[pi].squad_indices.clone();
        if squad_indices.is_empty() {
            continue;
        }

        // --- Set target_size per squad ---
        let unit_count = units_by_town.get(&(tdi as i32)).copied().unwrap_or(0);

        match kind {
            AiKind::Raider => {
                // Raider towns: single squad gets all raiders
                if let Some(&si) = squad_indices.first() {
                    if let Some(squad) = squad_state.squads.get_mut(si) {
                        let new_size = unit_count;
                        if squad.target_size != new_size {
                            squad.target_size = new_size;
                            squads_dirty_w.write(crate::messages::SquadsDirtyMsg);
                        }
                        squad.patrol_enabled = false;
                        squad.rest_when_tired = false;
                    }
                }
            }
            AiKind::Builder => {
                // Builder AIs: defense + attack split
                let attack_squads = personality.attack_squad_count();
                let defense_size = unit_count * personality.defense_share_pct() / 100;
                let attack_total = unit_count.saturating_sub(defense_size);
                let total_attack_weight: usize = (0..attack_squads)
                    .map(|i| personality.attack_split_weight(i))
                    .sum::<usize>()
                    .max(1);

                for (role_idx, &si) in squad_indices.iter().enumerate() {
                    let role = if role_idx == 0 {
                        SquadRole::Reserve
                    } else if role_idx - 1 < attack_squads {
                        SquadRole::Attack
                    } else {
                        SquadRole::Idle
                    };

                    let new_target_size = match role {
                        SquadRole::Reserve => defense_size,
                        SquadRole::Attack => {
                            let attack_idx = role_idx - 1;
                            if attack_idx + 1 == attack_squads {
                                let allocated_before: usize = (0..attack_idx)
                                    .map(|i| {
                                        attack_total * personality.attack_split_weight(i)
                                            / total_attack_weight
                                    })
                                    .sum();
                                attack_total.saturating_sub(allocated_before)
                            } else {
                                attack_total * personality.attack_split_weight(attack_idx)
                                    / total_attack_weight
                            }
                        }
                        SquadRole::Idle => 0,
                    };

                    if let Some(squad) = squad_state.squads.get_mut(si) {
                        if squad.target_size != new_target_size {
                            squad.target_size = new_target_size;
                            squads_dirty_w.write(crate::messages::SquadsDirtyMsg);
                        }
                        let should_patrol = role == SquadRole::Reserve;
                        if squad.patrol_enabled != should_patrol {
                            squad.patrol_enabled = should_patrol;
                        }
                        if role != SquadRole::Attack && squad.target.is_some() {
                            squad.target = None;
                            squad.wave_active = false;
                        }
                        if !squad.rest_when_tired {
                            squad.rest_when_tired = true;
                        }
                    }
                }
            }
        }

        // --- Wave-based retarget for all attack squads ---
        let mut claimed_targets: HashSet<Entity> = HashSet::new();
        for &si in &squad_indices {
            let cmd = ai_state.players[pi].squad_cmd.entry(si).or_default();
            if cmd.cooldown > 0.0 {
                cmd.cooldown -= elapsed;
            }

            let Some(squad) = squad_state.squads.get(si) else {
                continue;
            };

            // Determine if this squad is an attack squad
            let is_attack = match kind {
                AiKind::Raider => true, // raider town squads always attack
                AiKind::Builder => {
                    let role_idx = squad_indices.iter().position(|&i| i == si).unwrap_or(0);
                    let attack_squads = personality.attack_squad_count();
                    role_idx >= 1 && role_idx - 1 < attack_squads
                }
            };
            if !is_attack {
                cmd.building_uid = None;
                continue;
            }

            let member_count = squad.members.len();

            if squad.wave_active {
                // --- Wave end conditions ---
                let target_alive = cmd
                    .building_uid
                    .and_then(|uid| resolve_building_pos(&entity_map, uid))
                    .is_some();

                let loss_threshold = squad.wave_start_count * squad.wave_retreat_below_pct / 100;
                let heavy_losses = member_count < loss_threshold.max(1);

                if !target_alive || heavy_losses {
                    // End wave -- clear target, reset to gathering
                    let reason = if !target_alive {
                        "target cleared"
                    } else {
                        "heavy losses"
                    };
                    let squad = squad_state.squads.get_mut(si).expect("squad index valid");
                    squad.wave_active = false;
                    squad.target = None;
                    squad.wave_start_count = 0;
                    cmd.building_uid = None;
                    cmd.cooldown = personality.retarget_cooldown()
                        + rand::rng().random_range(-RETARGET_JITTER..RETARGET_JITTER);

                    let town_name = &town.name;
                    let pname = personality.name();
                    combat_log.write(crate::messages::CombatLogMsg {
                        kind: CombatEventKind::Raid,
                        faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!(
                            "{} [{}] wave ended ({}), {} remaining",
                            town_name, pname, reason, member_count
                        ),
                        location: None,
                    });
                }
            } else {
                // --- Gathering phase: wait for wave_min_start ---
                let min_start = squad.wave_min_start.max(1);
                if member_count < min_start || cmd.cooldown > 0.0 {
                    continue; // not enough members or cooldown active
                }

                // Pick target based on AI kind
                let target = match kind {
                    AiKind::Raider => pick_raider_farm_target(&entity_map, center, faction),
                    AiKind::Builder => pick_ai_target_unclaimed(
                        &entity_map,
                        center,
                        faction,
                        personality,
                        SquadRole::Attack,
                        &claimed_targets,
                    ),
                };

                if let Some((bk, uid, pos)) = target {
                    cmd.building_uid = Some(uid);
                    claimed_targets.insert(uid);

                    let squad = squad_state.squads.get_mut(si).expect("squad index valid");
                    squad.target = Some(pos);
                    squad.wave_active = true;
                    squad.wave_start_count = member_count;

                    let town_name = &town.name;
                    let pname = personality.name();
                    let unit_label = match kind {
                        AiKind::Raider => "raiders",
                        AiKind::Builder => "units",
                    };
                    combat_log.write(crate::messages::CombatLogMsg {
                        kind: CombatEventKind::Raid,
                        faction,
                        day: game_time.day(),
                        hour: game_time.hour(),
                        minute: game_time.minute(),
                        message: format!(
                            "{} [{}] wave started: {} {} -> {}",
                            town_name,
                            pname,
                            member_count,
                            unit_label,
                            crate::constants::building_def(bk).label
                        ),
                        location: Some(pos),
                    });
                }
            }
        }
    }
}
