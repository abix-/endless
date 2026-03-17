use super::*;
use crate::components::{
    BaseAttackType, GoldStore, Job, Personality, TownEquipment, TraitInstance, TraitKind,
};
use crate::constants::BOW_TOWER_STATS;
use bevy::time::TimeUpdateStrategy;

// -- level_from_xp -------------------------------------------------------

#[test]
fn level_from_xp_zero() {
    assert_eq!(level_from_xp(0), 0);
}

#[test]
fn level_from_xp_negative() {
    assert_eq!(level_from_xp(-100), 0);
}

#[test]
fn level_from_xp_just_below_threshold() {
    // level 1 = sqrt(100/100) = 1 → need xp=100 for level 1
    assert_eq!(level_from_xp(99), 0);
}

#[test]
fn level_from_xp_at_threshold() {
    assert_eq!(level_from_xp(100), 1);
}

#[test]
fn level_from_xp_level_2() {
    // level 2 = sqrt(400/100) = 2
    assert_eq!(level_from_xp(400), 2);
}

#[test]
fn level_from_xp_between_levels() {
    assert_eq!(level_from_xp(300), 1); // sqrt(3) = 1.73 → floor = 1
}

// -- upgrade_cost --------------------------------------------------------

#[test]
fn upgrade_cost_level_0() {
    assert_eq!(upgrade_cost(0), 10);
}

#[test]
fn upgrade_cost_level_1() {
    assert_eq!(upgrade_cost(1), 20);
}

#[test]
fn upgrade_cost_level_2() {
    assert_eq!(upgrade_cost(2), 40);
}

#[test]
fn upgrade_cost_doubles_each_level() {
    for lv in 0..20 {
        assert_eq!(upgrade_cost(lv), 10 * (1 << lv as i32));
    }
}

#[test]
fn upgrade_cost_caps_at_20() {
    // levels above 20 should be clamped
    assert_eq!(upgrade_cost(21), upgrade_cost(20));
    assert_eq!(upgrade_cost(255), upgrade_cost(20));
}

// -- expansion_cost ------------------------------------------------------

#[test]
fn expansion_cost_level_0() {
    assert_eq!(expansion_cost(0), (24, 24));
}

#[test]
fn expansion_cost_level_1() {
    assert_eq!(expansion_cost(1), (32, 32));
}

#[test]
fn expansion_cost_scales_linearly() {
    let (f, g) = expansion_cost(5);
    assert_eq!(f, 24 + 8 * 5);
    assert_eq!(f, g);
}

// -- decode_upgrade_levels -----------------------------------------------

#[test]
fn decode_upgrade_levels_pads_short_input() {
    let result = decode_upgrade_levels(&[1, 2]);
    assert_eq!(result.len(), upgrade_count());
    assert_eq!(result[0], 1);
    assert_eq!(result[1], 2);
    // rest should be 0
    assert!(result[2..].iter().all(|&v| v == 0));
}

#[test]
fn decode_upgrade_levels_empty() {
    let result = decode_upgrade_levels(&[]);
    assert_eq!(result.len(), upgrade_count());
    assert!(result.iter().all(|&v| v == 0));
}

// -- upgrade_unlocked / upgrade_available --------------------------------

#[test]
fn upgrade_unlocked_no_prereqs() {
    // First upgrade in each branch typically has no prereqs
    let levels = vec![0u8; upgrade_count()];
    // Index 0 should have no prereqs (it's the first node)
    let node = &UPGRADES.nodes[0];
    if node.prereqs.is_empty() {
        assert!(upgrade_unlocked(&levels, 0));
    }
}

#[test]
fn upgrade_unlocked_with_unmet_prereqs() {
    let levels = vec![0u8; upgrade_count()];
    // Find a node that has prereqs
    for (idx, node) in UPGRADES.nodes.iter().enumerate() {
        if !node.prereqs.is_empty() {
            assert!(
                !upgrade_unlocked(&levels, idx),
                "node {idx} should be locked with all-zero levels"
            );
            break;
        }
    }
}

#[test]
fn upgrade_unlocked_with_met_prereqs() {
    let mut levels = vec![0u8; upgrade_count()];
    // Find a node with prereqs and satisfy them
    for (idx, node) in UPGRADES.nodes.iter().enumerate() {
        if !node.prereqs.is_empty() {
            for &(pi, min_lv) in &node.prereqs {
                levels[pi] = min_lv;
            }
            assert!(
                upgrade_unlocked(&levels, idx),
                "node {idx} should be unlocked after satisfying prereqs"
            );
            break;
        }
    }
}

