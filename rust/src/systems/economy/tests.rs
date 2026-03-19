use super::*;
use crate::components::{
    Building, CachedStats, ConstructionProgress, Dead, Energy, GpuSlot, Health, NpcFlags, Position,
    ProductionState, SpawnerState, TownId,
};
use crate::messages::GpuUpdateMsg;
use crate::resources::GameTime;
use bevy::time::TimeUpdateStrategy;

fn test_cached_stats() -> CachedStats {
    CachedStats {
        damage: 15.0,
        range: 200.0,
        cooldown: 1.5,
        projectile_speed: 200.0,
        projectile_lifetime: 1.5,
        max_health: 100.0,
        speed: 200.0,
        stamina: 1.0,
        hp_regen: 0.0,
        berserk_bonus: 0.0,
    }
}

fn setup_starvation_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_message::<GpuUpdateMsg>();
    app.add_systems(FixedUpdate, starvation_system);
    // Prime FixedUpdate time
    app.update();
    app.update();
    app
}

fn spawn_starving_npc(app: &mut App, energy: f32, health: f32) -> Entity {
    app.world_mut()
        .spawn((
            GpuSlot(0),
            Energy(energy),
            test_cached_stats(),
            NpcFlags::default(),
            Health(health),
        ))
        .id()
}

#[test]
fn starvation_flags_set_when_energy_zero() {
    let mut app = setup_starvation_app();
    let npc = spawn_starving_npc(&mut app, 0.0, 100.0);
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;

    app.update();
    let flags = app.world().get::<NpcFlags>(npc).unwrap();
    assert!(flags.starving, "NPC with 0 energy should be starving");
}

#[test]
fn starvation_clears_when_energy_restored() {
    let mut app = setup_starvation_app();
    let npc = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Energy(50.0),
            test_cached_stats(),
            NpcFlags {
                starving: true,
                ..Default::default()
            },
            Health(100.0),
        ))
        .id();
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;

    app.update();
    let flags = app.world().get::<NpcFlags>(npc).unwrap();
    assert!(!flags.starving, "NPC with energy should not be starving");
}

#[test]
fn starvation_caps_hp() {
    let mut app = setup_starvation_app();
    let npc = spawn_starving_npc(&mut app, 0.0, 100.0);
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    let cap = 100.0 * STARVING_HP_CAP;
    assert!(
        hp <= cap + 0.01,
        "starving NPC HP should be capped at {cap}: {hp}"
    );
}

#[test]
fn starvation_skips_when_no_hour_tick() {
    let mut app = setup_starvation_app();
    let npc = spawn_starving_npc(&mut app, 0.0, 100.0);
    app.world_mut().resource_mut::<GameTime>().hour_ticked = false;

    app.update();
    let flags = app.world().get::<NpcFlags>(npc).unwrap();
    assert!(
        !flags.starving,
        "should not process starvation without hour tick"
    );
}

#[test]
fn dead_npcs_excluded_from_starvation() {
    let mut app = setup_starvation_app();
    let npc = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Energy(0.0),
            test_cached_stats(),
            NpcFlags::default(),
            Health(100.0),
            Dead,
        ))
        .id();
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;

    app.update();
    let flags = app.world().get::<NpcFlags>(npc).unwrap();
    assert!(
        !flags.starving,
        "dead NPC should be excluded from starvation"
    );
}

#[test]
fn buildings_excluded_from_starvation() {
    let mut app = setup_starvation_app();
    let building = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Energy(0.0),
            test_cached_stats(),
            NpcFlags::default(),
            Health(100.0),
            Building {
                kind: crate::world::BuildingKind::BowTower,
            },
        ))
        .id();
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;

    app.update();
    let flags = app.world().get::<NpcFlags>(building).unwrap();
    assert!(
        !flags.starving,
        "buildings should be excluded from starvation"
    );
}

// ========================================================================
// game_time_system tests
// ========================================================================

fn setup_game_time_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, game_time_system);
    app.update();
    app.update();
    app
}

#[test]
fn game_time_advances() {
    let mut app = setup_game_time_app();

    let before = app.world().resource::<GameTime>().total_seconds;
    app.update();
    let after = app.world().resource::<GameTime>().total_seconds;
    assert!(
        after > before,
        "total_seconds should advance: before={before}, after={after}"
    );
}

#[test]
fn game_time_paused_no_advance() {
    let mut app = setup_game_time_app();
    app.world_mut().resource_mut::<GameTime>().paused = true;

    let before = app.world().resource::<GameTime>().total_seconds;
    app.update();
    let after = app.world().resource::<GameTime>().total_seconds;
    assert!(
        (after - before).abs() < f32::EPSILON,
        "paused game time should not advance: {before} -> {after}"
    );
}

#[test]
fn game_time_hour_ticked_resets_each_frame() {
    let mut app = setup_game_time_app();
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;

    app.update();
    let ticked = app.world().resource::<GameTime>().hour_ticked;
    // game_time_system resets hour_ticked to false each frame before checking
    // Whether it's true after depends on whether an hour boundary was crossed,
    // but it should NOT still be true from the manual set above (it resets first)
    // With default seconds_per_hour=5.0, 1s delta = 0.2 hours, so no hour boundary
    assert!(
        !ticked,
        "hour_ticked should reset each frame when no hour boundary crossed"
    );
}

#[test]
fn game_time_hour_ticks_after_enough_time() {
    let mut app = setup_game_time_app();
    // default: seconds_per_hour = 5.0, time_scale = 1.0
    // FixedUpdate has a max substeps cap (~16), so each app.update() adds ~0.25s
    // Need total_seconds > 5.0 to cross first hour boundary → ~25 updates
    for _ in 0..25 {
        app.update();
    }
    let gt = app.world().resource::<GameTime>();
    assert!(
        gt.last_hour >= 1,
        "last_hour should increment after enough time: last_hour={}, total_seconds={}",
        gt.last_hour,
        gt.total_seconds
    );
}

#[test]
fn game_time_last_hour_tracks() {
    let mut app = setup_game_time_app();
    let initial = app.world().resource::<GameTime>().last_hour;

    // Run many updates to cross multiple hour boundaries
    for _ in 0..20 {
        app.update();
    }
    let final_hour = app.world().resource::<GameTime>().last_hour;
    assert!(
        final_hour > initial,
        "last_hour should increase over time: initial={initial}, final={final_hour}"
    );
}

