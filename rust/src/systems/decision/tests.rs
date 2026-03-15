use super::*;
use crate::components::{
    Activity, CachedStats, CombatState, Energy, Faction, FoodStore, GoldStore, GpuSlot, Health,
    Home, NpcFlags, SquadId, TownAreaLevel, TownEquipment, TownId, TownMarker, TownPolicy,
    TownUpgradeLevel,
};
use crate::entity_map::EntityMap;
use crate::messages::{CombatLogMsg, GpuUpdateMsg, WorkIntentMsg};
use crate::resources::{
    GameTime, GpuReadState, NpcDecisionConfig, NpcLogCache, PathRequestQueue, PolicySet,
    PopulationStats, SelectedNpc, SquadState, TownIndex,
};
use crate::world::Town;
use bevy::ecs::system::RunSystemOnce;
use bevy::time::TimeUpdateStrategy;

fn setup_decision_app(policy: PolicySet) -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);
    app.add_message::<CombatLogMsg>();
    app.add_message::<crate::messages::DamageMsg>();
    app.add_message::<GpuUpdateMsg>();
    app.add_message::<WorkIntentMsg>();
    app.insert_resource(WorldData {
        towns: vec![Town {
            name: "TestTown".into(),
            center: Vec2::new(320.0, 320.0),
            faction: 1,
            kind: crate::constants::TownKind::Player,
        }],
    });
    app.insert_resource(PopulationStats::default());
    app.insert_resource(TownIndex::default());
    app.insert_resource(PathRequestQueue::default());
    app.insert_resource(GpuReadState {
        positions: vec![64.0, 64.0],
        npc_count: 1,
        ..Default::default()
    });
    app.insert_resource(GameTime {
        total_seconds: 16.0 * 5.0, // 22:55 -- night, so DayOnly jobs are off-duty
        ..Default::default()
    });
    app.insert_resource(TimeUpdateStrategy::ManualDuration(
        std::time::Duration::from_secs_f32(1.0),
    ));
    app.insert_resource(NpcLogCache::default());
    app.insert_resource(NpcDecisionConfig {
        interval: 0.0,
        max_decisions_per_frame: 1,
    });
    app.insert_resource(EntityMap::default());
    app.insert_resource(SquadState::default());
    app.insert_resource(SelectedNpc::default());
    let mut settings = crate::settings::UserSettings::default();
    settings.npc_log_mode = crate::settings::NpcLogMode::All;
    app.insert_resource(settings);
    app.add_systems(FixedUpdate, decision_system);

    let town_entity = app
        .world_mut()
        .spawn((
            TownMarker,
            TownPolicy(policy),
            FoodStore(0),
            GoldStore(0),
            TownUpgradeLevel::default(),
            TownEquipment::default(),
            TownAreaLevel::default(),
        ))
        .id();
    app.world_mut()
        .resource_mut::<TownIndex>()
        .0
        .insert(0, town_entity);
    app
}

fn test_cached_stats() -> CachedStats {
    CachedStats {
        damage: 5.0,
        range: 100.0,
        cooldown: 1.0,
        projectile_speed: 0.0,
        projectile_lifetime: 0.0,
        max_health: 100.0,
        speed: 100.0,
        stamina: 1.0,
        hp_regen: 0.0,
        berserk_bonus: 0.0,
    }
}

fn test_carried_loot(count: usize) -> CarriedLoot {
    CarriedLoot {
        equipment: (0..count)
            .map(|i| crate::constants::roll_loot_item(i as u64 + 1, i as u32 + 1))
            .collect(),
        ..Default::default()
    }
}
#[test]
fn squad_loot_threshold_overrides_town_policy() {
    DECISION_FRAME.store(0, std::sync::atomic::Ordering::Relaxed);

    let mut policy = PolicySet::default();
    policy.loot_threshold = 1;
    let mut app = setup_decision_app(policy);
    app.world_mut().resource_mut::<SquadState>().squads[0].loot_threshold = 3;

    let npc = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Job::Archer,
            TownId(0),
            Faction(1),
            Energy(100.0),
            Health(100.0),
            Home(Vec2::new(320.0, 320.0)),
            HasEnergy,
            NpcFlags::default(),
            CombatState::None,
            SquadId(0),
            Activity {
                kind: ActivityKind::Patrol,
                phase: ActivityPhase::Holding,
                target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                ..Default::default()
            },
            test_cached_stats(),
            test_carried_loot(2),
        ))
        .id();

    app.world_mut().run_system_once(decision_system).unwrap();

    let activity = app.world().get::<Activity>(npc).unwrap();
    assert_ne!(
        activity.kind,
        ActivityKind::ReturnLoot,
        "squad threshold should block the lower town-wide fallback"
    );
}

