//! Factorio-style system benchmarks.
//!
//! Measures individual Bevy ECS systems at controlled entity counts
//! to establish scaling characteristics and detect regressions.
//!
//! Run: `cargo bench --bench system_bench`
//! Reports: `target/criterion/` (HTML with violin plots + regression detection)

use bevy_ecs::system::RunSystemOnce;
use bevy::prelude::*;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use endless::components::*;
use endless::constants::*;
use endless::gpu::EntityGpuState;
use endless::messages::*;
use endless::resources::*;
use endless::systems::{
    AiPlayerConfig, AiPlayerState, decision_system, attack_system,
    damage_system, healing_system, resolve_movement_system,
    building_tower_system, death_system, spawner_respawn_system,
};
use endless::gpu::populate_gpu_state;
use endless::systems::stats;
use endless::world;

// Entity counts to benchmark (Factorio-style scaling analysis)
const COUNTS: &[usize] = &[1_000, 5_000, 10_000, 25_000, 50_000];

// ── Headless app builder ───────────────────────────────────────────

/// Build a minimal Bevy App with all resources/messages but no rendering.
/// Mirrors `build_app()` resource registration without GPU/UI plugins.
fn build_bench_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins);

    // Messages (same as build_app)
    app.add_message::<SpawnNpcMsg>()
        .add_message::<DamageMsg>()
        .add_message::<GpuUpdateMsg>()
        .add_message::<ProjGpuUpdateMsg>()
        .add_message::<CombatLogMsg>()
        .add_message::<WorkIntentMsg>()
        .add_message::<BuildingGridDirtyMsg>()
        .add_message::<TerrainDirtyMsg>()
        .add_message::<PatrolsDirtyMsg>()
        .add_message::<PatrolPerimeterDirtyMsg>()
        .add_message::<HealingZonesDirtyMsg>()
        .add_message::<SquadsDirtyMsg>()
        .add_message::<MiningDirtyMsg>()
        .add_message::<PatrolSwapMsg>()
        .add_message::<PlaySfxMsg>()
        .add_message::<stats::UpgradeMsg>()
        .add_message::<stats::EquipItemMsg>()
        .add_message::<stats::UnequipItemMsg>()
        .add_message::<DestroyBuildingMsg>()
        .add_message::<SelectFactionMsg>();

    // Resources (same as build_app, minus UI/audio/save-specific ones)
    app.init_resource::<Difficulty>()
        .init_resource::<EntityMap>()
        .init_resource::<PopulationStats>()
        .init_resource::<GameConfig>()
        .init_resource::<GameTime>()
        .init_resource::<world::WorldData>()
        .init_resource::<HealthDebug>()
        .init_resource::<CombatDebug>()
        .init_resource::<NpcTargetThrashDebug>()
        .init_resource::<PathRequestQueue>()
        .init_resource::<PathfindConfig>()
        .init_resource::<PathfindStats>()
        .init_resource::<KillStats>()
        .init_resource::<SelectedNpc>()
        .init_resource::<SelectedBuilding>()
        .init_resource::<FollowSelected>()
        .init_resource::<NpcMetaCache>()
        .init_resource::<NpcsByTownCache>()
        .init_resource::<NpcLogCache>()
        .init_resource::<DebugFlags>()
        .init_resource::<GpuReadState>()
        .init_resource::<ProjHitState>()
        .init_resource::<ProjPositionState>()
        .init_resource::<GpuSlotPool>()
        .init_resource::<NextEntityUid>()
        .init_resource::<ProjSlotAllocator>()
        .init_resource::<FoodStorage>()
        .init_resource::<GoldStorage>()
        .init_resource::<FactionStats>()
        .init_resource::<FactionList>()
        .init_resource::<Reputation>()
        .init_resource::<RaiderState>()
        .init_resource::<BuildingHealState>()
        .init_resource::<ActiveHealingSlots>()
        .init_resource::<HealingZoneCache>()
        .init_resource::<SystemTimings>()
        .init_resource::<UpsCounter>()
        .init_resource::<world::WorldGrid>()
        .init_resource::<world::WorldGenConfig>()
        .init_resource::<UiState>()
        .init_resource::<CombatLog>()
        .init_resource::<BuildMenuContext>()
        .init_resource::<TowerState>()
        .init_resource::<BuildingHpRender>()
        .init_resource::<SquadState>()
        .insert_resource(HelpCatalog::new())
        .init_resource::<TutorialState>()
        .init_resource::<MigrationState>()
        .init_resource::<EndlessMode>()
        .init_resource::<AiPlayerState>()
        .init_resource::<AiPlayerConfig>()
        .init_resource::<NpcDecisionConfig>()
        .init_resource::<stats::CombatConfig>()
        .init_resource::<stats::TownUpgrades>()
        .init_resource::<AutoUpgrade>()
        .init_resource::<TownPolicies>()
        .init_resource::<MiningPolicy>()
        .init_resource::<GameAudio>()
        .init_resource::<NextLootItemId>()
        .init_resource::<TownInventory>()
        .init_resource::<MerchantInventory>()
        .init_resource::<EntityGpuState>()
        .insert_resource(endless::settings::UserSettings::default());

    app
}