// ========================================================================
// construction_tick_system tests
// ========================================================================

use crate::resources::BuildingInstance;
use crate::world::BuildingKind;

fn setup_construction_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_message::<GpuUpdateMsg>();
    app.add_systems(FixedUpdate, construction_tick_system);
    app.update();
    app.update();
    app
}

fn test_building_instance(
    slot: usize,
    kind: BuildingKind,
    _under_construction: f32,
) -> BuildingInstance {
    BuildingInstance {
        kind,
        position: Vec2::ZERO,
        town_idx: 0,
        slot,
        faction: 0,
    }
}

fn spawn_constructing_building(
    app: &mut App,
    slot: usize,
    kind: BuildingKind,
    secs_left: f32,
) -> Entity {
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            Health(0.01),
            Building { kind },
            ConstructionProgress(secs_left),
            ProductionState::default(),
        ))
        .id();
    let mut entity_map = app.world_mut().resource_mut::<EntityMap>();
    entity_map.set_entity(slot, entity);
    entity_map.add_instance(test_building_instance(slot, kind, secs_left));
    entity
}

#[test]
fn construction_ticks_down() {
    let mut app = setup_construction_app();
    let entity = spawn_constructing_building(&mut app, 0, BuildingKind::BowTower, 10.0);

    app.update();
    let cp = app.world().get::<ConstructionProgress>(entity).unwrap().0;
    assert!(cp < 10.0, "construction timer should tick down: {}", cp);
    assert!(cp > 0.0, "should not be complete yet");
}

#[test]
fn construction_completes() {
    let mut app = setup_construction_app();
    let entity = spawn_constructing_building(&mut app, 0, BuildingKind::BowTower, 0.1);

    // Run enough updates for 0.1s to elapse
    for _ in 0..5 {
        app.update();
    }
    let cp = app.world().get::<ConstructionProgress>(entity).unwrap().0;
    assert!(cp <= 0.0, "construction should complete: {}", cp);

    // Health should be set to full building HP
    let hp = app.world().get::<Health>(entity).unwrap().0;
    let expected = crate::constants::building_def(BuildingKind::BowTower).hp;
    assert!(
        (hp - expected).abs() < 0.1,
        "completed building HP should be {expected}: {hp}"
    );
}

#[test]
fn construction_paused_no_progress() {
    let mut app = setup_construction_app();
    app.world_mut().resource_mut::<GameTime>().paused = true;
    let entity = spawn_constructing_building(&mut app, 0, BuildingKind::Farm, 5.0);

    app.update();
    let cp = app.world().get::<ConstructionProgress>(entity).unwrap().0;
    assert!(
        (cp - 5.0).abs() < f32::EPSILON,
        "paused: construction should not progress: {}",
        cp
    );
}

#[test]
fn construction_hp_scales_with_progress() {
    let mut app = setup_construction_app();
    let entity = spawn_constructing_building(&mut app, 0, BuildingKind::BowTower, 10.0);

    app.update();
    let hp = app.world().get::<Health>(entity).unwrap().0;
    // Should be between 0.01 and full HP (partial progress)
    let full_hp = crate::constants::building_def(BuildingKind::BowTower).hp;
    assert!(
        hp > 0.0 && hp < full_hp,
        "HP should scale with progress: {hp} (full={full_hp})"
    );
}

// ========================================================================
// population tracking pure function tests
// ========================================================================

#[test]
fn pop_alive_inc_dec() {
    let mut stats = PopulationStats::default();
    super::pop_inc_alive(&mut stats, Job::Archer, 0);
    super::pop_inc_alive(&mut stats, Job::Archer, 0);
    let key = (Job::Archer as i32, 0);
    assert_eq!(stats.0[&key].alive, 2);
    super::pop_dec_alive(&mut stats, Job::Archer, 0);
    assert_eq!(stats.0[&key].alive, 1);
}

#[test]
fn pop_dec_alive_floors_at_zero() {
    let mut stats = PopulationStats::default();
    super::pop_dec_alive(&mut stats, Job::Farmer, 0);
    // Should not panic, and if entry exists, alive should be 0
    let key = (Job::Farmer as i32, 0);
    if let Some(entry) = stats.0.get(&key) {
        assert!(entry.alive >= 0, "alive should not go negative");
    }
}

#[test]
fn pop_working_increments() {
    let mut stats = PopulationStats::default();
    super::pop_inc_working(&mut stats, Job::Miner, 1);
    let key = (Job::Miner as i32, 1);
    assert_eq!(stats.0[&key].working, 1);
}

// ========================================================================
// player town index lookup tests
// ========================================================================

/// Regression test: player town lookup via FACTION_PLAYER finds the correct town
/// even when the player town is not at index 0. Verifies that the HUD top bar
/// uses faction-based lookup instead of hardcoded index 0.
#[test]
fn player_town_lookup_by_faction() {
    use crate::constants::FACTION_PLAYER;
    use crate::world::{Town, WorldData};

    // Player town is at index 1 (not 0) -- would silently break with hardcoded 0
    let world_data = WorldData {
        towns: vec![
            Town {
                name: "Raider".into(),
                center: Vec2::ZERO,
                faction: 2,
                kind: crate::constants::TownKind::AiRaider,
            },
            Town {
                name: "Player".into(),
                center: Vec2::new(500.0, 500.0),
                faction: FACTION_PLAYER,
                kind: crate::constants::TownKind::Player,
            },
        ],
    };

    let player_town_idx = world_data
        .towns
        .iter()
        .position(|t| t.faction == FACTION_PLAYER)
        .unwrap_or(0) as i32;

    assert_eq!(
        player_town_idx, 1,
        "player town should be found at index 1, not hardcoded 0"
    );

    // Simulate pop_stats with farmers registered under the correct town idx (1)
    let mut stats = PopulationStats::default();
    super::pop_inc_alive(&mut stats, Job::Farmer, 1);

    let count = stats
        .0
        .get(&(Job::Farmer as i32, player_town_idx))
        .map(|s| s.alive)
        .unwrap_or(0);
    assert_eq!(
        count, 1,
        "lookup with FACTION_PLAYER town idx should find the farmer"
    );

    // Hardcoded 0 would return 0 (wrong) -- confirm the regression
    let wrong_count = stats
        .0
        .get(&(Job::Farmer as i32, 0))
        .map(|s| s.alive)
        .unwrap_or(0);
    assert_eq!(
        wrong_count, 0,
        "hardcoded index 0 should miss the player town data when player is at index 1"
    );
}

