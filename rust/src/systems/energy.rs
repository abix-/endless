//! Energy systems - Drain and recovery

use bevy::prelude::*;

use crate::components::{Activity, Building, CachedStats, Dead, Energy, GpuSlot};
use crate::resources::GameTime;

/// Energy recovery/drain rates (per game hour)
const ENERGY_RECOVER_PER_HOUR: f32 = 100.0 / 6.0; // 6 hours to full (resting)
const ENERGY_DRAIN_PER_HOUR: f32 = 100.0 / 24.0; // 24 hours to empty (active)

/// Energy system: drain while active, recover while resting or healing at fountain.
/// Uses game time so it respects time_scale.
/// State transitions (wake-up, stop working) are handled in decision_system.
pub fn energy_system(
    time: Res<Time>,
    game_time: Res<GameTime>,
    mut energy_q: Query<(&GpuSlot, &mut Energy, &Activity, &CachedStats), (Without<Building>, Without<Dead>)>,
) {
    if game_time.is_paused() {
        return;
    }

    // Convert delta to game hours
    let hours_elapsed = game_time.delta(&time) / game_time.seconds_per_hour;

    for (_es, mut energy, activity, stats) in energy_q.iter_mut() {
        if matches!(
            *activity,
            Activity::Resting | Activity::HealingAtFountain { .. }
        ) {
            energy.0 = (energy.0 + ENERGY_RECOVER_PER_HOUR * hours_elapsed).min(100.0);
        } else {
            energy.0 = (energy.0 - ENERGY_DRAIN_PER_HOUR * stats.stamina * hours_elapsed).max(0.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;

    fn test_cached_stats() -> CachedStats {
        CachedStats {
            damage: 15.0, range: 200.0, cooldown: 1.5,
            projectile_speed: 200.0, projectile_lifetime: 1.5,
            max_health: 100.0, speed: 200.0, stamina: 1.0,
            hp_regen: 0.0, berserk_bonus: 0.0,
        }
    }

    /// Build a test app with energy_system on FixedUpdate, matching real game.
    /// Two priming updates: first initializes Time, second accumulates enough
    /// delta for FixedUpdate to trigger. After setup, entities are spawned and
    /// one more update() runs the system with a real time step.
    fn setup_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(GameTime::default());
        app.insert_resource(TimeUpdateStrategy::ManualDuration(
            std::time::Duration::from_secs_f32(1.0),
        ));
        app.add_systems(FixedUpdate, energy_system);
        // Prime time so FixedUpdate accumulates delta
        app.update();
        app.update();
        app
    }

    fn spawn_npc(app: &mut App, activity: Activity, energy: f32) -> Entity {
        app.world_mut().spawn((
            GpuSlot(0),
            Energy(energy),
            activity,
            test_cached_stats(),
        )).id()
    }

    #[test]
    fn energy_drains_while_working() {
        let mut app = setup_app();
        let npc = spawn_npc(&mut app, Activity::Working, 100.0);

        app.update();
        let energy = app.world().get::<Energy>(npc).unwrap().0;
        assert!(energy < 100.0, "energy should drain while working: {energy}");
    }

    #[test]
    fn energy_recovers_while_resting() {
        let mut app = setup_app();
        let npc = spawn_npc(&mut app, Activity::Resting, 50.0);

        app.update();
        let energy = app.world().get::<Energy>(npc).unwrap().0;
        assert!(energy > 50.0, "energy should recover while resting: {energy}");
    }

    #[test]
    fn energy_recovers_while_healing_at_fountain() {
        let mut app = setup_app();
        let npc = spawn_npc(&mut app, Activity::HealingAtFountain { recover_until: 100.0 }, 50.0);

        app.update();
        let energy = app.world().get::<Energy>(npc).unwrap().0;
        assert!(energy > 50.0, "energy should recover at fountain: {energy}");
    }

    #[test]
    fn energy_capped_at_100() {
        let mut app = setup_app();
        let npc = spawn_npc(&mut app, Activity::Resting, 99.9);

        for _ in 0..100 {
            app.update();
        }
        let energy = app.world().get::<Energy>(npc).unwrap().0;
        assert!(energy <= 100.0, "energy should not exceed 100: {energy}");
    }

    #[test]
    fn energy_floored_at_0() {
        let mut app = setup_app();
        let npc = spawn_npc(&mut app, Activity::Working, 0.1);

        for _ in 0..1000 {
            app.update();
        }
        let energy = app.world().get::<Energy>(npc).unwrap().0;
        assert!(energy >= 0.0, "energy should not go negative: {energy}");
    }

    #[test]
    fn energy_paused_no_change() {
        let mut app = setup_app();
        app.world_mut().resource_mut::<GameTime>().paused = true;
        let npc = spawn_npc(&mut app, Activity::Working, 75.0);

        app.update();
        let energy = app.world().get::<Energy>(npc).unwrap().0;
        assert!((energy - 75.0).abs() < f32::EPSILON, "energy should not change when paused: {energy}");
    }

    #[test]
    fn stamina_multiplier_affects_drain() {
        let mut app = setup_app();

        // NPC with stamina 1.0 (normal)
        let npc_normal = app.world_mut().spawn((
            GpuSlot(0), Energy(100.0), Activity::Working, test_cached_stats(),
        )).id();

        // NPC with stamina 0.5 (slower drain)
        let mut slow_stats = test_cached_stats();
        slow_stats.stamina = 0.5;
        let npc_slow = app.world_mut().spawn((
            GpuSlot(1), Energy(100.0), Activity::Working, slow_stats,
        )).id();

        app.update();
        let e_normal = app.world().get::<Energy>(npc_normal).unwrap().0;
        let e_slow = app.world().get::<Energy>(npc_slow).unwrap().0;
        assert!(e_slow > e_normal, "lower stamina mult should drain slower: normal={e_normal}, slow={e_slow}");
    }

    #[test]
    fn dead_npcs_excluded() {
        let mut app = setup_app();
        let npc = app.world_mut().spawn((
            GpuSlot(0), Energy(100.0), Activity::Working, test_cached_stats(), Dead,
        )).id();

        app.update();
        let energy = app.world().get::<Energy>(npc).unwrap().0;
        assert!((energy - 100.0).abs() < f32::EPSILON, "dead NPC energy should not change: {energy}");
    }
}