#[test]
fn upgrade_available_needs_resources() {
    let mut levels = vec![0u8; upgrade_count()];
    // Find first node with no prereqs
    let idx = UPGRADES
        .nodes
        .iter()
        .position(|n| n.prereqs.is_empty())
        .unwrap();
    // Ensure prereqs met but no resources
    for &(pi, min_lv) in &UPGRADES.nodes[idx].prereqs {
        levels[pi] = min_lv;
    }
    assert!(!upgrade_available(&levels, idx, 0, 0));
    // With abundant resources
    assert!(upgrade_available(&levels, idx, 100_000, 100_000));
}

// -- deduct_upgrade_cost -------------------------------------------------

#[test]
fn deduct_upgrade_cost_decrements() {
    let idx = UPGRADES
        .nodes
        .iter()
        .position(|n| n.prereqs.is_empty())
        .unwrap();
    let mut food = 100_000;
    let mut gold = 100_000;
    let food_before = food;
    let gold_before = gold;
    deduct_upgrade_cost(idx, 0, &mut food, &mut gold);
    assert!(food <= food_before, "food should decrease or stay same");
    assert!(gold <= gold_before, "gold should decrease or stay same");
    assert!(
        food < food_before || gold < gold_before,
        "at least one resource should decrease"
    );
}

// -- format_upgrade_cost -------------------------------------------------

#[test]
fn format_upgrade_cost_contains_resource_label() {
    let idx = 0;
    let s = format_upgrade_cost(idx, 0);
    assert!(
        s.contains("food") || s.contains("gold"),
        "cost string should mention resource: {s}"
    );
}

// -- missing_prereqs -----------------------------------------------------

#[test]
fn missing_prereqs_none_when_satisfied() {
    let idx = UPGRADES
        .nodes
        .iter()
        .position(|n| n.prereqs.is_empty())
        .unwrap();
    let levels = vec![0u8; upgrade_count()];
    assert!(missing_prereqs(&levels, idx).is_none());
}

#[test]
fn missing_prereqs_returns_string_when_unsatisfied() {
    let levels = vec![0u8; upgrade_count()];
    for (idx, node) in UPGRADES.nodes.iter().enumerate() {
        if !node.prereqs.is_empty() {
            let msg = missing_prereqs(&levels, idx);
            assert!(msg.is_some(), "should have missing prereqs for node {idx}");
            assert!(msg.unwrap().contains("Requires:"));
            break;
        }
    }
}

// -- resolve_combat_stats ------------------------------------------------

fn default_config() -> CombatConfig {
    CombatConfig::default()
}

fn empty_upgrades() -> Vec<u8> {
    vec![0u8; upgrade_count()]
}

#[test]
fn resolve_combat_stats_archer_defaults() {
    let config = default_config();
    let upgrades = empty_upgrades();
    let personality = Personality::default();
    let stats = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        0,
        &personality,
        &config,
        &upgrades,
        0.0,
        0.0,
        0.0,
    );
    let def = npc_def(Job::Archer);
    // With no upgrades, no level, no equipment, no traits:
    // damage = base_damage * 1.0 * 1.0 * 1.0 * 1.0
    assert!(
        (stats.damage - def.base_damage).abs() < 0.01,
        "damage: {} vs {}",
        stats.damage,
        def.base_damage
    );
    assert!(
        (stats.max_health - def.base_hp).abs() < 0.01,
        "hp: {} vs {}",
        stats.max_health,
        def.base_hp
    );
    assert!(
        (stats.speed - def.base_speed).abs() < 0.01,
        "speed: {} vs {}",
        stats.speed,
        def.base_speed
    );
    assert_eq!(stats.berserk_bonus, 0.0);
}