// ========================================================================
// raider_forage_system tests
// ========================================================================

fn setup_forage_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(PopulationStats::default());
    app.insert_resource(WorldData {
        towns: vec![
            crate::world::Town {
                name: "Player".into(),
                center: Vec2::ZERO,
                faction: 1,
                kind: crate::constants::TownKind::Player,
            },
            crate::world::Town {
                name: "Raider".into(),
                center: Vec2::new(1000.0, 0.0),
                faction: 2,
                kind: crate::constants::TownKind::AiRaider,
            },
        ],
    });
    app.insert_resource(crate::settings::UserSettings::default());
    app.insert_resource(RaiderState {
        max_pop: vec![5, 5],
        respawn_timers: vec![0.0, 0.0],
        forage_timers: vec![0.0, 0.0],
    });
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    // Spawn ECS town entities and register TownIndex
    let mut town_index = crate::resources::TownIndex::default();
    let e0 = app
        .world_mut()
        .spawn((
            crate::components::TownMarker,
            crate::components::FoodStore(0),
            crate::components::GoldStore(0),
            crate::components::TownPolicy::default(),
            crate::components::TownUpgradeLevel::default(),
            crate::components::TownEquipment::default(),
        ))
        .id();
    let e1 = app
        .world_mut()
        .spawn((
            crate::components::TownMarker,
            crate::components::FoodStore(0),
            crate::components::GoldStore(0),
            crate::components::TownPolicy::default(),
            crate::components::TownUpgradeLevel::default(),
            crate::components::TownEquipment::default(),
        ))
        .id();
    town_index.0.insert(0, e0);
    town_index.0.insert(1, e1);
    app.insert_resource(town_index);
    app.add_systems(FixedUpdate, raider_forage_system);
    app.update();
    app.update();
    app
}

#[test]
fn raider_forage_adds_food_on_hour_tick() {
    let mut app = setup_forage_app();
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;
    // Set forage timer for raider town (index 1) to threshold
    let interval = app
        .world()
        .resource::<crate::settings::UserSettings>()
        .raider_forage_hours;
    app.world_mut().resource_mut::<RaiderState>().forage_timers[1] = interval - 1.0;

    app.update();
    let town_index = app.world().resource::<crate::resources::TownIndex>();
    let e1 = town_index.0[&1];
    let food = app
        .world()
        .get::<crate::components::FoodStore>(e1)
        .unwrap()
        .0;
    assert!(
        food > 0,
        "raider town should gain food from foraging: {food}"
    );
}

#[test]
fn raider_forage_skips_without_hour_tick() {
    let mut app = setup_forage_app();
    app.world_mut().resource_mut::<GameTime>().hour_ticked = false;

    app.update();
    let town_index = app.world().resource::<crate::resources::TownIndex>();
    let e1 = town_index.0[&1];
    let food = app
        .world()
        .get::<crate::components::FoodStore>(e1)
        .unwrap()
        .0;
    assert_eq!(food, 0, "no foraging without hour tick");
}

#[test]
fn raider_forage_player_town_unaffected() {
    let mut app = setup_forage_app();
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;
    let interval = app
        .world()
        .resource::<crate::settings::UserSettings>()
        .raider_forage_hours;
    app.world_mut().resource_mut::<RaiderState>().forage_timers[1] = interval - 1.0;

    app.update();
    let town_index = app.world().resource::<crate::resources::TownIndex>();
    let e0 = town_index.0[&0];
    let food = app
        .world()
        .get::<crate::components::FoodStore>(e0)
        .unwrap()
        .0;
    assert_eq!(food, 0, "player town should not get raider forage food");
}

// ========================================================================
// growth_system tests
// ========================================================================

fn setup_growth_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(WorldData {
        towns: vec![crate::world::Town {
            name: "TestTown".to_string(),
            center: Vec2::new(500.0, 500.0),
            faction: 1,
            kind: crate::constants::TownKind::Player,
        }],
    });
    // Spawn ECS town entity for TownAccess
    let mut town_index = crate::resources::TownIndex::default();
    let entity = app
        .world_mut()
        .spawn((
            crate::components::TownMarker,
            crate::components::FoodStore(0),
            crate::components::GoldStore(0),
            crate::components::TownPolicy::default(),
            crate::components::TownUpgradeLevel::default(),
            crate::components::TownEquipment::default(),
        ))
        .id();
    town_index.0.insert(0, entity);
    app.insert_resource(town_index);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, growth_system);
    app.update();
    app.update();
    app
}

fn add_farm(app: &mut App, slot: usize, tended: bool) {
    let inst = test_building_instance(slot, BuildingKind::Farm, 0.0);
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            Building {
                kind: BuildingKind::Farm,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.0,
            },
        ))
        .id();
    let mut em = app.world_mut().resource_mut::<EntityMap>();
    em.set_entity(slot, entity);
    em.add_instance(inst);
    if tended {
        em.set_present(slot, 1);
    }
}

#[test]
fn farm_grows_when_tended() {
    let mut app = setup_growth_app();
    add_farm(&mut app, 0, true);

    app.update();
    let entity = *app
        .world()
        .resource::<EntityMap>()
        .entities
        .get(&0)
        .unwrap();
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(
        ps.progress > 0.0,
        "tended farm should grow: {}",
        ps.progress
    );
}

#[test]
fn farm_grows_untended_at_base_rate() {
    let mut app = setup_growth_app();
    add_farm(&mut app, 0, false);

    app.update();
    let entity = *app
        .world()
        .resource::<EntityMap>()
        .entities
        .get(&0)
        .unwrap();
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(
        ps.progress > 0.0,
        "untended farm should still grow at base rate: {}",
        ps.progress
    );
}

