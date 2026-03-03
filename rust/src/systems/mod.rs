//! Bevy ECS Systems - Game logic that operates on components

pub mod ai_player;
pub mod audio;
mod behavior;
mod combat;
mod drain;
mod economy;
mod energy;
mod health;
mod movement;
pub mod pathfinding;
pub(crate) mod spawn;
pub mod stats;
pub use ai_player::{
    AiKind, AiPersonality, AiPlayer, AiPlayerConfig, AiPlayerState, ai_decision_system,
    ai_squad_commander_system, rebuild_squad_indices, sync_patrol_perimeter_system,
};
pub use behavior::*;
pub use combat::*;
pub use drain::*;
pub use economy::*;
pub use energy::*;
pub use health::*;
pub use movement::*;
pub use spawn::*;
pub use stats::{
    CombatConfig, TownUpgrades, UPGRADES, UpgradeMsg, auto_upgrade_system, expansion_cost,
    level_from_xp, process_upgrades_system, resolve_combat_stats, resolve_tower_instance_stats,
    upgrade_cost, upgrade_count,
};