#[test]
fn resolve_combat_stats_level_scaling() {
    let config = default_config();
    let upgrades = empty_upgrades();
    let personality = Personality::default();
    let stats_lv0 = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        0,
        &personality,
        &config,
        &upgrades,
        0.0,
        0.0,
        0.0,
    );
    let stats_lv10 = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        10,
        &personality,
        &config,
        &upgrades,
        0.0,
        0.0,
        0.0,
    );
    // level 10 = 1.10x multiplier on damage and hp
    assert!(stats_lv10.damage > stats_lv0.damage);
    let expected_ratio = 1.10;
    let actual_ratio = stats_lv10.damage / stats_lv0.damage;
    assert!(
        (actual_ratio - expected_ratio).abs() < 0.01,
        "ratio: {actual_ratio}"
    );
}

#[test]
fn resolve_combat_stats_equipment_bonus() {
    let config = default_config();
    let upgrades = empty_upgrades();
    let personality = Personality::default();
    let base = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        0,
        &personality,
        &config,
        &upgrades,
        0.0,
        0.0,
        0.0,
    );
    let with_weapon = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        0,
        &personality,
        &config,
        &upgrades,
        0.5,
        0.0,
        0.0,
    );
    let with_armor = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        0,
        &personality,
        &config,
        &upgrades,
        0.0,
        0.5,
        0.0,
    );
    // 50% weapon bonus → 1.5x damage
    assert!((with_weapon.damage / base.damage - 1.5).abs() < 0.01);
    // 50% armor bonus → 1.5x max_health
    assert!((with_armor.max_health / base.max_health - 1.5).abs() < 0.01);
}

#[test]
fn resolve_combat_stats_berserk_from_ferocity() {
    let config = default_config();
    let upgrades = empty_upgrades();
    let personality = Personality {
        trait1: Some(TraitInstance {
            kind: TraitKind::Ferocity,
            magnitude: 1.0,
        }),
        trait2: None,
    };
    let stats = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        0,
        &personality,
        &config,
        &upgrades,
        0.0,
        0.0,
        0.0,
    );
    // Ferocity m=1.0 → berserk_bonus = 0.50 * 1.0 = 0.50
    assert!(
        (stats.berserk_bonus - 0.5).abs() < 0.01,
        "berserk: {}",
        stats.berserk_bonus
    );
}

#[test]
fn resolve_combat_stats_timid_negative_berserk() {
    let config = default_config();
    let upgrades = empty_upgrades();
    let personality = Personality {
        trait1: Some(TraitInstance {
            kind: TraitKind::Ferocity,
            magnitude: -1.0,
        }),
        trait2: None,
    };
    let stats = resolve_combat_stats(
        Job::Archer,
        BaseAttackType::Ranged,
        0,
        0,
        &personality,
        &config,
        &upgrades,
        0.0,
        0.0,
        0.0,
    );
    assert!(
        stats.berserk_bonus < 0.0,
        "timid should have negative berserk: {}",
        stats.berserk_bonus
    );
}

// -- resolve_tower_instance_stats ----------------------------------------

#[test]
fn resolve_tower_instance_stats_level_0_defaults() {
    let base = BOW_TOWER_STATS;
    let stats = resolve_tower_instance_stats(&base, 0, &[]);
    assert!((stats.range - base.range).abs() < 0.01);
    assert!((stats.damage - base.damage).abs() < 0.01);
    assert!((stats.cooldown - base.cooldown).abs() < 0.01);
}

#[test]
fn resolve_tower_instance_stats_level_scales() {
    let base = BOW_TOWER_STATS;
    let stats_lv0 = resolve_tower_instance_stats(&base, 0, &[]);
    let stats_lv10 = resolve_tower_instance_stats(&base, 10, &[]);
    assert!(stats_lv10.damage > stats_lv0.damage);
    assert!(stats_lv10.range > stats_lv0.range);
}

// -- proficiency_mult (unclamped linear) ----------------------------------

#[test]
fn proficiency_mult_zero_is_one() {
    assert!((proficiency_mult(0.0) - 1.0).abs() < f32::EPSILON);
}

#[test]
fn proficiency_mult_hundred_is_two() {
    assert!((proficiency_mult(100.0) - 2.0).abs() < 0.001);
}

#[test]
fn proficiency_mult_1000_is_eleven() {
    assert!((proficiency_mult(1000.0) - 11.0).abs() < 0.001);
}

#[test]
fn proficiency_mult_9999_is_godlike() {
    let cap = crate::constants::SOFT_CAP as f32;
    assert!((proficiency_mult(cap) - 100.99).abs() < 0.01);
}

// -- UpgradeRegistry::stat_mult ------------------------------------------