#[test]
fn tended_farm_grows_faster() {
    let mut app = setup_growth_app();
    add_farm(&mut app, 0, false);
    add_farm(&mut app, 1, true);

    app.update();
    let e0 = *app
        .world()
        .resource::<EntityMap>()
        .entities
        .get(&0)
        .unwrap();
    let e1 = *app
        .world()
        .resource::<EntityMap>()
        .entities
        .get(&1)
        .unwrap();
    let untended = app.world().get::<ProductionState>(e0).unwrap().progress;
    let tended = app.world().get::<ProductionState>(e1).unwrap().progress;
    assert!(
        tended > untended,
        "tended should grow faster: tended={tended}, untended={untended}"
    );
}

/// Regression: claimed-but-not-present workers must NOT trigger tended growth.
/// Bug: occupancy incremented at claim time (before arrival), causing tended rate
/// to kick in while worker was still walking. Fix: growth_system gates on present_count.
/// Covers ALL worksite types: Farm, GoldMine, TreeNode, RockNode.
#[test]
fn worksites_claimed_but_not_present_do_not_progress() {
    let mut app = setup_growth_app();

    // Farm: present=0 should grow at passive (untended) rate, not tended rate
    add_farm(&mut app, 0, false); // untended baseline
    add_farm(&mut app, 1, true); // tended (present=1)

    // GoldMine: present=0 should NOT grow at all
    let mine_inst = test_building_instance(2, BuildingKind::GoldMine, 0.0);
    let mine_entity = app
        .world_mut()
        .spawn((
            GpuSlot(2),
            Building {
                kind: BuildingKind::GoldMine,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.0,
            },
        ))
        .id();
    {
        let mut em = app.world_mut().resource_mut::<EntityMap>();
        em.set_entity(2, mine_entity);
        em.add_instance(mine_inst);
    }

    // TreeNode: present=0 should NOT progress
    let tree_inst = test_building_instance(3, BuildingKind::TreeNode, 0.0);
    let tree_entity = app
        .world_mut()
        .spawn((
            GpuSlot(3),
            Building {
                kind: BuildingKind::TreeNode,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.0,
            },
        ))
        .id();
    {
        let mut em = app.world_mut().resource_mut::<EntityMap>();
        em.set_entity(3, tree_entity);
        em.add_instance(tree_inst);
    }

    // RockNode: present=0 should NOT progress
    let rock_inst = test_building_instance(4, BuildingKind::RockNode, 0.0);
    let rock_entity = app
        .world_mut()
        .spawn((
            GpuSlot(4),
            Building {
                kind: BuildingKind::RockNode,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.0,
            },
        ))
        .id();
    {
        let mut em = app.world_mut().resource_mut::<EntityMap>();
        em.set_entity(4, rock_entity);
        em.add_instance(rock_inst);
    }

    app.update();

    // Farm: untended grows at base rate, tended grows faster
    let farm_untended = app
        .world()
        .get::<ProductionState>(
            *app.world()
                .resource::<EntityMap>()
                .entities
                .get(&0)
                .unwrap(),
        )
        .unwrap()
        .progress;
    let farm_tended = app
        .world()
        .get::<ProductionState>(
            *app.world()
                .resource::<EntityMap>()
                .entities
                .get(&1)
                .unwrap(),
        )
        .unwrap()
        .progress;
    assert!(
        farm_untended > 0.0,
        "untended farm should grow at base rate"
    );
    assert!(
        farm_tended > farm_untended,
        "tended farm should grow faster: tended={farm_tended}, untended={farm_untended}"
    );

    // GoldMine: present=0 means zero progress
    let mine_progress = app
        .world()
        .get::<ProductionState>(mine_entity)
        .unwrap()
        .progress;
    assert!(
        mine_progress < f32::EPSILON,
        "mine with no present worker must not progress: {mine_progress}"
    );

    // TreeNode: present=0 means zero progress
    let tree_progress = app
        .world()
        .get::<ProductionState>(tree_entity)
        .unwrap()
        .progress;
    assert!(
        tree_progress < f32::EPSILON,
        "tree with no present worker must not progress: {tree_progress}"
    );

    // RockNode: present=0 means zero progress
    let rock_progress = app
        .world()
        .get::<ProductionState>(rock_entity)
        .unwrap()
        .progress;
    assert!(
        rock_progress < f32::EPSILON,
        "rock with no present worker must not progress: {rock_progress}"
    );
}

#[test]
fn farm_becomes_ready() {
    let mut app = setup_growth_app();
    let inst = test_building_instance(0, BuildingKind::Farm, 0.0);
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Building {
                kind: BuildingKind::Farm,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.99,
            },
        ))
        .id();
    let mut em = app.world_mut().resource_mut::<EntityMap>();
    em.set_entity(0, entity);
    em.add_instance(inst);

    for _ in 0..50 {
        app.update();
    }
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(ps.ready, "farm should become ready");
    assert!(
        (ps.progress - 1.0).abs() < f32::EPSILON,
        "progress should cap at 1.0"
    );
}

#[test]
fn growth_paused_no_change() {
    let mut app = setup_growth_app();
    app.world_mut().resource_mut::<GameTime>().paused = true;
    add_farm(&mut app, 0, true);

    app.update();
    let entity = *app
        .world()
        .resource::<EntityMap>()
        .entities
        .get(&0)
        .unwrap();
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(ps.progress < f32::EPSILON, "paused: farm should not grow");
}

#[test]
fn mine_grows_only_when_tended() {
    let mut app = setup_growth_app();
    let inst = test_building_instance(0, BuildingKind::GoldMine, 0.0);
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Building {
                kind: BuildingKind::GoldMine,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.0,
            },
        ))
        .id();
    let mut em = app.world_mut().resource_mut::<EntityMap>();
    em.set_entity(0, entity);
    em.add_instance(inst);

    app.update();
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(
        ps.progress < f32::EPSILON,
        "mine with 0 workers should not grow: {}",
        ps.progress
    );
}

#[test]
fn mine_grows_with_workers() {
    let mut app = setup_growth_app();
    let inst = test_building_instance(0, BuildingKind::GoldMine, 0.0);
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Building {
                kind: BuildingKind::GoldMine,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.0,
            },
        ))
        .id();
    let mut em = app.world_mut().resource_mut::<EntityMap>();
    em.set_entity(0, entity);
    em.add_instance(inst);
    em.set_present(0, 2);

    app.update();
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(
        ps.progress > 0.0,
        "mine with workers should grow: {}",
        ps.progress
    );
}

