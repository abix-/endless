//! Bevy ECS Systems - Game logic that operates on components

mod drain;
pub(crate) mod spawn;
mod movement;
mod energy;
mod behavior;
mod health;
mod combat;
mod economy;
pub mod stats;
pub mod ai_player;
pub mod audio;
pub use drain::*;
pub use spawn::*;
pub use movement::*;
pub use energy::*;
pub use behavior::*;
pub use health::*;
pub use combat::*;
pub use economy::*;
pub use stats::{CombatConfig, TownUpgrades, UpgradeQueue, UPGRADES, upgrade_count, resolve_combat_stats, level_from_xp, upgrade_cost, expansion_cost, process_upgrades_system, auto_upgrade_system, xp_grant_system};
pub use ai_player::{AiPlayerConfig, AiPlayerState, AiPlayer, AiKind, AiPersonality, ai_decision_system, ai_squad_commander_system, rebuild_squad_indices, sync_patrol_perimeter_system};