#[test]
fn stat_mult_zero_levels_returns_1() {
    let levels = vec![0u8; upgrade_count()];
    let mult = UPGRADES.stat_mult(&levels, "Military (Ranged)", UpgradeStatKind::Attack);
    assert!(
        (mult - 1.0).abs() < 0.001,
        "zero upgrade should give 1.0x mult, got {mult}"
    );
}

// -- auto_upgrade_system -------------------------------------------------

#[derive(Resource, Default)]
struct CollectedUpgrades(Vec<(usize, usize)>); // (town_idx, upgrade_idx)

fn collect_upgrades(
    mut reader: MessageReader<UpgradeMsg>,
    mut collected: ResMut<CollectedUpgrades>,
) {
    for msg in reader.read() {
        collected.0.push((msg.town_idx, msg.upgrade_idx));
    }
}

fn setup_auto_upgrade_app() -> App {
    use crate::components::*;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(crate::resources::GameTime::default());
    app.insert_resource(crate::resources::AutoUpgrade::default());
    app.insert_resource(CollectedUpgrades::default());
    // Spawn ECS town entity for TownAccess
    let mut town_index = crate::resources::TownIndex::default();
    let entity = app
        .world_mut()
        .spawn((
            TownMarker,
            FoodStore(0),
            GoldStore(0),
            TownPolicy::default(),
            TownUpgradeLevel::default(),
            TownEquipment::default(),
        ))
        .id();
    town_index.0.insert(0, entity);
    app.insert_resource(town_index);
    app.add_message::<UpgradeMsg>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, (auto_upgrade_system, collect_upgrades).chain());
    app.update();
    app.update();
    app
}

fn set_town_resources(app: &mut App, town_idx: i32, food: i32, gold: i32) {
    let entity = app.world().resource::<crate::resources::TownIndex>().0[&town_idx];
    app.world_mut()
        .get_mut::<crate::components::FoodStore>(entity)
        .unwrap()
        .0 = food;
    app.world_mut()
        .get_mut::<crate::components::GoldStore>(entity)
        .unwrap()
        .0 = gold;
}

fn get_town_food_gold(app: &App, town_idx: i32) -> (i32, i32) {
    let entity = app.world().resource::<crate::resources::TownIndex>().0[&town_idx];
    let food = app
        .world()
        .get::<crate::components::FoodStore>(entity)
        .unwrap()
        .0;
    let gold = app
        .world()
        .get::<crate::components::GoldStore>(entity)
        .unwrap()
        .0;
    (food, gold)
}

#[test]
fn auto_upgrade_skips_without_hour_tick() {
    let mut app = setup_auto_upgrade_app();
    // Enable auto for upgrade 0 but don't tick hour
    {
        let mut auto = app
            .world_mut()
            .resource_mut::<crate::resources::AutoUpgrade>();
        auto.ensure_towns(1);
        auto.flags[0][0] = true;
    }
    // Give plenty of resources
    set_town_resources(&mut app, 0, 999999, 999999);
    app.update();
    let collected = app.world().resource::<CollectedUpgrades>();
    assert!(
        collected.0.is_empty(),
        "no upgrades should fire without hour_ticked"
    );
}

#[test]
fn auto_upgrade_fires_on_hour_tick() {
    let mut app = setup_auto_upgrade_app();
    {
        let mut auto = app
            .world_mut()
            .resource_mut::<crate::resources::AutoUpgrade>();
        auto.ensure_towns(1);
        auto.flags[0][0] = true;
    }
    set_town_resources(&mut app, 0, 999999, 999999);
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();
    let collected = app.world().resource::<CollectedUpgrades>();
    assert!(
        !collected.0.is_empty(),
        "should fire at least one upgrade on hour tick with resources and auto enabled"
    );
    assert_eq!(collected.0[0].0, 0, "town_idx should be 0");
    assert_eq!(collected.0[0].1, 0, "upgrade_idx should be 0");
}

#[test]
fn auto_upgrade_skips_disabled_flags() {
    let mut app = setup_auto_upgrade_app();
    // All flags default to false
    set_town_resources(&mut app, 0, 999999, 999999);
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();
    let collected = app.world().resource::<CollectedUpgrades>();
    assert!(
        collected.0.is_empty(),
        "no upgrades should fire when all flags are false"
    );
}