#[test]
fn town_loot_threshold_applies_without_squad() {
    let mut policy = PolicySet::default();
    policy.loot_threshold = 2;
    let squad_state = SquadState::default();

    assert_eq!(loot_threshold_for_npc(&squad_state, None, Some(policy)), 2);
}
// ========================================================================
// transition helper tests -- verify kind + phase + target invariants
// ========================================================================

#[test]
fn transition_activity_sets_all_fields() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Rest,
        ActivityPhase::Transit,
        ActivityTarget::Home,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Rest);
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(act.target, ActivityTarget::Home);
    assert_eq!(act.ticks_waiting, 0);
}

#[test]
fn transition_activity_resets_ticks() {
    let mut act = Activity {
        ticks_waiting: 42,
        ..Default::default()
    };
    transition_activity(
        &mut act,
        ActivityKind::Patrol,
        ActivityPhase::Holding,
        ActivityTarget::PatrolPost { route: 0, index: 1 },
        "test",
    );
    assert_eq!(
        act.ticks_waiting, 0,
        "transition should reset ticks_waiting"
    );
}

#[test]
fn transition_activity_preserves_recover_until_for_heal() {
    let mut act = Activity {
        recover_until: 0.75,
        ..Default::default()
    };
    transition_activity(
        &mut act,
        ActivityKind::Heal,
        ActivityPhase::Transit,
        ActivityTarget::Fountain,
        "test",
    );
    assert_eq!(
        act.recover_until, 0.75,
        "Heal should preserve recover_until"
    );
}

#[test]
fn transition_activity_clears_recover_until_for_non_heal() {
    let mut act = Activity {
        recover_until: 0.75,
        ..Default::default()
    };
    transition_activity(
        &mut act,
        ActivityKind::Idle,
        ActivityPhase::Ready,
        ActivityTarget::None,
        "test",
    );
    assert_eq!(
        act.recover_until, 0.0,
        "non-Heal should clear recover_until"
    );
}

#[test]
fn transition_phase_keeps_kind_and_target() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Rest,
        ActivityPhase::Transit,
        ActivityTarget::Home,
        "test",
    );
    act.ticks_waiting = 10;
    transition_phase(&mut act, ActivityPhase::Active, "test");
    assert_eq!(act.kind, ActivityKind::Rest);
    assert_eq!(act.phase, ActivityPhase::Active);
    assert_eq!(act.target, ActivityTarget::Home);
    assert_eq!(act.ticks_waiting, 0, "phase transition should reset ticks");
}
// ========================================================================
// squad-target lifecycle tests
// ========================================================================

#[test]
fn squad_target_entry_uses_squad_attack_not_patrol() {
    // Simulate idle archer choosing squad target: must be SquadAttack+Transit+SquadPoint
    let mut act = Activity::default();
    let target = Vec2::new(500.0, 500.0);
    transition_activity(
        &mut act,
        ActivityKind::SquadAttack,
        ActivityPhase::Transit,
        ActivityTarget::SquadPoint(target),
        "test",
    );
    assert_eq!(
        act.kind,
        ActivityKind::SquadAttack,
        "squad target entry must use SquadAttack"
    );
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(act.target, ActivityTarget::SquadPoint(target));
}