/// Populate an app with `n` NPC entities and matching GPU state.
/// All NPCs are alive Farmers in town 0 at grid positions within a 1600x1600 world.
fn populate_npcs(app: &mut App, count: usize) {
    let world = app.world_mut();

    // Set up world grid + town data
    {
        let mut grid = world.resource_mut::<world::WorldGrid>();
        grid.width = 25;
        grid.height = 25;
        grid.cell_size = TOWN_GRID_SPACING;
        grid.cells = vec![world::WorldCell::default(); 25 * 25];
    }
    {
        let mut wd = world.resource_mut::<world::WorldData>();
        wd.towns.push(world::Town {
            name: "BenchTown".into(),
            center: Vec2::new(800.0, 800.0),
            faction: 1,
            sprite_type: 0,
            area_level: 0,
        });
    }
    {
        let mut em = world.resource_mut::<EntityMap>();
        em.init_spatial(1600.0);
    }
    {
        let mut fl = world.resource_mut::<FactionList>();
        fl.factions.push(FactionData {
            kind: FactionKind::Neutral,
            name: "Neutral".into(),
            towns: vec![],
        });
        fl.factions.push(FactionData {
            kind: FactionKind::Player,
            name: "Player".into(),
            towns: vec![0],
        });
    }
    // Give the town food so systems don't early-return on starvation
    {
        let mut food = world.resource_mut::<FoodStorage>();
        food.init(1);
        food.food[0] = 100_000;
    }
    {
        let mut gold = world.resource_mut::<GoldStorage>();
        gold.init(1);
        gold.gold[0] = 100_000;
    }

    // Allocate GPU slots
    let mut slots = Vec::with_capacity(count);
    {
        let mut pool = world.resource_mut::<GpuSlotPool>();
        for _ in 0..count {
            if let Some(slot) = pool.alloc_reset() {
                slots.push(slot);
            }
        }
    }

    // Spawn NPC entities with full component sets
    let mut entity_slots: Vec<(Entity, usize)> = Vec::with_capacity(count);
    for (i, &slot) in slots.iter().enumerate() {
        let x = (i % 100) as f32 * 16.0;
        let y = (i / 100) as f32 * 16.0;
        // Split into nested tuples to stay under Bevy's 15-element Bundle limit
        let entity = world
            .spawn((
                (
                    GpuSlot(slot),
                    Position { x, y },
                    Health(100.0),
                    Job::Farmer,
                    Faction(1),
                    TownId(0),
                    Activity::Idle,
                    CombatState::default(),
                    Energy(100.0),
                    Speed(60.0),
                    Home(Vec2::new(800.0, 800.0)),
                    NpcFlags::default(),
                ),
                (
                    CachedStats {
                        damage: 10.0,
                        range: 40.0,
                        cooldown: 1.0,
                        projectile_speed: 0.0,
                        projectile_lifetime: 0.0,
                        max_health: 100.0,
                        speed: 60.0,
                        stamina: 1.0,
                        hp_regen: 0.0,
                        berserk_bonus: 0.0,
                    },
                    BaseAttackType::Melee,
                    AttackTimer(0.0),
                    NpcWorkState::default(),
                    PatrolRoute {
                        posts: vec![],
                        current: 0,
                    },
                    CarriedLoot {
                        food: 0,
                        gold: 0,
                        equipment: vec![],
                    },
                    Personality::default(),
                    FleeThreshold { pct: 0.2 },
                    LeashRange(400.0),
                    WoundedThreshold { pct: 0.3 },
                    HasEnergy,
                    NpcEquipment::default(),
                    SquadId(0),
                ),
            ))
            .id();
        entity_slots.push((entity, slot));
    }

    // Register entities in EntityMap
    {
        let mut em = world.resource_mut::<EntityMap>();
        for &(entity, slot) in &entity_slots {
            em.register_npc(slot, entity, Job::Farmer, 1, 0);
        }
    }

    // Pre-populate GPU readback state (normally comes from GPU compute)
    let max_slot = slots.iter().copied().max().unwrap_or(0) + 1;
    {
        let mut gpu_read = world.resource_mut::<GpuReadState>();
        gpu_read.positions.resize(max_slot * 2, 0.0);
        gpu_read.combat_targets.resize(max_slot, -1);
        gpu_read.health.resize(max_slot, 1.0);
        gpu_read.factions.resize(max_slot, 1);
        gpu_read.threat_counts.resize(max_slot, 0);
        gpu_read.npc_count = count;
        for (i, &slot) in slots.iter().enumerate() {
            let x = (i % 100) as f32 * 16.0;
            let y = (i / 100) as f32 * 16.0;
            gpu_read.positions[slot * 2] = x;
            gpu_read.positions[slot * 2 + 1] = y;
        }
    }

    // Pre-populate EntityGpuState (normally maintained by populate_gpu_state)
    {
        let mut gpu_state = world.resource_mut::<EntityGpuState>();
        for (i, &slot) in slots.iter().enumerate() {
            let x = (i % 100) as f32 * 16.0;
            let y = (i / 100) as f32 * 16.0;
            gpu_state.positions[slot * 2] = x;
            gpu_state.positions[slot * 2 + 1] = y;
            gpu_state.factions[slot] = 1;
            gpu_state.healths[slot] = 1.0;
            gpu_state.max_healths[slot] = 100.0;
            gpu_state.speeds[slot] = 60.0;
        }
    }
}