#[test]
fn auto_upgrade_skips_unaffordable() {
    let mut app = setup_auto_upgrade_app();
    {
        let mut auto = app
            .world_mut()
            .resource_mut::<crate::resources::AutoUpgrade>();
        auto.ensure_towns(1);
        auto.flags[0][0] = true;
    }
    // Zero resources — can't afford anything
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();
    let collected = app.world().resource::<CollectedUpgrades>();
    assert!(
        collected.0.is_empty(),
        "no upgrades should fire with zero resources"
    );
}

// -- auto_tower_upgrade_system -------------------------------------------

fn setup_auto_tower_app() -> App {
    use crate::components::*;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(crate::resources::GameTime::default());
    app.insert_resource(crate::resources::EntityMap::default());
    // Spawn ECS town entity for TownAccess
    let mut town_index = crate::resources::TownIndex::default();
    let entity = app
        .world_mut()
        .spawn((
            TownMarker,
            FoodStore(100),
            GoldStore(100),
            TownPolicy::default(),
            TownUpgradeLevel::default(),
            TownEquipment::default(),
        ))
        .id();
    town_index.0.insert(0, entity);
    app.insert_resource(town_index);
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, auto_tower_upgrade_system);
    app.update();
    app.update();
    app
}

fn add_tower(app: &mut App, slot: usize, auto_flags: Vec<bool>) {
    use crate::components::*;
    use crate::resources::BuildingInstance;
    use crate::world::BuildingKind;
    let num = auto_flags.len();
    let inst = BuildingInstance {
        kind: BuildingKind::BowTower,
        position: bevy::math::Vec2::ZERO,
        town_idx: 0,
        slot,
        faction: 0,
    };
    app.world_mut()
        .resource_mut::<crate::resources::EntityMap>()
        .add_instance(inst);
    // Spawn ECS entity so Query<TowerBuildingState> finds it
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            TownId(0),
            Building {
                kind: BuildingKind::BowTower,
            },
            TowerBuildingState {
                kills: 0,
                xp: 0,
                upgrade_levels: vec![0; num],
                auto_upgrade_flags: auto_flags,
                equipped_weapon: None,
            },
        ))
        .id();
    app.world_mut()
        .resource_mut::<crate::resources::EntityMap>()
        .set_entity(slot, entity);
}

fn get_tower_state(app: &App, slot: usize) -> crate::components::TowerBuildingState {
    let em = app.world().resource::<crate::resources::EntityMap>();
    let entity = em.entities[&slot];
    app.world()
        .get::<crate::components::TowerBuildingState>(entity)
        .unwrap()
        .clone()
}

#[test]
fn auto_tower_upgrade_skips_without_hour_tick() {
    let mut app = setup_auto_tower_app();
    let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
    add_tower(&mut app, 5000, vec![true; num_tower_upgrades]);
    app.update();
    let tower = get_tower_state(&app, 5000);
    assert!(
        tower.upgrade_levels.iter().all(|&l| l == 0),
        "no upgrades should apply without hour_ticked"
    );
}

#[test]
fn auto_tower_upgrade_buys_cheapest_on_hour_tick() {
    let mut app = setup_auto_tower_app();
    let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
    add_tower(&mut app, 5000, vec![true; num_tower_upgrades]);
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();
    let tower = get_tower_state(&app, 5000);
    let total_upgrades: u8 = tower.upgrade_levels.iter().sum();
    assert!(
        total_upgrades > 0,
        "should buy at least one tower upgrade on hour tick"
    );
}

#[test]
fn auto_tower_upgrade_deducts_resources() {
    let mut app = setup_auto_tower_app();
    let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
    add_tower(&mut app, 5000, vec![true; num_tower_upgrades]);
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();
    let (food, gold) = get_town_food_gold(&app, 0);
    assert!(
        food < 100 || gold < 100,
        "resources should be deducted after purchase, food={food} gold={gold}"
    );
}

#[test]
fn auto_tower_upgrade_skips_disabled_flags() {
    let mut app = setup_auto_tower_app();
    let num_tower_upgrades = crate::constants::TOWER_UPGRADES.len();
    add_tower(&mut app, 5000, vec![false; num_tower_upgrades]);
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();
    let tower = get_tower_state(&app, 5000);
    let total_upgrades: u8 = tower.upgrade_levels.iter().sum();
    assert_eq!(
        total_upgrades, 0,
        "should not upgrade when all flags are false"
    );
}