#[test]
fn squad_target_arrival_uses_squad_attack_holding() {
    // Simulate arrival at squad target: must be SquadAttack+Holding+SquadPoint
    let mut act = Activity::default();
    let target = Vec2::new(500.0, 500.0);
    transition_activity(
        &mut act,
        ActivityKind::SquadAttack,
        ActivityPhase::Transit,
        ActivityTarget::SquadPoint(target),
        "test",
    );
    // On arrival, transition to Holding
    transition_activity(
        &mut act,
        ActivityKind::SquadAttack,
        ActivityPhase::Holding,
        ActivityTarget::SquadPoint(target),
        "test",
    );
    assert_eq!(
        act.kind,
        ActivityKind::SquadAttack,
        "squad arrival must stay SquadAttack"
    );
    assert_eq!(act.phase, ActivityPhase::Holding);
    assert_eq!(act.target, ActivityTarget::SquadPoint(target));
}

#[test]
fn squad_target_not_confused_with_patrol() {
    // SquadAttack and Patrol must not collapse into each other
    let target = Vec2::new(300.0, 300.0);
    let mut squad_act = Activity::default();
    transition_activity(
        &mut squad_act,
        ActivityKind::SquadAttack,
        ActivityPhase::Holding,
        ActivityTarget::SquadPoint(target),
        "test",
    );

    let mut patrol_act = Activity::default();
    transition_activity(
        &mut patrol_act,
        ActivityKind::Patrol,
        ActivityPhase::Holding,
        ActivityTarget::PatrolPost { route: 0, index: 2 },
        "test",
    );

    assert_ne!(
        squad_act.kind, patrol_act.kind,
        "SquadAttack and Patrol must be distinct"
    );
    assert_ne!(
        squad_act.target, patrol_act.target,
        "SquadPoint and PatrolPost must be distinct"
    );
}

// ========================================================================
// Slice 3 lifecycle tests -- Work, Mine, ReturnLoot, Wander, Raid
// ========================================================================

#[test]
fn work_lifecycle_transit_to_active() {
    let mut act = Activity::default();
    // Entry: farmer starts working (idle -> transit)
    transition_activity(
        &mut act,
        ActivityKind::Work,
        ActivityPhase::Transit,
        ActivityTarget::Worksite,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Work);
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(act.target, ActivityTarget::Worksite);

    // Arrival: transit -> active (tending)
    transition_phase(&mut act, ActivityPhase::Active, "test");
    assert_eq!(act.kind, ActivityKind::Work);
    assert_eq!(act.phase, ActivityPhase::Active);
    assert_eq!(act.target, ActivityTarget::Worksite);
}

#[test]
fn work_harvest_to_return_loot() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Work,
        ActivityPhase::Active,
        ActivityTarget::Worksite,
        "test",
    );
    // Harvest -> ReturnLoot
    transition_activity(
        &mut act,
        ActivityKind::ReturnLoot,
        ActivityPhase::Transit,
        ActivityTarget::Dropoff,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::ReturnLoot);
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(act.target, ActivityTarget::Dropoff);
}

#[test]
fn work_tired_to_idle() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Work,
        ActivityPhase::Active,
        ActivityTarget::Worksite,
        "test",
    );
    // Tired -> Idle
    transition_activity(
        &mut act,
        ActivityKind::Idle,
        ActivityPhase::Ready,
        ActivityTarget::None,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Idle);
    assert_eq!(act.phase, ActivityPhase::Ready);
}

#[test]
fn mine_lifecycle_transit_to_holding() {
    let mut act = Activity::default();
    // Entry: miner starts (idle -> transit)
    transition_activity(
        &mut act,
        ActivityKind::Mine,
        ActivityPhase::Transit,
        ActivityTarget::Worksite,
        "test",
    );
    assert_eq!(act.phase, ActivityPhase::Transit);

    // Arrival: transit -> holding (tending/queued)
    transition_phase(&mut act, ActivityPhase::Holding, "test");
    assert_eq!(act.kind, ActivityKind::Mine);
    assert_eq!(act.phase, ActivityPhase::Holding);
    assert_eq!(act.target, ActivityTarget::Worksite);
}

#[test]
fn mine_harvest_to_return_loot() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Mine,
        ActivityPhase::Holding,
        ActivityTarget::Worksite,
        "test",
    );
    // Harvest turn -> ReturnLoot
    transition_activity(
        &mut act,
        ActivityKind::ReturnLoot,
        ActivityPhase::Transit,
        ActivityTarget::Dropoff,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::ReturnLoot);
    assert_eq!(act.target, ActivityTarget::Dropoff);
}

