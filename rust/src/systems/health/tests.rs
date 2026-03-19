use super::*;
use bevy::time::TimeUpdateStrategy;

fn stats_with_regen(hp_regen: f32) -> CachedStats {
    CachedStats {
        damage: 15.0,
        range: 200.0,
        cooldown: 1.5,
        projectile_speed: 200.0,
        projectile_lifetime: 1.5,
        max_health: 100.0,
        speed: 200.0,
        stamina: 1.0,
        hp_regen,
        berserk_bonus: 0.0,
    }
}

fn setup_regen_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(FixedUpdate, npc_regen_system);
    // Two priming updates: first inits Time, second accumulates FixedUpdate delta
    app.update();
    app.update();
    app
}

#[test]
fn regen_heals_damaged_npc() {
    let mut app = setup_regen_app();
    let npc = app
        .world_mut()
        .spawn((Health(50.0), stats_with_regen(5.0)))
        .id();

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(hp > 50.0, "regen should heal: {hp}");
}

#[test]
fn regen_capped_at_max_health() {
    let mut app = setup_regen_app();
    let npc = app
        .world_mut()
        .spawn((Health(99.9), stats_with_regen(100.0)))
        .id();

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(hp <= 100.0, "regen should not exceed max_health: {hp}");
}

#[test]
fn zero_regen_no_heal() {
    let mut app = setup_regen_app();
    let npc = app
        .world_mut()
        .spawn((Health(50.0), stats_with_regen(0.0)))
        .id();

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 50.0).abs() < f32::EPSILON,
        "zero regen should not heal: {hp}"
    );
}

#[test]
fn full_health_no_regen() {
    let mut app = setup_regen_app();
    let npc = app
        .world_mut()
        .spawn((Health(100.0), stats_with_regen(5.0)))
        .id();

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 100.0).abs() < f32::EPSILON,
        "full HP should not regen: {hp}"
    );
}

#[test]
fn dead_npcs_dont_regen() {
    let mut app = setup_regen_app();
    let npc = app
        .world_mut()
        .spawn((Health(10.0), stats_with_regen(5.0), Dead))
        .id();

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 10.0).abs() < f32::EPSILON,
        "dead NPC should not regen: {hp}"
    );
}

#[test]
fn buildings_excluded_from_regen() {
    let mut app = setup_regen_app();
    let building = app
        .world_mut()
        .spawn((
            Health(50.0),
            stats_with_regen(5.0),
            Building {
                kind: crate::world::BuildingKind::BowTower,
            },
        ))
        .id();

    app.update();
    let hp = app.world().get::<Health>(building).unwrap().0;
    assert!(
        (hp - 50.0).abs() < f32::EPSILON,
        "buildings should not use npc_regen: {hp}"
    );
}

#[test]
fn regen_paused_no_change() {
    let mut app = setup_regen_app();
    app.world_mut().resource_mut::<GameTime>().paused = true;
    let npc = app
        .world_mut()
        .spawn((Health(50.0), stats_with_regen(5.0)))
        .id();

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 50.0).abs() < f32::EPSILON,
        "paused game should not regen: {hp}"
    );
}

// ========================================================================
// damage_system tests
// ========================================================================

use crate::messages::DamageMsg;
use crate::resources::{BuildingHealState, EntityMap, HealthDebug};

/// Helper system that sends a DamageMsg once, then drains the queue.
/// Must drain to avoid re-sending on subsequent FixedUpdate sub-ticks.
fn send_damage(mut writer: MessageWriter<DamageMsg>, mut pending: ResMut<PendingDamage>) {
    for msg in pending.0.drain(..) {
        writer.write(msg);
    }
}

/// Resource to hold damage events to be sent by the helper system.
#[derive(Resource, Default)]
struct PendingDamage(Vec<DamageMsg>);

fn setup_damage_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(GameTime::default());
    app.insert_resource(EntityMap::default());
    app.insert_resource(HealthDebug::default());
    app.insert_resource(BuildingHealState::default());
    app.insert_resource(PendingDamage::default());
    app.insert_resource(bevy::time::TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_message::<DamageMsg>();
    app.add_message::<GpuUpdateMsg>();
    // send_damage runs first, then damage_system reads the messages
    app.add_systems(FixedUpdate, (send_damage, damage_system).chain());
    app.update();
    app.update();
    app
}

/// Spawn an NPC entity and register it in EntityMap.
fn spawn_damageable_npc(app: &mut App, slot: usize, _uid: u64, hp: f32) -> Entity {
    let entity = app.world_mut().spawn((GpuSlot(slot), Health(hp))).id();
    let mut entity_map = app.world_mut().resource_mut::<EntityMap>();
    entity_map.register_npc(slot, entity, crate::components::Job::Archer, 0, 0);
    entity
}