// -- prune_town_equipment_system -------------------------------------------

fn setup_prune_app(item_count: usize) -> App {
    use crate::components::*;
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(crate::resources::GameTime::default());
    app.insert_resource(crate::resources::EntityMap::default());

    let mut world_data = crate::world::WorldData::default();
    world_data.towns.push(crate::world::Town {
        name: "PruneTown".into(),
        center: bevy::prelude::Vec2::new(0.0, 0.0),
        faction: 1,
        kind: crate::constants::TownKind::Player,
    });
    app.insert_resource(world_data);

    // Spawn town entity with items
    let mut town_index = crate::resources::TownIndex::default();
    let mut items: Vec<crate::constants::LootItem> = Vec::new();
    for i in 0..item_count {
        let rarity = match i % 4 {
            0 => crate::constants::Rarity::Common,
            1 => crate::constants::Rarity::Uncommon,
            2 => crate::constants::Rarity::Rare,
            _ => crate::constants::Rarity::Epic,
        };
        items.push(crate::constants::LootItem {
            id: i as u64,
            kind: crate::constants::ItemKind::Weapon,
            name: format!("Item{}", i),
            rarity,
            stat_bonus: (i as f32) * 0.01,
            sprite: (0.0, 0.0),
            weapon_type: None,
        });
    }
    let entity = app
        .world_mut()
        .spawn((
            TownMarker,
            FoodStore(0),
            GoldStore(0),
            WoodStore(0),
            StoneStore(0),
            TownPolicy::default(),
            TownUpgradeLevel::default(),
            TownEquipment(items),
        ))
        .id();
    town_index.0.insert(0, entity);
    app.insert_resource(town_index);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, prune_town_equipment_system);
    app.update();
    app.update();
    app
}

#[test]
fn prune_caps_town_equipment_at_limit() {
    let cap = crate::constants::TOWN_EQUIPMENT_CAP;
    let over = cap + 100;
    let mut app = setup_prune_app(over);
    let entity = app.world().resource::<crate::resources::TownIndex>().0[&0];
    let count_before = app.world().get::<TownEquipment>(entity).unwrap().0.len();
    assert_eq!(count_before, over);

    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();

    let eq = app.world().get::<TownEquipment>(entity).unwrap();
    assert!(
        eq.0.len() <= cap,
        "should prune to cap: got {}",
        eq.0.len()
    );

    let gold = app.world().get::<GoldStore>(entity).unwrap().0;
    let expected_gold = over - cap;
    assert_eq!(
        gold, expected_gold as i32,
        "should receive 1 gold per pruned item"
    );
}

#[test]
fn prune_keeps_highest_value_items() {
    let over = crate::constants::TOWN_EQUIPMENT_CAP + 100;
    let mut app = setup_prune_app(over);
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();

    let entity = app.world().resource::<crate::resources::TownIndex>().0[&0];
    let eq = app.world().get::<TownEquipment>(entity).unwrap();

    // All remaining items should have higher stat_bonus than the pruned threshold
    // Items were created with stat_bonus = i * 0.01, lowest indices = lowest value
    // After sorting by rarity then stat_bonus, the 100 lowest-value items are removed
    // Remaining items should all be from the higher end
    let min_bonus = eq.0.iter().map(|i| i.stat_bonus).fold(f32::MAX, f32::min);
    assert!(
        min_bonus > 0.0,
        "pruned items should have removed the lowest stat_bonus items"
    );
}

#[test]
fn prune_skips_under_cap() {
    let mut app = setup_prune_app(50);
    app.world_mut()
        .resource_mut::<crate::resources::GameTime>()
        .hour_ticked = true;
    app.update();

    let entity = app.world().resource::<crate::resources::TownIndex>().0[&0];
    let eq = app.world().get::<TownEquipment>(entity).unwrap();
    assert_eq!(eq.0.len(), 50, "should not prune when under cap");
    let gold = app.world().get::<GoldStore>(entity).unwrap().0;
    assert_eq!(gold, 0, "no gold when nothing pruned");
}