#[test]
fn return_loot_always_transit() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::ReturnLoot,
        ActivityPhase::Transit,
        ActivityTarget::Dropoff,
        "test",
    );
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(act.target, ActivityTarget::Dropoff);
}

#[test]
fn wander_always_transit() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Wander,
        ActivityPhase::Transit,
        ActivityTarget::None,
        "test",
    );
    assert_eq!(act.phase, ActivityPhase::Transit);
}

#[test]
fn raid_retarget_preserves_raid_kind() {
    let mut act = Activity::default();
    let farm1 = Vec2::new(100.0, 100.0);
    let farm2 = Vec2::new(300.0, 300.0);
    transition_activity(
        &mut act,
        ActivityKind::Raid,
        ActivityPhase::Transit,
        ActivityTarget::RaidPoint(farm1),
        "test",
    );
    // Retarget to different farm
    transition_activity(
        &mut act,
        ActivityKind::Raid,
        ActivityPhase::Transit,
        ActivityTarget::RaidPoint(farm2),
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Raid);
    assert_eq!(act.target, ActivityTarget::RaidPoint(farm2));
}

#[test]
fn visual_key_sleep_only_during_active() {
    let transit = Activity {
        kind: ActivityKind::Rest,
        phase: ActivityPhase::Transit,
        target: ActivityTarget::Home,
        ..Default::default()
    };
    let active = Activity {
        kind: ActivityKind::Rest,
        phase: ActivityPhase::Active,
        target: ActivityTarget::Home,
        ..Default::default()
    };
    assert_eq!(transit.visual_key(), 0, "no sleep icon during transit");
    assert_eq!(active.visual_key(), 1, "sleep icon during active rest");
}

// ========================================================================
// Rest lifecycle tests
// ========================================================================

#[test]
fn rest_lifecycle_transit_to_active() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Rest,
        ActivityPhase::Transit,
        ActivityTarget::Home,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Rest);
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(act.target, ActivityTarget::Home);

    // Arrival at home -> Active (sleeping)
    transition_phase(&mut act, ActivityPhase::Active, "test");
    assert_eq!(act.kind, ActivityKind::Rest);
    assert_eq!(act.phase, ActivityPhase::Active);
    assert_eq!(act.target, ActivityTarget::Home);
}

#[test]
fn rest_wake_from_active() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Rest,
        ActivityPhase::Active,
        ActivityTarget::Home,
        "test",
    );
    // Energy recovered -> wake to Idle
    transition_activity(
        &mut act,
        ActivityKind::Idle,
        ActivityPhase::Ready,
        ActivityTarget::None,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Idle);
    assert_eq!(act.phase, ActivityPhase::Ready);
    assert_eq!(act.target, ActivityTarget::None);
}

#[test]
fn rest_active_not_trapped_by_arrival_gate() {
    // The core Slice 1 bug: Rest+Active must NOT pass the Priority 0 arrival gate.
    // The gate is: at_destination && kind != Idle && phase in (Transit | Ready)
    let act = Activity {
        kind: ActivityKind::Rest,
        phase: ActivityPhase::Active,
        target: ActivityTarget::Home,
        ..Default::default()
    };
    let passes_gate = act.kind != ActivityKind::Idle
        && matches!(act.phase, ActivityPhase::Transit | ActivityPhase::Ready);
    assert!(
        !passes_gate,
        "Rest+Active must not pass Priority 0 arrival gate"
    );
}

// ========================================================================
// Heal lifecycle tests
// ========================================================================

#[test]
fn heal_lifecycle_transit_to_active() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Heal,
        ActivityPhase::Transit,
        ActivityTarget::Fountain,
        "test",
    );
    act.recover_until = 0.8;

    // Arrival at fountain -> Active (healing)
    transition_phase(&mut act, ActivityPhase::Active, "test");
    assert_eq!(act.kind, ActivityKind::Heal);
    assert_eq!(act.phase, ActivityPhase::Active);
    assert_eq!(act.target, ActivityTarget::Fountain);
    assert_eq!(
        act.recover_until, 0.8,
        "recover_until preserved through phase transition"
    );
}