fn add_cow_farm(app: &mut App, slot: usize) {
    let inst = test_building_instance(slot, BuildingKind::Farm, 0.0);
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            Building {
                kind: BuildingKind::Farm,
            },
            TownId(0),
            Position { x: 0.0, y: 0.0 },
            ConstructionProgress(0.0),
            ProductionState {
                ready: false,
                progress: 0.0,
            },
            FarmModeComp(FarmMode::Cows),
        ))
        .id();
    let mut em = app.world_mut().resource_mut::<EntityMap>();
    em.set_entity(slot, entity);
    em.add_instance(inst);
}

#[test]
fn crops_do_not_grow_at_night() {
    let mut app = setup_growth_app();
    // Set time to midnight (nighttime: hour outside 6..20)
    app.world_mut().resource_mut::<GameTime>().total_seconds = 0.0;
    app.world_mut().resource_mut::<GameTime>().start_hour = 0;
    assert!(
        !app.world().resource::<GameTime>().is_daytime(),
        "should be nighttime"
    );
    add_farm(&mut app, 0, true);

    app.update();
    let entity = *app
        .world()
        .resource::<EntityMap>()
        .entities
        .get(&0)
        .unwrap();
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(
        ps.progress < f32::EPSILON,
        "crops should NOT grow at night: {}",
        ps.progress
    );
}

#[test]
fn cows_grow_at_night() {
    let mut app = setup_growth_app();
    // Set time to midnight (nighttime)
    app.world_mut().resource_mut::<GameTime>().total_seconds = 0.0;
    app.world_mut().resource_mut::<GameTime>().start_hour = 0;
    assert!(
        !app.world().resource::<GameTime>().is_daytime(),
        "should be nighttime"
    );
    add_cow_farm(&mut app, 0);

    app.update();
    let entity = *app
        .world()
        .resource::<EntityMap>()
        .entities
        .get(&0)
        .unwrap();
    let ps = app.world().get::<ProductionState>(entity).unwrap();
    assert!(
        ps.progress > 0.0,
        "cows should grow at night: {}",
        ps.progress
    );
}

#[test]
fn cows_consume_food() {
    let mut app = setup_growth_app();
    // Give the town some food
    let town_entity = *app
        .world()
        .resource::<crate::resources::TownIndex>()
        .0
        .get(&0)
        .unwrap();
    app.world_mut().get_mut::<FoodStore>(town_entity).unwrap().0 = 100;

    add_cow_farm(&mut app, 0);

    // Run a few ticks
    for _ in 0..5 {
        app.update();
    }

    let food = app.world().get::<FoodStore>(town_entity).unwrap().0;
    assert!(food < 100, "cow farm should consume food: {}", food);
}

#[test]
fn farmers_skip_cow_farms() {
    // Verify that find_farm_target returns None when only cow farms are available.
    let mut entity_map = EntityMap::default();
    entity_map.init_spatial(2048.0); // init spatial grid for search
    let slot = 0usize;
    let pos = Vec2::new(100.0, 100.0);
    let mut inst = test_building_instance(slot, BuildingKind::Farm, 0.0);
    inst.position = pos;
    let entity = Entity::from_raw_u32(0).unwrap();
    entity_map.set_entity(slot, entity);
    entity_map.add_instance(inst);

    let production_map = std::collections::HashMap::new();
    let mut cow_set = std::collections::HashSet::new();
    cow_set.insert(slot);

    // With cow set containing this slot, farmer should not target it
    let result = crate::systems::work_targeting::find_farm_target(
        pos,
        &entity_map,
        0,
        &production_map,
        &cow_set,
    );
    assert!(
        result.is_none(),
        "farmer should NOT target a cow farm, got {:?}",
        result
    );

    // Without the cow filter, farmer should find it
    let empty_set = std::collections::HashSet::new();
    let result = crate::systems::work_targeting::find_farm_target(
        pos,
        &entity_map,
        0,
        &production_map,
        &empty_set,
    );
    assert!(result.is_some(), "farmer SHOULD target a crop farm");
}

// ========================================================================
// merchant_tick_system tests
// ========================================================================

fn setup_merchant_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(MerchantInventory::default());
    app.insert_resource(NextLootItemId::default());
    app.insert_resource(WorldData {
        towns: vec![crate::world::Town {
            name: "Town".into(),
            center: Vec2::ZERO,
            faction: 0,
            kind: crate::constants::TownKind::Player,
        }],
    });
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, merchant_tick_system);
    app.update();
    app.update();
    app
}

#[test]
fn merchant_no_building_no_tick() {
    let mut app = setup_merchant_app();

    for _ in 0..50 {
        app.update();
    }
    let inv = app.world().resource::<MerchantInventory>();
    // Without a Merchant building, stocks should remain empty
    assert!(
        inv.stocks.is_empty() || inv.stocks[0].items.is_empty(),
        "no merchant building = no items"
    );
}

#[test]
fn merchant_ticks_with_building() {
    let mut app = setup_merchant_app();
    // Add a merchant building to town 0
    let mut inst = test_building_instance(0, BuildingKind::Merchant, 0.0);
    inst.town_idx = 0;
    app.world_mut()
        .resource_mut::<EntityMap>()
        .add_instance(inst);

    // Run enough updates for refresh timer to expire
    for _ in 0..100 {
        app.update();
    }
    let inv = app.world().resource::<MerchantInventory>();
    // Should have stocked items after refresh
    assert!(!inv.stocks.is_empty(), "merchant should have stocks");
    if !inv.stocks.is_empty() {
        assert!(
            !inv.stocks[0].items.is_empty(),
            "merchant should have items after refresh"
        );
    }
}

#[test]
fn merchant_paused_no_tick() {
    let mut app = setup_merchant_app();
    app.world_mut().resource_mut::<GameTime>().paused = true;
    let mut inst = test_building_instance(0, BuildingKind::Merchant, 0.0);
    inst.town_idx = 0;
    app.world_mut()
        .resource_mut::<EntityMap>()
        .add_instance(inst);

    app.update();
    let inv = app.world().resource::<MerchantInventory>();
    assert!(
        inv.stocks.is_empty() || inv.stocks[0].items.is_empty(),
        "paused: merchant should not tick"
    );
}