/// Regression test: TownEquipment stays bounded under 50K NPC kill rates.
///
/// Growth rate analysis at 50K NPCs:
///   - Assume 50% are enemy raiders (25,000 NPCs).
///   - Kill rate: roughly 600 kills/hour at heavy combat (10 kills/min sustained).
///   - Equipment drop rate: 0.30 (30% of raider kills generate 1 item).
///   - Raw generation: 600 * 0.30 = 180 items/hour per town.
///   - Cap: TOWN_EQUIPMENT_CAP = SOFT_CAP items per town.
///   - Prune fires hourly; excess removed oldest/lowest-value first -> gold.
///   - After 1 hour: max(180, 200) -> prune leaves at most 200.
///
/// Memory impact at cap: LootItem ~120 bytes * SOFT_CAP = ~1.2 MB per town (acceptable).
/// At 8 hours of play: 1,440 items accumulated (well under SOFT_CAP).
///
/// This test simulates 8 in-game hours at the 50K NPC kill rate by directly
/// inserting items into TownEquipment and running prune each hour.
#[test]
fn town_equipment_bounded_at_50k_kill_rate() {
    use crate::components::*;

    // 50K NPC scenario constants
    // 600 kills/hour * 0.30 drop rate = 180 items generated per hour
    const ITEMS_PER_HOUR: usize = 180;
    const HOURS: usize = 8;
    const CAP: usize = crate::constants::TOWN_EQUIPMENT_CAP;

    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(crate::resources::GameTime::default());
    app.insert_resource(crate::resources::EntityMap::default());

    let mut world_data = crate::world::WorldData::default();
    world_data.towns.push(crate::world::Town {
        name: "StressTown".into(),
        center: bevy::prelude::Vec2::ZERO,
        faction: 1,
        kind: crate::constants::TownKind::Player,
    });
    app.insert_resource(world_data);

    let mut town_index = crate::resources::TownIndex::default();
    let entity = app
        .world_mut()
        .spawn((
            TownMarker,
            FoodStore(0),
            GoldStore(0),
            WoodStore(0),
            StoneStore(0),
            TownPolicy::default(),
            TownUpgradeLevel::default(),
            TownEquipment::default(),
        ))
        .id();
    town_index.0.insert(0, entity);
    app.insert_resource(town_index);
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, prune_town_equipment_system);
    app.update();

    let mut next_id: u64 = 1;
    let mut counts: Vec<usize> = Vec::new();
    for hour in 0..HOURS {
        // Simulate items arriving this hour (raiders killed, equipment dropped)
        {
            let eq = app
                .world_mut()
                .get_mut::<TownEquipment>(entity)
                .expect("TownEquipment missing");
            let mut eq = eq;
            for i in 0..ITEMS_PER_HOUR {
                eq.0.push(crate::constants::LootItem {
                    id: next_id,
                    kind: crate::constants::ItemKind::Weapon,
                    name: format!("h{}i{}", hour, i),
                    rarity: crate::constants::Rarity::Common,
                    stat_bonus: (next_id as f32) * 0.001,
                    sprite: (0.0, 0.0),
                    weapon_type: None,
                });
                next_id += 1;
            }
        }

        // Prune fires once per game hour
        app.world_mut()
            .resource_mut::<crate::resources::GameTime>()
            .hour_ticked = true;
        app.update();

        let count = app.world().get::<TownEquipment>(entity).unwrap().0.len();
        assert!(
            count <= CAP,
            "hour {}: TownEquipment {} exceeds cap {} -- prune failed at 50K NPC kill rate",
            hour + 1,
            count,
            CAP
        );
        counts.push(count);
    }

    let final_count = *counts.last().unwrap();
    let total_generated = ITEMS_PER_HOUR * HOURS; // 1440 without cap

    // With cap=SOFT_CAP and 180 items/hour for 8 hours (1440 total), never hits cap.
    // Each hour accumulates: 180, 360, 540, ..., 1440
    for h in 0..HOURS {
        let expected = ITEMS_PER_HOUR * (h + 1);
        assert_eq!(counts[h], expected, "hour {}: expected {} items", h + 1, expected);
    }
    // Final state: all 1440 items kept (well under SOFT_CAP)
    assert_eq!(final_count, total_generated, "final: all {} items kept (cap {})", total_generated, CAP);
    assert!(
        total_generated < CAP,
        "at 50K NPCs for 8 hours, {} items generated stays under cap {}",
        total_generated,
        CAP
    );
}