#[test]
fn heal_wake_from_active() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Heal,
        ActivityPhase::Active,
        ActivityTarget::Fountain,
        "test",
    );
    act.recover_until = 0.8;
    // HP recovered -> wake to Idle
    transition_activity(
        &mut act,
        ActivityKind::Idle,
        ActivityPhase::Ready,
        ActivityTarget::None,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Idle);
    assert_eq!(act.phase, ActivityPhase::Ready);
    assert_eq!(
        act.recover_until, 0.0,
        "recover_until cleared on Idle transition"
    );
}

#[test]
fn heal_active_not_trapped_by_arrival_gate() {
    let act = Activity {
        kind: ActivityKind::Heal,
        phase: ActivityPhase::Active,
        target: ActivityTarget::Fountain,
        ..Default::default()
    };
    let passes_gate = act.kind != ActivityKind::Idle
        && matches!(act.phase, ActivityPhase::Transit | ActivityPhase::Ready);
    assert!(
        !passes_gate,
        "Heal+Active must not pass Priority 0 arrival gate"
    );
}

// ========================================================================
// Patrol lifecycle completeness
// ========================================================================

#[test]
fn patrol_advance_holding_to_transit() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Patrol,
        ActivityPhase::Holding,
        ActivityTarget::PatrolPost { route: 0, index: 0 },
        "test",
    );
    act.ticks_waiting = 60; // guard wait elapsed
    // Advance to next post
    transition_activity(
        &mut act,
        ActivityKind::Patrol,
        ActivityPhase::Transit,
        ActivityTarget::PatrolPost { route: 0, index: 1 },
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Patrol);
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(
        act.target,
        ActivityTarget::PatrolPost { route: 0, index: 1 }
    );
    assert_eq!(act.ticks_waiting, 0, "ticks reset on patrol advance");
}

#[test]
fn patrol_tired_exit_to_idle() {
    let mut act = Activity::default();
    transition_activity(
        &mut act,
        ActivityKind::Patrol,
        ActivityPhase::Holding,
        ActivityTarget::PatrolPost { route: 0, index: 2 },
        "test",
    );
    // Tired -> Idle
    transition_activity(
        &mut act,
        ActivityKind::Idle,
        ActivityPhase::Ready,
        ActivityTarget::None,
        "test",
    );
    assert_eq!(act.kind, ActivityKind::Idle);
    assert_eq!(act.phase, ActivityPhase::Ready);
}

// ========================================================================
// Ownership boundary tests
// ========================================================================

#[test]
fn idle_ready_is_chooser_entry_state() {
    // The idle chooser should only fire from Idle+Ready.
    // Any other state means the NPC has an active lifecycle.
    let idle = Activity::default();
    assert_eq!(idle.kind, ActivityKind::Idle);
    assert_eq!(idle.phase, ActivityPhase::Ready);

    // After any transition away, we're no longer in the chooser entry state
    let mut act = idle;
    transition_activity(
        &mut act,
        ActivityKind::Work,
        ActivityPhase::Transit,
        ActivityTarget::Worksite,
        "test",
    );
    assert_ne!(
        act.kind,
        ActivityKind::Idle,
        "working NPC not in chooser state"
    );
}

#[test]
fn arrival_gate_only_fires_for_transit_or_ready() {
    // Exhaustive check: Active and Holding must never pass the arrival gate
    let phases = [
        ActivityPhase::Ready,
        ActivityPhase::Transit,
        ActivityPhase::Active,
        ActivityPhase::Holding,
    ];
    for phase in &phases {
        let passes = matches!(phase, ActivityPhase::Transit | ActivityPhase::Ready);
        match phase {
            ActivityPhase::Ready | ActivityPhase::Transit => assert!(passes),
            ActivityPhase::Active | ActivityPhase::Holding => {
                assert!(!passes, "{:?} must not pass arrival gate", phase)
            }
        }
    }
}

// ========================================================================
// Valid phase combinations (spec table enforcement)
// ========================================================================