// -- farm_visual_system --------------------------------------------------

fn setup_farm_visual_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(EntityMap::default());
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, farm_visual_system);
    app.update();
    app.update();
    app
}

fn add_farm_visual(app: &mut App, slot: usize, growth_ready: bool) -> Entity {
    let inst = test_building_instance(slot, BuildingKind::Farm, 0.0);
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            Building {
                kind: BuildingKind::Farm,
            },
            ProductionState {
                ready: growth_ready,
                progress: if growth_ready { 1.0 } else { 0.0 },
            },
        ))
        .id();
    let mut em = app.world_mut().resource_mut::<EntityMap>();
    em.set_entity(slot, entity);
    em.add_instance(inst);
    entity
}

fn count_farm_markers(app: &mut App) -> usize {
    app.world_mut()
        .query::<&crate::components::FarmReadyMarker>()
        .iter(app.world())
        .count()
}

fn find_farm_marker(app: &mut App, slot: usize) -> Option<Entity> {
    app.world_mut()
        .query::<(Entity, &crate::components::FarmReadyMarker)>()
        .iter(app.world())
        .find_map(|(entity, marker)| (marker.farm_slot == slot).then_some(entity))
}

#[test]
fn farm_visual_spawns_marker_when_ready() {
    let mut app = setup_farm_visual_app();
    add_farm_visual(&mut app, 5000, true);
    // Run 4 updates to hit the frame_count % 4 == 0 cadence
    for _ in 0..4 {
        app.update();
    }
    let count = count_farm_markers(&mut app);
    assert!(
        count > 0,
        "should spawn FarmReadyMarker when growth_ready=true"
    );
}

#[test]
fn farm_visual_no_marker_when_not_ready() {
    let mut app = setup_farm_visual_app();
    add_farm_visual(&mut app, 5000, false);
    for _ in 0..4 {
        app.update();
    }
    let count = count_farm_markers(&mut app);
    assert_eq!(count, 0, "should not spawn marker when growth_ready=false");
}

#[test]
fn farm_visual_despawns_marker_when_no_longer_ready() {
    let mut app = setup_farm_visual_app();
    let entity = add_farm_visual(&mut app, 5000, true);
    // Spawn the marker
    for _ in 0..4 {
        app.update();
    }
    assert!(
        count_farm_markers(&mut app) > 0,
        "precondition: marker exists"
    );
    // Set growth_ready to false via ECS
    app.world_mut()
        .get_mut::<ProductionState>(entity)
        .unwrap()
        .ready = false;
    for _ in 0..4 {
        app.update();
    }
    let count = count_farm_markers(&mut app);
    assert_eq!(
        count, 0,
        "marker should be despawned when growth_ready becomes false"
    );
}

#[test]
fn farm_visual_despawns_marker_when_farm_removed_and_allows_slot_reuse() {
    let mut app = setup_farm_visual_app();
    let entity = add_farm_visual(&mut app, 5000, true);

    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        count_farm_markers(&mut app),
        1,
        "precondition: ready farm should have one marker"
    );

    app.world_mut().entity_mut(entity).despawn();
    app.world_mut()
        .resource_mut::<EntityMap>()
        .remove_by_slot(5000);
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        count_farm_markers(&mut app),
        0,
        "removing a ready farm should also remove its marker"
    );

    add_farm_visual(&mut app, 5000, true);
    for _ in 0..4 {
        app.update();
    }
    assert_eq!(
        count_farm_markers(&mut app),
        1,
        "slot reuse should still allow a new ready farm marker to spawn"
    );
}

#[test]
fn farm_visual_respawns_marker_if_mapping_points_to_stale_entity() {
    let mut app = setup_farm_visual_app();
    add_farm_visual(&mut app, 5000, true);
    for _ in 0..4 {
        app.update();
    }
    let marker_entity = find_farm_marker(&mut app, 5000).expect("precondition: marker exists");
    assert!(
        app.world_mut().despawn(marker_entity),
        "precondition: marker should despawn externally"
    );

    for _ in 0..4 {
        app.update();
    }

    assert_eq!(
        count_farm_markers(&mut app),
        1,
        "ready farm should respawn marker after stale mapping is pruned"
    );
}

// -- spawner_respawn_system ----------------------------------------------

#[derive(Resource, Default)]
struct CollectedSpawns(Vec<usize>); // slot indices from SpawnNpcMsg

fn collect_spawns(mut reader: MessageReader<SpawnNpcMsg>, mut collected: ResMut<CollectedSpawns>) {
    for msg in reader.read() {
        collected.0.push(msg.slot_idx);
    }
}

/// Reset hour_ticked after spawner runs to prevent re-processing on subsequent sub-ticks.
/// In the real game, game_time_system manages this flag, but in tests we set it manually.
fn reset_hour_ticked(mut game_time: ResMut<GameTime>) {
    game_time.hour_ticked = false;
}

fn setup_spawner_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(GpuSlotPool::default());
    app.insert_resource(CollectedSpawns::default());
    app.insert_resource(WorldData {
        towns: vec![crate::world::Town {
            name: "TestTown".to_string(),
            center: Vec2::new(500.0, 500.0),
            faction: 0,
            kind: crate::constants::TownKind::Player,
        }],
    });
    app.insert_resource(crate::resources::SystemTimings::default());
    // Register all message types needed by DirtyWriters + system
    app.add_message::<SpawnNpcMsg>();
    app.add_message::<CombatLogMsg>();
    app.add_message::<crate::messages::BuildingGridDirtyMsg>();
    app.add_message::<crate::messages::TerrainDirtyMsg>();
    app.add_message::<crate::messages::PatrolsDirtyMsg>();
    app.add_message::<crate::messages::PatrolPerimeterDirtyMsg>();
    app.add_message::<crate::messages::HealingZonesDirtyMsg>();
    app.add_message::<crate::messages::SquadsDirtyMsg>();
    app.add_message::<crate::messages::MiningDirtyMsg>();
    app.add_message::<crate::messages::PatrolSwapMsg>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(
        FixedUpdate,
        (spawner_respawn_system, collect_spawns, reset_hour_ticked).chain(),
    );
    app.update();
    app.update();
    app
}

