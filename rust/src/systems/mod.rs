//! Bevy ECS Systems - Game logic that operates on components

mod drain;
mod spawn;
mod movement;
mod energy;
mod behavior;
mod health;
mod combat;
mod economy;
pub mod stats;
pub mod ai_player;
pub use drain::*;
pub use spawn::*;
pub use movement::*;
pub use energy::*;
pub use behavior::*;
pub use health::*;
pub use combat::*;
pub use economy::*;
pub use stats::{CombatConfig, TownUpgrades, UpgradeQueue, UpgradeType, UPGRADE_PCT, UPGRADE_COUNT, resolve_combat_stats, level_from_xp, upgrade_cost, process_upgrades_system, auto_upgrade_system, xp_grant_system};
pub use ai_player::{AiPlayerConfig, AiPlayerState, AiPlayer, AiKind, AiPersonality, ai_decision_system};