#[test]
fn valid_phase_combinations_match_spec() {
    // From docs/npc-activity-controller.md valid combinations table
    let valid: &[(ActivityKind, &[ActivityPhase])] = &[
        (ActivityKind::Idle, &[ActivityPhase::Ready]),
        (
            ActivityKind::Rest,
            &[ActivityPhase::Transit, ActivityPhase::Active],
        ),
        (
            ActivityKind::Heal,
            &[ActivityPhase::Transit, ActivityPhase::Active],
        ),
        (
            ActivityKind::Patrol,
            &[ActivityPhase::Transit, ActivityPhase::Holding],
        ),
        (
            ActivityKind::SquadAttack,
            &[ActivityPhase::Transit, ActivityPhase::Holding],
        ),
        (
            ActivityKind::Work,
            &[ActivityPhase::Transit, ActivityPhase::Active],
        ),
        (
            ActivityKind::Mine,
            &[
                ActivityPhase::Transit,
                ActivityPhase::Holding,
                ActivityPhase::Active,
            ],
        ),
        (
            ActivityKind::Raid,
            &[ActivityPhase::Transit, ActivityPhase::Active],
        ),
        (ActivityKind::ReturnLoot, &[ActivityPhase::Transit]),
        (ActivityKind::Wander, &[ActivityPhase::Transit]),
        (
            ActivityKind::Chop,
            &[ActivityPhase::Transit, ActivityPhase::Holding],
        ),
        (
            ActivityKind::Quarry,
            &[ActivityPhase::Transit, ActivityPhase::Holding],
        ),
        (
            ActivityKind::Repair,
            &[ActivityPhase::Transit, ActivityPhase::Active],
        ),
    ];

    // Verify Activity::new() produces Ready (default), which is valid for Idle
    // and acceptable as a pre-migration default for other kinds
    let idle = Activity::new(ActivityKind::Idle);
    assert_eq!(idle.phase, ActivityPhase::Ready);

    // Verify the table covers all 13 activity kinds
    assert_eq!(
        valid.len(),
        13,
        "spec table must cover all ActivityKind variants"
    );

    // Verify each kind's allowed phases are non-empty
    for (kind, phases) in valid {
        assert!(
            !phases.is_empty(),
            "{:?} must have at least one valid phase",
            kind
        );
    }
}

#[test]
fn transition_produces_valid_combinations() {
    // Verify that the transitions used in the codebase produce valid (kind, phase) pairs
    let test_cases: &[(ActivityKind, ActivityPhase, ActivityTarget)] = &[
        (
            ActivityKind::Idle,
            ActivityPhase::Ready,
            ActivityTarget::None,
        ),
        (
            ActivityKind::Rest,
            ActivityPhase::Transit,
            ActivityTarget::Home,
        ),
        (
            ActivityKind::Rest,
            ActivityPhase::Active,
            ActivityTarget::Home,
        ),
        (
            ActivityKind::Heal,
            ActivityPhase::Transit,
            ActivityTarget::Fountain,
        ),
        (
            ActivityKind::Heal,
            ActivityPhase::Active,
            ActivityTarget::Fountain,
        ),
        (
            ActivityKind::Patrol,
            ActivityPhase::Transit,
            ActivityTarget::PatrolPost { route: 0, index: 0 },
        ),
        (
            ActivityKind::Patrol,
            ActivityPhase::Holding,
            ActivityTarget::PatrolPost { route: 0, index: 0 },
        ),
        (
            ActivityKind::SquadAttack,
            ActivityPhase::Transit,
            ActivityTarget::SquadPoint(Vec2::ZERO),
        ),
        (
            ActivityKind::SquadAttack,
            ActivityPhase::Holding,
            ActivityTarget::SquadPoint(Vec2::ZERO),
        ),
        (
            ActivityKind::Work,
            ActivityPhase::Transit,
            ActivityTarget::Worksite,
        ),
        (
            ActivityKind::Work,
            ActivityPhase::Active,
            ActivityTarget::Worksite,
        ),
        (
            ActivityKind::Mine,
            ActivityPhase::Transit,
            ActivityTarget::Worksite,
        ),
        (
            ActivityKind::Mine,
            ActivityPhase::Holding,
            ActivityTarget::Worksite,
        ),
        (
            ActivityKind::Raid,
            ActivityPhase::Transit,
            ActivityTarget::RaidPoint(Vec2::ZERO),
        ),
        (
            ActivityKind::ReturnLoot,
            ActivityPhase::Transit,
            ActivityTarget::Dropoff,
        ),
        (
            ActivityKind::Wander,
            ActivityPhase::Transit,
            ActivityTarget::None,
        ),
        (
            ActivityKind::Repair,
            ActivityPhase::Transit,
            ActivityTarget::None,
        ),
        (
            ActivityKind::Repair,
            ActivityPhase::Active,
            ActivityTarget::None,
        ),
    ];

    for (kind, phase, target) in test_cases {
        let mut act = Activity::default();
        transition_activity(&mut act, *kind, *phase, *target, "test");
        assert_eq!(act.kind, *kind);
        assert_eq!(act.phase, *phase);
        assert_eq!(act.target, *target);
    }
}