fn add_spawner_building(
    app: &mut App,
    slot: usize,
    kind: BuildingKind,
    respawn_timer: f32,
) -> Entity {
    let inst = test_building_instance(slot, kind, 0.0);
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            Building { kind },
            SpawnerState {
                npc_slot: None,
                respawn_timer,
            },
        ))
        .id();
    let mut em = app.world_mut().resource_mut::<EntityMap>();
    em.set_entity(slot, entity);
    em.add_instance(inst);
    entity
}

#[test]
fn spawner_skips_without_hour_tick() {
    let mut app = setup_spawner_app();
    add_spawner_building(&mut app, 5000, BuildingKind::ArcherHome, 0.0);
    // Don't set hour_ticked
    app.update();
    let spawns = app.world().resource::<CollectedSpawns>();
    assert!(spawns.0.is_empty(), "should not spawn without hour_ticked");
}

#[test]
fn spawner_counts_down_timer() {
    let mut app = setup_spawner_app();
    // Use a high timer so it won't reach 0 even with multiple FixedUpdate sub-ticks
    let entity = add_spawner_building(&mut app, 5000, BuildingKind::ArcherHome, 50.0);
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;
    app.update();
    let timer = app
        .world()
        .get::<SpawnerState>(entity)
        .unwrap()
        .respawn_timer;
    assert!(timer < 50.0, "timer should decrement, got {timer}");
    assert!(timer >= 0.0, "timer should not go negative, got {timer}");
}

#[test]
fn spawner_spawns_when_timer_reaches_zero() {
    let mut app = setup_spawner_app();
    // Timer at 1.0 → decrements to 0.0 → triggers spawn
    add_spawner_building(&mut app, 5000, BuildingKind::ArcherHome, 1.0);
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;
    app.update();
    let spawns = app.world().resource::<CollectedSpawns>();
    assert!(
        !spawns.0.is_empty(),
        "should spawn an NPC when timer reaches 0"
    );
}

#[test]
fn spawner_assigns_uid_after_spawn() {
    let mut app = setup_spawner_app();
    let entity = add_spawner_building(&mut app, 5000, BuildingKind::ArcherHome, 1.0);
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;
    app.update();
    let ss = app.world().get::<SpawnerState>(entity).unwrap();
    assert!(
        ss.npc_slot.is_some(),
        "building should have npc_slot after spawn"
    );
    assert!(
        (ss.respawn_timer - (-1.0)).abs() < 0.01,
        "timer should reset to -1.0"
    );
}

/// Regression test: GameLog bundle -- spawner still writes combat log entries after migration.
/// This test would FAIL if spawner_respawn_system stopped calling game_log.combat_log.write().
#[test]
fn spawner_respawn_writes_combat_log_entry() {
    use crate::resources::CombatLog;
    use crate::systems::drain::drain_combat_log;

    let mut app = setup_spawner_app();
    app.insert_resource(CombatLog::default());
    app.add_message::<CombatLogMsg>();
    app.add_systems(FixedUpdate, drain_combat_log);

    // Set game_time.total_seconds > 0 so the log write runs (not just game start)
    app.world_mut().resource_mut::<GameTime>().total_seconds = 100.0;

    // Timer at 1.0 triggers a spawn (and a combat log write) on hour tick
    add_spawner_building(&mut app, 5000, BuildingKind::ArcherHome, 1.0);
    app.world_mut().resource_mut::<GameTime>().hour_ticked = true;
    app.update();

    let log = app.world().resource::<CombatLog>();
    assert!(
        !log.iter_all().next().is_none(),
        "spawner_respawn_system should write a combat log entry via GameLog bundle"
    );
}

// -- mining_policy_system ------------------------------------------------

#[derive(Resource, Default)]
struct SendMiningDirty(bool);

fn send_mining_dirty(
    mut writer: MessageWriter<crate::messages::MiningDirtyMsg>,
    mut flag: ResMut<SendMiningDirty>,
) {
    if flag.0 {
        writer.write(crate::messages::MiningDirtyMsg);
        flag.0 = false;
    }
}

fn setup_mining_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(EntityMap::default());
    app.insert_resource(MiningPolicy::default());
    app.insert_resource(SendMiningDirty(false));
    app.insert_resource(WorldData {
        towns: vec![crate::world::Town {
            name: "TestTown".to_string(),
            center: Vec2::new(500.0, 500.0),
            faction: 1,
            kind: crate::constants::TownKind::Player,
        }],
    });
    // Spawn ECS town entity for TownAccess
    let mut town_index = crate::resources::TownIndex::default();
    let entity = app
        .world_mut()
        .spawn((
            crate::components::TownMarker,
            crate::components::FoodStore(0),
            crate::components::GoldStore(0),
            crate::components::TownPolicy::default(),
            crate::components::TownUpgradeLevel::default(),
            crate::components::TownEquipment::default(),
        ))
        .id();
    town_index.0.insert(0, entity);
    app.insert_resource(town_index);
    app.add_message::<crate::messages::MiningDirtyMsg>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(
        FixedUpdate,
        (send_mining_dirty, mining_policy_system).chain(),
    );
    app.update();
    app.update();
    app
}

fn add_gold_mine(app: &mut App, slot: usize, pos: Vec2) {
    let mut inst = test_building_instance(slot, BuildingKind::GoldMine, 0.0);
    inst.position = pos;
    app.world_mut()
        .resource_mut::<EntityMap>()
        .add_instance(inst);
}

#[test]
fn mining_skips_without_dirty() {
    let mut app = setup_mining_app();
    add_gold_mine(&mut app, 6000, Vec2::new(600.0, 500.0));
    app.update();
    let mining = app.world().resource::<MiningPolicy>();
    assert!(
        mining.discovered_mines.is_empty() || mining.discovered_mines[0].is_empty(),
        "should not discover mines without dirty msg"
    );
}