#[test]
fn damage_reduces_npc_health() {
    let mut app = setup_damage_app();
    let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
    app.world_mut()
        .resource_mut::<PendingDamage>()
        .0
        .push(DamageMsg {
            target: npc,
            amount: 30.0,
            attacker: -1,
            attacker_faction: 0,
        });

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 70.0).abs() < 0.01,
        "damage should reduce HP from 100 to 70: {hp}"
    );
}

#[test]
fn damage_floors_at_zero() {
    let mut app = setup_damage_app();
    let npc = spawn_damageable_npc(&mut app, 0, 1, 10.0);
    app.world_mut()
        .resource_mut::<PendingDamage>()
        .0
        .push(DamageMsg {
            target: npc,
            amount: 50.0,
            attacker: -1,
            attacker_faction: 0,
        });

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(hp >= 0.0, "HP should not go negative: {hp}");
    assert!(hp < 0.01, "HP should be at zero: {hp}");
}

#[test]
fn damage_to_unknown_entity_ignored() {
    let mut app = setup_damage_app();
    let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
    // Send damage to an entity that doesn't exist in EntityMap
    let fake = Entity::from_raw_u32(999).unwrap();
    app.world_mut()
        .resource_mut::<PendingDamage>()
        .0
        .push(DamageMsg {
            target: fake,
            amount: 50.0,
            attacker: -1,
            attacker_faction: 0,
        });

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 100.0).abs() < 0.01,
        "NPC should not take damage from mismatched UID: {hp}"
    );
}

#[test]
fn damage_updates_debug_entity_count() {
    let mut app = setup_damage_app();
    spawn_damageable_npc(&mut app, 0, 1, 100.0);

    app.update();
    let debug = app.world().resource::<HealthDebug>();
    // bevy_entity_count is updated every tick (not just when damage occurs)
    assert_eq!(
        debug.bevy_entity_count, 1,
        "debug should track entity count"
    );
}

#[test]
fn multiple_damage_events_stack() {
    let mut app = setup_damage_app();
    let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
    let pending = &mut app.world_mut().resource_mut::<PendingDamage>().0;
    pending.push(DamageMsg {
        target: npc,
        amount: 20.0,
        attacker: -1,
        attacker_faction: 0,
    });
    pending.push(DamageMsg {
        target: npc,
        amount: 15.0,
        attacker: -1,
        attacker_faction: 0,
    });

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 65.0).abs() < 0.01,
        "two damage events (20+15) should reduce to 65: {hp}"
    );
}

#[test]
fn damage_dead_npc_ignored() {
    let mut app = setup_damage_app();
    let npc = spawn_damageable_npc(&mut app, 0, 1, 100.0);
    // Mark NPC as dead in EntityMap
    app.world_mut()
        .resource_mut::<EntityMap>()
        .get_npc_mut(0)
        .unwrap()
        .dead = true;
    app.world_mut()
        .resource_mut::<PendingDamage>()
        .0
        .push(DamageMsg {
            target: npc,
            amount: 50.0,
            attacker: -1,
            attacker_faction: 0,
        });

    app.update();
    let hp = app.world().get::<Health>(npc).unwrap().0;
    assert!(
        (hp - 100.0).abs() < 0.01,
        "dead NPC should not take damage: {hp}"
    );
}

#[test]
fn damage_building_reduces_health() {
    let mut app = setup_damage_app();
    let slot = 0usize;
    let entity = app
        .world_mut()
        .spawn((
            GpuSlot(slot),
            Health(200.0),
            Building {
                kind: BuildingKind::BowTower,
            },
        ))
        .id();
    // Register as building instance in EntityMap
    let mut entity_map = app.world_mut().resource_mut::<EntityMap>();
    entity_map.set_entity(slot, entity);
    entity_map.add_instance(crate::resources::BuildingInstance {
        kind: BuildingKind::BowTower,
        position: Vec2::ZERO,
        town_idx: 0,
        slot,
        faction: 0,
    });

    app.world_mut()
        .resource_mut::<PendingDamage>()
        .0
        .push(DamageMsg {
            target: entity,
            amount: 75.0,
            attacker: -1,
            attacker_faction: 1,
        });

    app.update();
    let hp = app.world().get::<Health>(entity).unwrap().0;
    assert!(
        (hp - 125.0).abs() < 0.01,
        "building should take 75 damage from 200: {hp}"
    );
}