// ── Benchmarks ─────────────────────────────────────────────────────

fn bench_decision_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("decision_system");
    group.sample_size(20);
    for &count in COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, count);
                let _ = app.world_mut().run_system_once(decision_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(decision_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_damage_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("damage_system");
    for &count in COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, count);
                let damage_count = count / 10;
                let _ = app.world_mut().run_system_once(decision_system);
                b.iter(|| {
                    // Inject damage messages before each run
                    let _ = app.world_mut()
                        .run_system_once(move |mut writer: MessageWriter<DamageMsg>,
                                               q: Query<(&GpuSlot, &Faction), Without<Building>>| {
                            for (slot, faction) in q.iter().take(damage_count) {
                                writer.write(DamageMsg {
                                    target: EntityUid(slot.0 as u64),
                                    amount: 5.0,
                                    attacker: -1,
                                    attacker_faction: if faction.0 == 1 { 2 } else { 1 },
                                });
                            }
                        });
                    let _ = app.world_mut().run_system_once(damage_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_healing_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("healing_system");
    for &count in COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, count);
                // Damage 25% of NPCs so healing has work to do
                let _ = app.world_mut().run_system_once(
                    move |mut q: Query<&mut Health, Without<Building>>| {
                        let mut damaged = 0usize;
                        for mut hp in q.iter_mut() {
                            if damaged < count / 4 {
                                hp.0 = 50.0;
                                damaged += 1;
                            }
                        }
                    },
                );
                let _ = app.world_mut().run_system_once(healing_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(healing_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_attack_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("attack_system");
    group.sample_size(20);
    for &count in COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, count);
                // Set 10% of NPCs to Fighting with combat targets
                {
                    let world = app.world_mut();
                    let mut gpu_read = world.resource_mut::<GpuReadState>();
                    for i in (0..count).step_by(10) {
                        if i + 1 < count {
                            gpu_read.combat_targets[i] = (i + 1) as i32;
                        }
                    }
                }
                let _ = app.world_mut().run_system_once(
                    |mut q: Query<(&GpuSlot, &mut CombatState)>| {
                        for (slot, mut cs) in q.iter_mut() {
                            if slot.0 % 10 == 0 {
                                *cs = CombatState::Fighting { origin: Vec2::ZERO };
                            }
                        }
                    },
                );
                let _ = app.world_mut().run_system_once(attack_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(attack_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_resolve_movement_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolve_movement");
    group.sample_size(20);
    for &count in COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, count);
                // Initialize pathfind costs so A* has valid terrain data
                {
                    let world = app.world_mut();
                    let mut grid = world.resource_mut::<world::WorldGrid>();
                    grid.init_pathfind_costs();
                }
                // Add NpcPath to all NPCs (resolve_movement queries it)
                let _ = app.world_mut().run_system_once(
                    |mut commands: Commands, q: Query<Entity, Without<Building>>| {
                        for entity in q.iter() {
                            commands.entity(entity).insert(NpcPath {
                                waypoints: vec![],
                                current: 0,
                                goal_world: Vec2::ZERO,
                                path_cooldown: 0.0,
                            });
                        }
                    },
                );
                // Warmup run
                let _ = app.world_mut().run_system_once(resolve_movement_system);
                b.iter(|| {
                    // Enqueue path requests for 10% of NPCs each iteration
                    let _ = app.world_mut().run_system_once(
                        move |q: Query<(Entity, &GpuSlot, &Position), Without<Building>>,
                              mut queue: ResMut<PathRequestQueue>| {
                            for (entity, slot, pos) in q.iter().take(count / 10) {
                                let start_col = (pos.x / TOWN_GRID_SPACING) as i32;
                                let start_row = (pos.y / TOWN_GRID_SPACING) as i32;
                                queue.enqueue(PathRequest {
                                    entity,
                                    slot: slot.0,
                                    start: IVec2::new(start_col, start_row),
                                    goal: IVec2::new(start_col + 5, start_row + 3),
                                    goal_world: Vec2::new(
                                        (start_col + 5) as f32 * TOWN_GRID_SPACING,
                                        (start_row + 3) as f32 * TOWN_GRID_SPACING,
                                    ),
                                    priority: 1,
                                    source: PathSource::Movement,
                                });
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(resolve_movement_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_resolve_movement_unbounded(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolve_movement_unbounded");
    group.sample_size(20);
    for &count in COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, count);
                {
                    let world = app.world_mut();
                    world.resource_mut::<world::WorldGrid>().init_pathfind_costs();
                    // Lift budget caps to measure true unbounded cost
                    let mut config = world.resource_mut::<PathfindConfig>();
                    config.max_per_frame = 100_000;
                    config.max_time_budget_ms = 60_000.0; // 60 seconds — effectively unlimited
                }
                let _ = app.world_mut().run_system_once(
                    |mut commands: Commands, q: Query<Entity, Without<Building>>| {
                        for entity in q.iter() {
                            commands.entity(entity).insert(NpcPath {
                                waypoints: vec![],
                                current: 0,
                                goal_world: Vec2::ZERO,
                                path_cooldown: 0.0,
                            });
                        }
                    },
                );
                let _ = app.world_mut().run_system_once(resolve_movement_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(
                        move |q: Query<(Entity, &GpuSlot, &Position), Without<Building>>,
                              mut queue: ResMut<PathRequestQueue>| {
                            for (entity, slot, pos) in q.iter().take(count / 10) {
                                let start_col = (pos.x / TOWN_GRID_SPACING) as i32;
                                let start_row = (pos.y / TOWN_GRID_SPACING) as i32;
                                queue.enqueue(PathRequest {
                                    entity,
                                    slot: slot.0,
                                    start: IVec2::new(start_col, start_row),
                                    goal: IVec2::new(start_col + 5, start_row + 3),
                                    goal_world: Vec2::new(
                                        (start_col + 5) as f32 * TOWN_GRID_SPACING,
                                        (start_row + 3) as f32 * TOWN_GRID_SPACING,
                                    ),
                                    priority: 1,
                                    source: PathSource::Movement,
                                });
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(resolve_movement_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_building_tower_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("building_tower");
    group.sample_size(20);
    // Scale by tower count (with fixed enemy NPC population)
    const TOWER_COUNTS: &[usize] = &[100, 500, 1_000, 5_000, 50_000];
    for &tower_count in TOWER_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(tower_count),
            &tower_count,
            |b, &tower_count| {
                let mut app = build_bench_app();
                // Populate 5K enemy NPCs as targets
                populate_npcs(&mut app, 5_000);
                // Add a second (enemy) faction
                {
                    let world = app.world_mut();
                    let mut fl = world.resource_mut::<FactionList>();
                    fl.factions.push(FactionData {
                        kind: FactionKind::AiRaider,
                        name: "Enemy".into(),
                        towns: vec![],
                    });
                }
                // Spawn tower buildings with Building + Health components
                {
                    let world = app.world_mut();
                    let mut tower_slots = Vec::with_capacity(tower_count);
                    {
                        let mut pool = world.resource_mut::<GpuSlotPool>();
                        for _ in 0..tower_count {
                            if let Some(slot) = pool.alloc_reset() {
                                tower_slots.push(slot);
                            }
                        }
                    }
                    let mut tower_entities = Vec::with_capacity(tower_count);
                    for (i, &slot) in tower_slots.iter().enumerate() {
                        let x = 400.0 + (i % 224) as f32 * 32.0;
                        let y = 400.0 + (i / 224) as f32 * 32.0;
                        let entity = world.spawn((
                            GpuSlot(slot),
                            Position { x, y },
                            Health(500.0),
                            Faction(1),
                            TownId(0),
                            Building { kind: world::BuildingKind::Tower },
                        )).id();
                        tower_entities.push((entity, slot, x, y));
                    }
                    // Register tower buildings in EntityMap
                    let mut em = world.resource_mut::<EntityMap>();
                    for &(entity, slot, x, y) in &tower_entities {
                        em.entities.insert(slot, entity);
                        em.add_instance(BuildingInstance {
                            kind: world::BuildingKind::Tower,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: 1,
                            patrol_order: 0,
                            assigned_mine: None,
                            manual_mine: false,
                            wall_level: 0,
                            npc_uid: None,
                            respawn_timer: -1.0,
                            growth_ready: false,
                            growth_progress: 0.0,
                            occupants: 0,
                            under_construction: 0.0,
                            kills: 0,
                            xp: 0,
                            upgrade_levels: vec![],
                            auto_upgrade_flags: vec![],
                        });
                    }
                    // Set enemy faction on some NPCs in GpuReadState so towers have targets
                    let mut gpu_read = world.resource_mut::<GpuReadState>();
                    for i in 0..5_000.min(gpu_read.factions.len()) {
                        if i % 2 == 0 {
                            gpu_read.factions[i] = 2; // enemy faction
                        }
                    }
                }
                // Warmup
                let _ = app.world_mut().run_system_once(building_tower_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(building_tower_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_death_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("death_system");
    group.sample_size(20);
    // Scale by deaths-per-frame at fixed 50K total NPCs
    const DEATH_COUNTS: &[usize] = &[100, 500, 1_000, 5_000, 25_000];
    for &death_count in DEATH_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(death_count),
            &death_count,
            |b, &death_count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, 50_000);
                // Warmup — run once so system caches are primed
                let _ = app.world_mut().run_system_once(death_system);
                b.iter(|| {
                    // Set health to 0 on N NPCs so death_system discovers them
                    // (previous bug: inserting Dead directly bypassed the detection query)
                    let _ = app.world_mut().run_system_once(
                        move |mut q: Query<&mut Health, (Without<Building>, Without<Dead>)>| {
                            let mut killed = 0usize;
                            for mut hp in q.iter_mut() {
                                if killed >= death_count { break; }
                                hp.0 = 0.0;
                                killed += 1;
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(death_system);
                    // Flush despawn commands from death_system
                    app.world_mut().flush();

                    // Respawn killed NPCs so next iteration has victims
                    // (death_system despawns + unregisters + frees GPU slots)
                    let _ = app.world_mut().run_system_once(
                        move |world_mut: &mut World| {
                            let live_count = world_mut
                                .query_filtered::<&GpuSlot, Without<Building>>()
                                .iter(world_mut)
                                .count();
                            let need = 50_000usize.saturating_sub(live_count);
                            if need == 0 { return; }

                            let mut slots = Vec::with_capacity(need);
                            {
                                let mut pool = world_mut.resource_mut::<GpuSlotPool>();
                                for _ in 0..need {
                                    if let Some(slot) = pool.alloc_reset() {
                                        slots.push(slot);
                                    }
                                }
                            }
                            let mut spawned = Vec::with_capacity(need);
                            for (i, &slot) in slots.iter().enumerate() {
                                let x = (slot % 100) as f32 * 16.0;
                                let y = (slot / 100) as f32 * 16.0;
                                let entity = world_mut.spawn((
                                    (
                                        GpuSlot(slot),
                                        Position { x, y },
                                        Health(100.0),
                                        Job::Farmer,
                                        Faction(1),
                                        TownId(0),
                                        Activity::Idle,
                                        CombatState::default(),
                                        Energy(100.0),
                                        Speed(60.0),
                                        Home(Vec2::new(800.0, 800.0)),
                                        NpcFlags::default(),
                                    ),
                                    (
                                        CachedStats {
                                            damage: 10.0, range: 40.0, cooldown: 1.0,
                                            projectile_speed: 0.0, projectile_lifetime: 0.0,
                                            max_health: 100.0, speed: 60.0, stamina: 1.0,
                                            hp_regen: 0.0, berserk_bonus: 0.0,
                                        },
                                        BaseAttackType::Melee,
                                        AttackTimer(0.0),
                                        NpcWorkState::default(),
                                        PatrolRoute { posts: vec![], current: 0 },
                                        CarriedLoot { food: 0, gold: 0, equipment: vec![] },
                                        Personality::default(),
                                        FleeThreshold { pct: 0.2 },
                                        LeashRange(400.0),
                                        WoundedThreshold { pct: 0.3 },
                                        HasEnergy,
                                        NpcEquipment::default(),
                                        SquadId(0),
                                    ),
                                )).id();
                                spawned.push((entity, slot));
                                let _ = i;
                            }
                            let mut em = world_mut.resource_mut::<EntityMap>();
                            for &(entity, slot) in &spawned {
                                em.register_npc(slot, entity, Job::Farmer, 1, 0);
                            }
                        },
                    );
                });
            },
        );
    }
    group.finish();
}

fn bench_spawner_respawn_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("spawner_respawn");
    group.sample_size(20);
    // Scale by spawner building count
    const SPAWNER_COUNTS: &[usize] = &[100, 500, 1_000, 5_000, 50_000];
    for &spawner_count in SPAWNER_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(spawner_count),
            &spawner_count,
            |b, &spawner_count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, 1_000); // base NPCs for world setup
                // Create spawner building instances in EntityMap
                {
                    let world = app.world_mut();
                    let mut building_slots = Vec::with_capacity(spawner_count);
                    {
                        let mut pool = world.resource_mut::<GpuSlotPool>();
                        for _ in 0..spawner_count {
                            if let Some(slot) = pool.alloc_reset() {
                                building_slots.push(slot);
                            }
                        }
                    }
                    let mut em = world.resource_mut::<EntityMap>();
                    for (i, &slot) in building_slots.iter().enumerate() {
                        let x = 100.0 + (i % 224) as f32 * 32.0;
                        let y = 100.0 + (i / 224) as f32 * 32.0;
                        em.add_instance(BuildingInstance {
                            kind: world::BuildingKind::FarmerHome,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: 1,
                            patrol_order: 0,
                            assigned_mine: None,
                            manual_mine: false,
                            wall_level: 0,
                            npc_uid: None,
                            respawn_timer: 0.0, // ready to spawn
                            growth_ready: false,
                            growth_progress: 0.0,
                            occupants: 0,
                            under_construction: 0.0,
                            kills: 0,
                            xp: 0,
                            upgrade_levels: vec![],
                            auto_upgrade_flags: vec![],
                        });
                    }
                    // Set hour_ticked so system doesn't early-return
                    let mut game_time = world.resource_mut::<GameTime>();
                    game_time.hour_ticked = true;
                }
                // Warmup
                let _ = app.world_mut().run_system_once(spawner_respawn_system);
                b.iter(|| {
                    // Reset hour_ticked and spawner timers each iteration
                    let _ = app.world_mut().run_system_once(
                        move |mut game_time: ResMut<GameTime>,
                              mut entity_map: ResMut<EntityMap>| {
                            game_time.hour_ticked = true;
                            // Reset respawn timers so spawners fire again
                            for inst in entity_map.iter_instances_mut() {
                                if inst.respawn_timer < 0.0 {
                                    inst.respawn_timer = 0.0;
                                    inst.npc_uid = None;
                                }
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(spawner_respawn_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_populate_gpu_state(c: &mut Criterion) {
    let mut group = c.benchmark_group("populate_gpu_state");
    for &count in COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(count),
            &count,
            |b, &count| {
                let mut app = build_bench_app();
                populate_npcs(&mut app, count);
                // Warmup
                let _ = app.world_mut().run_system_once(populate_gpu_state);
                b.iter(|| {
                    // Seed GpuUpdateMsg messages (SetTarget for N/5 entities)
                    let msg_count = count / 5;
                    let _ = app.world_mut().run_system_once(
                        move |mut writer: MessageWriter<GpuUpdateMsg>| {
                            for i in 0..msg_count {
                                writer.write(GpuUpdateMsg(GpuUpdate::SetTarget {
                                    idx: i,
                                    x: (i % 100) as f32 * 16.0 + 8.0,
                                    y: (i / 100) as f32 * 16.0 + 8.0,
                                }));
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(populate_gpu_state);
                });
            },
        );
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_decision_system,
    bench_damage_system,
    bench_healing_system,
    bench_attack_system,
    bench_resolve_movement_system,
    bench_resolve_movement_unbounded,
    bench_building_tower_system,
    bench_death_system,
    bench_spawner_respawn_system,
    bench_populate_gpu_state,
);
criterion_main!(benches);