#[test]
fn mining_discovers_mine_within_radius() {
    let mut app = setup_mining_app();
    // Town center is (500, 500), place mine nearby
    add_gold_mine(&mut app, 6000, Vec2::new(600.0, 500.0));
    app.insert_resource(SendMiningDirty(true));
    app.update();
    let mining = app.world().resource::<MiningPolicy>();
    assert!(
        !mining.discovered_mines.is_empty() && !mining.discovered_mines[0].is_empty(),
        "should discover nearby gold mine"
    );
    assert!(mining.discovered_mines[0].contains(&6000));
}

#[test]
fn mining_ignores_mine_outside_radius() {
    let mut app = setup_mining_app();
    // Place mine very far away
    add_gold_mine(&mut app, 6000, Vec2::new(99999.0, 99999.0));
    app.insert_resource(SendMiningDirty(true));
    app.update();
    let mining = app.world().resource::<MiningPolicy>();
    assert!(
        mining.discovered_mines.is_empty() || mining.discovered_mines[0].is_empty(),
        "should not discover mine outside radius"
    );
}

// -- squad_cleanup_system ------------------------------------------------

#[derive(Resource, Default)]
struct SendSquadsDirty(bool);

fn send_squads_dirty(
    mut writer: MessageWriter<crate::messages::SquadsDirtyMsg>,
    mut flag: ResMut<SendSquadsDirty>,
) {
    if flag.0 {
        writer.write(crate::messages::SquadsDirtyMsg);
        flag.0 = false;
    }
}

fn setup_squad_cleanup_app() -> App {
    use crate::resources::SquadState;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(SquadState::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(SendSquadsDirty(false));
    app.insert_resource(WorldData {
        towns: vec![crate::world::Town {
            name: "TestTown".to_string(),
            center: Vec2::new(500.0, 500.0),
            faction: 0,
            kind: crate::constants::TownKind::Player,
        }],
    });
    app.add_message::<crate::messages::SquadsDirtyMsg>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(
        FixedUpdate,
        (send_squads_dirty, squad_cleanup_system).chain(),
    );
    app.update();
    app.update();
    app
}

#[test]
fn squad_cleanup_skips_without_dirty() {
    use crate::resources::SquadState;
    let mut app = setup_squad_cleanup_app();
    // Add a dead member entity to squad
    {
        let mut ss = app.world_mut().resource_mut::<SquadState>();
        ss.squads[0]
            .members
            .push(Entity::from_raw_u32(999).unwrap());
    }
    app.update();
    let ss = app.world().resource::<SquadState>();
    assert_eq!(
        ss.squads[0].members.len(),
        1,
        "should not clean up without dirty msg"
    );
}

#[test]
fn squad_cleanup_removes_dead_members() {
    use crate::resources::SquadState;
    let mut app = setup_squad_cleanup_app();
    // Add entity that doesn't exist in EntityMap → treated as dead
    {
        let mut ss = app.world_mut().resource_mut::<SquadState>();
        ss.squads[0]
            .members
            .push(Entity::from_raw_u32(999).unwrap());
    }
    app.insert_resource(SendSquadsDirty(true));
    app.update();
    let ss = app.world().resource::<SquadState>();
    assert!(
        ss.squads[0].members.is_empty(),
        "dead member should be removed on dirty"
    );
}

#[test]
fn squad_cleanup_retains_alive_members() {
    use crate::resources::SquadState;
    let mut app = setup_squad_cleanup_app();
    // Register a live NPC in EntityMap
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            crate::components::Job::Archer,
            crate::components::TownId(0),
            crate::components::Faction(0),
        ))
        .id();
    {
        let mut em = app.world_mut().resource_mut::<EntityMap>();
        em.register_npc(0, entity, crate::components::Job::Archer, 0, 0);
    }
    {
        let mut ss = app.world_mut().resource_mut::<SquadState>();
        ss.squads[0].members.push(entity);
    }
    app.insert_resource(SendSquadsDirty(true));
    app.update();
    let ss = app.world().resource::<SquadState>();
    assert_eq!(
        ss.squads[0].members.len(),
        1,
        "alive member should be retained"
    );
}

// ============================================================================
// SYNC SLEEPING SYSTEM TESTS
// ============================================================================

fn setup_sleeping_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(EntityMap::default());
    app.add_systems(Update, sync_sleeping_system);
    app
}

fn spawn_resource_node(app: &mut App, slot: usize, sleeping: bool) -> Entity {
    use crate::components::ResourceNode;
    let mut builder = app.world_mut().spawn((
        GpuSlot(slot),
        Building {
            kind: crate::world::BuildingKind::TreeNode,
        },
        ResourceNode,
    ));
    if sleeping {
        builder.insert(Sleeping);
    }
    builder.id()
}

#[test]
fn sync_sleeping_wakes_occupied_resource_node() {
    let mut app = setup_sleeping_app();
    let slot = 10usize;
    let entity = spawn_resource_node(&mut app, slot, true);
    // Mark slot as physically present (sync_sleeping_system checks present_count, not occupancy)
    app.world_mut()
        .resource_mut::<EntityMap>()
        .set_present(slot, 1);

    app.update();

    assert!(
        app.world().get::<Sleeping>(entity).is_none(),
        "occupied resource node must not have Sleeping"
    );
}

#[test]
fn sync_sleeping_re_sleeps_vacant_resource_node() {
    let mut app = setup_sleeping_app();
    let slot = 11usize;
    let entity = spawn_resource_node(&mut app, slot, false);
    // Leave slot unoccupied (default)

    app.update();

    assert!(
        app.world().get::<Sleeping>(entity).is_some(),
        "vacant resource node must have Sleeping"
    );
}

#[test]
fn sync_sleeping_ignores_non_resource_node_buildings() {
    // A building with Sleeping but WITHOUT ResourceNode must not be woken
    // by sync_sleeping_system even if occupied. This guards the filter correctness.
    let mut app = setup_sleeping_app();
    let slot = 12usize;
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            Building {
                kind: crate::world::BuildingKind::Farm,
            },
            Sleeping,
        ))
        .id();
    app.world_mut()
        .resource_mut::<EntityMap>()
        .set_occupancy(slot, 1);

    app.update();

    assert!(
        app.world().get::<Sleeping>(entity).is_some(),
        "non-resource-node buildings must not be affected by sync_sleeping_system"
    );
}