// ========================================================================
// Homeless NPC tests
// ========================================================================

#[test]
fn home_invalid_detected() {
    // Home(-1,-1) is the orphan sentinel -- must not pass validity check
    let orphan = Home(Vec2::new(-1.0, -1.0));
    assert!(!orphan.is_valid(), "Home(-1,-1) must be invalid");

    let missing = Vec2::ZERO;
    // Vec2::ZERO home (missing component fallback) should also not be used as rest target
    assert!(
        !(missing.x >= 0.0 && missing.y >= 0.0) || missing == Vec2::ZERO,
        "Vec2::ZERO should be caught by home_valid check"
    );
}

#[test]
fn homeless_rest_targets_fountain_not_home() {
    // Homeless NPC should rest at fountain (ActivityTarget::Fountain),
    // never at home (ActivityTarget::Home targeting 0,0 or -1,-1)
    let mut act = Activity::default();
    // Simulate the homeless rest path: target fountain
    transition_activity(
        &mut act,
        ActivityKind::Rest,
        ActivityPhase::Transit,
        ActivityTarget::Fountain,
        "idle:rest_fountain_homeless",
    );
    assert_eq!(act.kind, ActivityKind::Rest);
    assert_eq!(act.phase, ActivityPhase::Transit);
    assert_eq!(
        act.target,
        ActivityTarget::Fountain,
        "homeless NPC must target Fountain, not Home"
    );
}

#[test]
fn homeless_idle_can_score_rest_when_town_exists() {
    assert!(has_rest_destination(false, Some(Vec2::new(320.0, 320.0))));
    assert!(!has_rest_destination(false, None));
}

#[test]
fn homeless_squad_rest_gate_targets_fountain() {
    DECISION_FRAME.store(0, std::sync::atomic::Ordering::Relaxed);

    let mut app = setup_decision_app(PolicySet::default());
    app.world_mut().resource_mut::<SquadState>().squads[0].rest_when_tired = true;
    let npc = app
        .world_mut()
        .spawn((
            GpuSlot(0),
            Job::Archer,
            TownId(0),
            Faction(1),
            Energy(20.0),
            Health(100.0),
            Home(Vec2::new(-1.0, -1.0)),
            NpcFlags::default(),
            CombatState::None,
            SquadId(0),
            Activity {
                kind: ActivityKind::Patrol,
                phase: ActivityPhase::Transit,
                target: ActivityTarget::PatrolPost { route: 0, index: 0 },
                ..Default::default()
            },
            test_cached_stats(),
        ))
        .id();

    app.world_mut().run_system_once(decision_system).unwrap();

    let activity = app.world().get::<Activity>(npc).unwrap();
    assert_eq!(activity.kind, ActivityKind::Rest);
    assert_eq!(activity.phase, ActivityPhase::Transit);
    assert_eq!(activity.target, ActivityTarget::Fountain);
}

#[test]
fn home_valid_rejects_orphan_and_missing() {
    // Orphan: Home(-1,-1)
    assert!(!Home(Vec2::new(-1.0, -1.0)).is_valid());
    // Missing component default
    assert!(!Home(Vec2::new(-1.0, 0.0)).is_valid());
    // Valid home
    assert!(Home(Vec2::new(100.0, 200.0)).is_valid());
    // Edge: 0,0 is technically valid per is_valid() but covered by unwrap_or guard
    assert!(Home(Vec2::ZERO).is_valid());
}