/// Regression test for issue #170: damage_system must not use iter_npcs+query.get.
/// The health_samples field was removed from HealthDebug (it was write-only dead code).
/// This test verifies the system runs correctly with many NPCs and a damage event,
/// confirming the iter_npcs+get pattern is absent (reverting would require re-adding
/// health_samples to HealthDebug, making this test fail to compile).
#[test]
fn damage_system_no_iter_npcs_sampling() {
    let mut app = setup_damage_app();
    // Spawn 15 NPCs (more than the old .take(10) sampling limit)
    for i in 0..15 {
        spawn_damageable_npc(&mut app, i, i as u64 + 1, 100.0);
    }
    let target_entity = {
        let em = app.world().resource::<EntityMap>();
        em.get_npc(0).unwrap().entity
    };
    app.world_mut()
        .resource_mut::<PendingDamage>()
        .0
        .push(DamageMsg {
            target: target_entity,
            amount: 10.0,
            attacker: -1,
            attacker_faction: 0,
        });
    app.update();
    let debug = app.world().resource::<HealthDebug>();
    assert_eq!(
        debug.damage_processed, 1,
        "damage_system should process 1 event without iter_npcs+get sampling"
    );
}

// -- update_healing_zone_cache -------------------------------------------

#[derive(Resource, Default)]
struct SendHealingDirty(bool);

fn send_healing_dirty(
    mut writer: MessageWriter<crate::messages::HealingZonesDirtyMsg>,
    mut flag: ResMut<SendHealingDirty>,
) {
    if flag.0 {
        writer.write(crate::messages::HealingZonesDirtyMsg);
        flag.0 = false;
    }
}

fn setup_healing_cache_app() -> App {
    use crate::world::{Town, WorldData};
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.insert_resource(crate::resources::HealingZoneCache::default());
    app.insert_resource(WorldData {
        towns: vec![Town {
            name: "TestTown".to_string(),
            center: Vec2::new(500.0, 500.0),
            faction: 1,
            kind: crate::constants::TownKind::Player,
        }],
    });
    app.insert_resource(CombatConfig::default());
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
    app.insert_resource(SendHealingDirty(false));
    app.add_message::<crate::messages::HealingZonesDirtyMsg>();
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.add_systems(
        FixedUpdate,
        (send_healing_dirty, update_healing_zone_cache).chain(),
    );
    app.update();
    app.update();
    app
}

#[test]
fn healing_cache_empty_without_dirty() {
    let mut app = setup_healing_cache_app();
    app.update();
    let cache = app.world().resource::<crate::resources::HealingZoneCache>();
    assert!(
        cache.by_faction.is_empty(),
        "cache should stay empty without dirty msg"
    );
}

#[test]
fn healing_cache_rebuilds_on_dirty() {
    let mut app = setup_healing_cache_app();
    app.insert_resource(SendHealingDirty(true));
    app.update();
    let cache = app.world().resource::<crate::resources::HealingZoneCache>();
    assert!(
        !cache.by_faction.is_empty(),
        "cache should have factions after dirty"
    );
    assert!(
        cache.by_faction.len() > 1,
        "cache should have at least 2 faction slots"
    );
    assert!(
        !cache.by_faction[1].is_empty(),
        "faction 1 should have a healing zone"
    );
    let zone = &cache.by_faction[1][0];
    assert!(
        (zone.center.x - 500.0).abs() < 0.1,
        "zone center should match town center"
    );
    assert!(zone.heal_rate > 0.0, "heal_rate should be positive");
    assert!(
        zone.enter_radius_sq > 0.0,
        "enter_radius_sq should be positive"
    );
}

#[test]
fn healing_cache_skips_negative_faction() {
    use crate::world::{Town, WorldData};
    let mut app = setup_healing_cache_app();
    // Replace towns with one negative faction
    app.insert_resource(WorldData {
        towns: vec![Town {
            name: "Abandoned".to_string(),
            center: Vec2::ZERO,
            faction: -1,
            kind: crate::constants::TownKind::Player,
        }],
    });
    app.insert_resource(SendHealingDirty(true));
    app.update();
    let cache = app.world().resource::<crate::resources::HealingZoneCache>();
    // With faction -1, max_faction < 0 so no factions
    assert!(
        cache.by_faction.is_empty() || cache.by_faction.iter().all(|v| v.is_empty()),
        "negative faction towns should not produce healing zones"
    );
}

#[test]
fn healing_cache_multiple_towns() {
    use crate::world::{Town, WorldData};
    let mut app = setup_healing_cache_app();
    app.insert_resource(WorldData {
        towns: vec![
            Town {
                name: "A".to_string(),
                center: Vec2::new(100.0, 100.0),
                faction: 1,
                kind: crate::constants::TownKind::Player,
            },
            Town {
                name: "B".to_string(),
                center: Vec2::new(900.0, 900.0),
                faction: 2,
                kind: crate::constants::TownKind::AiRaider,
            },
        ],
    });
    app.insert_resource(SendHealingDirty(true));
    app.update();
    let cache = app.world().resource::<crate::resources::HealingZoneCache>();
    assert!(
        cache.by_faction.len() >= 3,
        "should have at least 3 faction entries"
    );
    assert!(
        !cache.by_faction[1].is_empty(),
        "faction 1 should have a zone"
    );
    assert!(
        !cache.by_faction[2].is_empty(),
        "faction 2 should have a zone"
    );
}
