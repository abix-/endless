//! Factorio-style system benchmarks.
//!
//! Measures individual Bevy ECS systems at controlled entity counts
//! to establish scaling characteristics and detect regressions.
//!
//! Run: `cargo bench --bench system_bench`
//! Reports: `target/criterion/` (HTML with violin plots + regression detection)

use bevy::prelude::*;
use bevy_ecs::system::RunSystemOnce;
use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};

use endless::components::*;
use endless::constants::*;
use endless::gpu::populate_gpu_state;
use endless::gpu::{EntityGpuState, ProjBufferWrites};
use endless::messages::*;
use endless::npc_render::{
    BuildingBodyDirty, BuildingBodyInstances, build_building_body_instances,
};
use endless::resources::*;
use endless::systems::ai_player::{AiSnapshotDirty, RoadStyle};
use endless::systems::stats;
use endless::systems::work_targeting::resolve_work_targets;
use endless::systems::{
    AiKind, AiPersonality, AiPlayer, AiPlayerConfig, AiPlayerState, advance_waypoints_system,
    ai_decision_system, arrival_system, attack_system, building_tower_system,
    construction_tick_system, cooldown_system, damage_system, death_system, decision_system,
    energy_system, farm_visual_system, gpu_position_readback, growth_system, healing_system,
    npc_regen_system, on_duty_tick_system, process_proj_hits, resolve_movement_system,
    spawn_npc_system, spawner_respawn_system, sync_sleeping_system,
};
use endless::world;

// Entity counts to benchmark (Factorio-style scaling analysis)
const COUNTS: &[usize] = &[1_000, 50_000];

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
        .add_message::<stats::AutoEquipNowMsg>()
        .add_message::<DestroyBuildingMsg>()
        .add_message::<SelectFactionMsg>();

    // Resources (same as build_app, minus UI/audio/save-specific ones)
    app.init_resource::<Difficulty>()
        .init_resource::<EntityMap>()
        .init_resource::<PopulationStats>()
        .init_resource::<GameConfig>()
        .init_resource::<GameTime>()
        .init_resource::<DeltaTime>()
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
        .init_resource::<NpcLogCache>()
        .init_resource::<DebugFlags>()
        .init_resource::<GpuReadState>()
        .init_resource::<ProjHitState>()
        .init_resource::<ProjPositionState>()
        .init_resource::<GpuSlotPool>()
        .init_resource::<ProjSlotAllocator>()
        .init_resource::<ProjBufferWrites>()
        .init_resource::<TownIndex>()
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
        .init_resource::<AutoUpgrade>()
        .init_resource::<MiningPolicy>()
        .init_resource::<GameAudio>()
        .init_resource::<NextLootItemId>()
        .init_resource::<MerchantInventory>()
        .init_resource::<EntityGpuState>()
        .insert_resource(endless::settings::UserSettings::default());

    app
}

/// Spawn a town entity with per-town components and register it in TownIndex + WorldData.
fn spawn_bench_town(app: &mut App) {
    let world = app.world_mut();
    let entity = world
        .spawn((
            TownMarker,
            FoodStore(100_000),
            GoldStore(100_000),
            TownPolicy::default(),
            TownUpgradeLevel::default(),
            TownEquipment::default(),
        ))
        .id();
    let mut town_index = world.resource_mut::<TownIndex>();
    town_index.0.insert(0, entity);
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
            kind: TownKind::Player,
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
                    Activity::default(),
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
                        wood: 0,
                        stone: 0,
                        equipment: vec![],
                    },
                    Personality::default(),
                    FleeThreshold { pct: 0.2 },
                    LeashRange(400.0),
                    WoundedThreshold { pct: 0.3 },
                    HasEnergy,
                    NpcEquipment::default(),
                    SquadId(0),
                    NpcPath::default(),
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
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            let _ = app.world_mut().run_system_once(decision_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(decision_system);
            });
        });
    }
    group.finish();
}

fn bench_damage_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("damage_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            let damage_count = count / 10;
            let _ = app.world_mut().run_system_once(decision_system);
            b.iter(|| {
                // Inject damage messages before each run
                let _ = app.world_mut().run_system_once(
                    move |mut writer: MessageWriter<DamageMsg>,
                          q: Query<(Entity, &Faction), Without<Building>>| {
                        for (entity, faction) in q.iter().take(damage_count) {
                            writer.write(DamageMsg {
                                target: entity,
                                amount: 5.0,
                                attacker: -1,
                                attacker_faction: if faction.0 == 1 { 2 } else { 1 },
                            });
                        }
                    },
                );
                let _ = app.world_mut().run_system_once(damage_system);
            });
        });
    }
    group.finish();
}

fn bench_healing_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("healing_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
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
        });
    }
    group.finish();
}

fn bench_attack_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("attack_system");
    group.sample_size(20);
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
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
            let _ =
                app.world_mut()
                    .run_system_once(|mut q: Query<(&GpuSlot, &mut CombatState)>| {
                        for (slot, mut cs) in q.iter_mut() {
                            if slot.0 % 10 == 0 {
                                *cs = CombatState::Fighting { origin: Vec2::ZERO };
                            }
                        }
                    });
            let _ = app.world_mut().run_system_once(attack_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(attack_system);
            });
        });
    }
    group.finish();
}

fn bench_resolve_movement_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolve_movement");
    group.sample_size(20);
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
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
                            path_chunks: vec![],
                            path_blocked: false,
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
        });
    }
    group.finish();
}

fn bench_resolve_movement_unbounded(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolve_movement_unbounded");
    group.sample_size(20);
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            {
                let world = app.world_mut();
                world
                    .resource_mut::<world::WorldGrid>()
                    .init_pathfind_costs();
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
                            path_chunks: vec![],
                            path_blocked: false,
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
        });
    }
    group.finish();
}

fn bench_building_tower_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("building_tower");
    group.sample_size(20);
    // Scale by tower count (with fixed enemy NPC population)
    const TOWER_COUNTS: &[usize] = &[1_000, 50_000];
    for &tower_count in TOWER_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(tower_count),
            &tower_count,
            |b, &tower_count| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
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
                        let entity = world
                            .spawn((
                                GpuSlot(slot),
                                Position { x, y },
                                Health(500.0),
                                Faction(1),
                                TownId(0),
                                Building {
                                    kind: world::BuildingKind::BowTower,
                                },
                            ))
                            .id();
                        tower_entities.push((entity, slot, x, y));
                    }
                    // Register tower buildings in EntityMap
                    let mut em = world.resource_mut::<EntityMap>();
                    for &(entity, slot, x, y) in &tower_entities {
                        em.set_entity(slot, entity);
                        em.add_instance(BuildingInstance {
                            kind: world::BuildingKind::BowTower,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: 1,
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
    // Scale by deaths-per-frame at fixed 50K total NPCs.
    // Measures full death→despawn→respawn cycle — the real cost the game pays.
    // Scale by deaths-per-frame at fixed 50K total NPCs. Benchmark the work that
    // death_system performs after damage_system has already marked the victims Dead.
    const DEATH_COUNTS: &[usize] = &[1_000, 50_000];
    for &death_count in DEATH_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(death_count),
            &death_count,
            |b, &death_count| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 50_000);
                let _ = app.world_mut().run_system_once(death_system);
                b.iter(|| {
                    // Set health to 0 on N NPCs so death_system discovers them
                    let _ = app
                        .world_mut()
                        .run_system_once(move |world_mut: &mut World| {
                            let mut killed = 0usize;
                            let mut dead_entities = Vec::with_capacity(death_count);
                            let mut q = world_mut.query_filtered::<
                                (Entity, &mut Health),
                                (Without<Building>, Without<Dead>),
                            >();
                            for (entity, mut hp) in q.iter_mut(world_mut) {
                                if killed >= death_count {
                                    break;
                                }
                                hp.0 = 0.0;
                                dead_entities.push(entity);
                                killed += 1;
                            }
                            for entity in dead_entities {
                                world_mut.entity_mut(entity).insert(Dead);
                            }
                        });
                    // death_system: process Dead-marked NPCs, grant XP/loot, cleanup, despawn
                    let _ = app.world_mut().run_system_once(death_system);
                    app.world_mut().flush();

                    // Respawn killed NPCs (game pays this via spawner_respawn_system)
                    let _ = app
                        .world_mut()
                        .run_system_once(move |world_mut: &mut World| {
                            let live_count = world_mut
                                .query_filtered::<&GpuSlot, Without<Building>>()
                                .iter(world_mut)
                                .count();
                            let need = 50_000usize.saturating_sub(live_count);
                            if need == 0 {
                                return;
                            }

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
                            for &slot in &slots {
                                let x = (slot % 100) as f32 * 16.0;
                                let y = (slot / 100) as f32 * 16.0;
                                let entity = world_mut
                                    .spawn((
                                        (
                                            GpuSlot(slot),
                                            Position { x, y },
                                            Health(100.0),
                                            Job::Farmer,
                                            Faction(1),
                                            TownId(0),
                                            Activity::default(),
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
                                                wood: 0,
                                                stone: 0,
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
                                spawned.push((entity, slot));
                            }
                            let mut em = world_mut.resource_mut::<EntityMap>();
                            for &(entity, slot) in &spawned {
                                em.register_npc(slot, entity, Job::Farmer, 1, 0);
                            }
                        });
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
    const SPAWNER_COUNTS: &[usize] = &[1_000, 50_000];
    for &spawner_count in SPAWNER_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(spawner_count),
            &spawner_count,
            |b, &spawner_count| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 1_000); // base NPCs for world setup
                // Create spawner building entities with SpawnerState component
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
                    let mut building_entities = Vec::with_capacity(spawner_count);
                    for (i, &slot) in building_slots.iter().enumerate() {
                        let x = 100.0 + (i % 224) as f32 * 32.0;
                        let y = 100.0 + (i / 224) as f32 * 32.0;
                        let entity = world
                            .spawn((
                                GpuSlot(slot),
                                Position { x, y },
                                Health(100.0),
                                Faction(1),
                                TownId(0),
                                Building {
                                    kind: world::BuildingKind::FarmerHome,
                                },
                                SpawnerState {
                                    npc_slot: None,
                                    respawn_timer: 0.0,
                                },
                            ))
                            .id();
                        building_entities.push((entity, slot, x, y));
                    }
                    let mut em = world.resource_mut::<EntityMap>();
                    for &(entity, slot, x, y) in &building_entities {
                        em.set_entity(slot, entity);
                        em.add_instance(BuildingInstance {
                            kind: world::BuildingKind::FarmerHome,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: 1,
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
                              mut spawner_q: Query<&mut SpawnerState>| {
                            game_time.hour_ticked = true;
                            for mut spawner in spawner_q.iter_mut() {
                                if spawner.respawn_timer < 0.0 {
                                    spawner.respawn_timer = 0.0;
                                    spawner.npc_slot = None;
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
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
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
        });
    }
    group.finish();
}

/// Populate `count` farm/mine buildings with ECS components for economy benchmarks.
/// Half farms (some tended), half gold mines. All growable, none under construction.
fn populate_growable_buildings(app: &mut App, count: usize) {
    let world = app.world_mut();
    let mut building_slots = Vec::with_capacity(count);
    {
        let mut pool = world.resource_mut::<GpuSlotPool>();
        for _ in 0..count {
            if let Some(slot) = pool.alloc_reset() {
                building_slots.push(slot);
            }
        }
    }
    let mut building_entities = Vec::with_capacity(count);
    for (i, &slot) in building_slots.iter().enumerate() {
        let x = 100.0 + (i % 224) as f32 * 32.0;
        let y = 100.0 + (i / 224) as f32 * 32.0;
        let is_farm = i % 2 == 0;
        let kind = if is_farm {
            world::BuildingKind::Farm
        } else {
            world::BuildingKind::GoldMine
        };
        let tended = i % 4 == 0; // 25% tended
        let entity = world
            .spawn((
                GpuSlot(slot),
                Position { x, y },
                Health(100.0),
                Faction(1),
                TownId(0),
                Building { kind },
                ConstructionProgress(0.0),
                ProductionState {
                    ready: false,
                    progress: 0.0,
                },
            ))
            .id();
        building_entities.push((entity, slot, x, y, kind, tended));
    }
    let mut em = world.resource_mut::<EntityMap>();
    for &(entity, slot, x, y, kind, tended) in &building_entities {
        em.set_entity(slot, entity);
        em.add_instance(BuildingInstance {
            kind,
            position: Vec2::new(x, y),
            town_idx: 0,
            slot,
            faction: 1,
        });
        if tended {
            em.set_occupancy(slot, 1);
        }
    }
}

fn bench_growth_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("growth_system");
    group.sample_size(20);
    const BUILDING_COUNTS: &[usize] = &[1_000, 50_000];
    for &bcount in BUILDING_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 100); // minimal NPCs for world setup
                populate_growable_buildings(&mut app, bcount);
                // Warmup
                let _ = app.world_mut().run_system_once(growth_system);
                b.iter(|| {
                    // Reset growth so system has work each iteration
                    let _ =
                        app.world_mut()
                            .run_system_once(|mut q: Query<&mut ProductionState>| {
                                for mut ps in q.iter_mut() {
                                    ps.ready = false;
                                    ps.progress = 0.5;
                                }
                            });
                    let _ = app.world_mut().run_system_once(growth_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_construction_tick_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("construction_tick");
    group.sample_size(20);
    const BUILDING_COUNTS: &[usize] = &[1_000, 50_000];
    for &bcount in BUILDING_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 100);
                // Populate buildings under construction
                {
                    let world = app.world_mut();
                    let mut building_slots = Vec::with_capacity(bcount);
                    {
                        let mut pool = world.resource_mut::<GpuSlotPool>();
                        for _ in 0..bcount {
                            if let Some(slot) = pool.alloc_reset() {
                                building_slots.push(slot);
                            }
                        }
                    }
                    // Spawn ECS entities with Building + Health + ConstructionProgress
                    let mut entities_and_slots = Vec::with_capacity(bcount);
                    for (i, &slot) in building_slots.iter().enumerate() {
                        let x = 100.0 + (i % 224) as f32 * 32.0;
                        let y = 100.0 + (i / 224) as f32 * 32.0;
                        let entity = world
                            .spawn((
                                GpuSlot(slot),
                                Position { x, y },
                                Health(0.01),
                                Faction(1),
                                TownId(0),
                                Building {
                                    kind: world::BuildingKind::FarmerHome,
                                },
                                ConstructionProgress(5.0),
                            ))
                            .id();
                        entities_and_slots.push((entity, slot, x, y));
                    }
                    let mut em = world.resource_mut::<EntityMap>();
                    for &(entity, slot, x, y) in &entities_and_slots {
                        em.set_entity(slot, entity);
                        em.add_instance(BuildingInstance {
                            kind: world::BuildingKind::FarmerHome,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: 1,
                        });
                    }
                }
                // Warmup
                let _ = app.world_mut().run_system_once(construction_tick_system);
                b.iter(|| {
                    // Reset construction timers each iteration
                    let _ = app.world_mut().run_system_once(
                        |mut q: Query<&mut ConstructionProgress>| {
                            for mut cp in q.iter_mut() {
                                cp.0 = 5.0;
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(construction_tick_system);
                });
            },
        );
    }
    group.finish();
}

fn bench_energy_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("energy_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            // Set game_time unpaused with non-zero delta
            {
                let world = app.world_mut();
                let mut gt = world.resource_mut::<GameTime>();
                gt.time_scale = 1.0;
            }
            let _ = app.world_mut().run_system_once(energy_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(energy_system);
            });
        });
    }
    group.finish();
}

fn bench_arrival_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("arrival_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            {
                let world = app.world_mut();
                let mut gt = world.resource_mut::<GameTime>();
                gt.time_scale = 1.0;
            }
            let _ = app.world_mut().run_system_once(arrival_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(arrival_system);
            });
        });
    }
    group.finish();
}

fn bench_gpu_position_readback(c: &mut Criterion) {
    let mut group = c.benchmark_group("gpu_position_readback");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            let _ = app.world_mut().run_system_once(gpu_position_readback);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(gpu_position_readback);
            });
        });
    }
    group.finish();
}

fn bench_advance_waypoints_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("advance_waypoints_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            {
                let world = app.world_mut();
                let mut gt = world.resource_mut::<GameTime>();
                gt.time_scale = 1.0;
            }
            let _ = app.world_mut().run_system_once(advance_waypoints_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(advance_waypoints_system);
            });
        });
    }
    group.finish();
}

fn bench_cooldown_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("cooldown_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            // Set 50% of NPCs with active cooldowns (timer > 0)
            let _ = app.world_mut().run_system_once(
                |mut q: Query<(&GpuSlot, &mut AttackTimer), Without<Building>>| {
                    for (slot, mut timer) in q.iter_mut() {
                        if slot.0 % 2 == 0 {
                            timer.0 = 0.8;
                        }
                    }
                },
            );
            let _ = app.world_mut().run_system_once(cooldown_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(cooldown_system);
            });
        });
    }
    group.finish();
}

fn bench_npc_regen_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("npc_regen_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            // Give 25% of NPCs hp_regen > 0 and damage them
            let _ = app.world_mut().run_system_once(
                move |mut q: Query<
                    (&GpuSlot, &mut Health, &mut CachedStats),
                    Without<Building>,
                >| {
                    for (slot, mut hp, mut stats) in q.iter_mut() {
                        if slot.0 % 4 == 0 {
                            stats.hp_regen = 2.0;
                            hp.0 = 50.0;
                        }
                    }
                },
            );
            let _ = app.world_mut().run_system_once(npc_regen_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(npc_regen_system);
            });
        });
    }
    group.finish();
}

fn bench_on_duty_tick_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("on_duty_tick_system");
    for &count in COUNTS {
        group.bench_with_input(BenchmarkId::from_parameter(count), &count, |b, &count| {
            let mut app = build_bench_app();
            spawn_bench_town(&mut app);
            populate_npcs(&mut app, count);
            // Give all NPCs PatrolRoute (on_duty_tick filters With<PatrolRoute>)
            // In reality ~20% are guards, but bench worst-case
            let _ = app.world_mut().run_system_once(
                |mut commands: Commands, q: Query<Entity, Without<Building>>| {
                    for entity in q.iter() {
                        commands.entity(entity).insert(Activity {
                            kind: ActivityKind::Patrol,
                            ..Default::default()
                        });
                    }
                },
            );
            app.world_mut().flush();
            {
                let world = app.world_mut();
                let mut gt = world.resource_mut::<GameTime>();
                gt.time_scale = 1.0;
            }
            let _ = app.world_mut().run_system_once(on_duty_tick_system);
            b.iter(|| {
                let _ = app.world_mut().run_system_once(on_duty_tick_system);
            });
        });
    }
    group.finish();
}

fn bench_spawn_npc_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("spawn_npc_system");
    group.sample_size(20);
    // Scale by spawn count per frame (message-driven system)
    const SPAWN_COUNTS: &[usize] = &[100, 500];
    for &spawn_count in SPAWN_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(spawn_count),
            &spawn_count,
            |b, &spawn_count| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 1_000); // base world setup
                let _ = app.world_mut().run_system_once(spawn_npc_system);
                b.iter(|| {
                    // Inject spawn messages
                    let _ = app.world_mut().run_system_once(
                        move |mut writer: MessageWriter<SpawnNpcMsg>,
                              mut pool: ResMut<GpuSlotPool>| {
                            for i in 0..spawn_count {
                                let Some(slot) = pool.alloc_reset() else {
                                    break;
                                };
                                writer.write(SpawnNpcMsg {
                                    slot_idx: slot,
                                    x: (i % 100) as f32 * 16.0,
                                    y: (i / 100) as f32 * 16.0,
                                    job: 0, // Farmer
                                    faction: 1,
                                    town_idx: 0,
                                    home_x: 800.0,
                                    home_y: 800.0,
                                    work_x: -1.0,
                                    work_y: -1.0,
                                    starting_post: -1,
                                    entity_override: None,
                                });
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(spawn_npc_system);
                    app.world_mut().flush();
                });
            },
        );
    }
    group.finish();
}

fn bench_process_proj_hits(c: &mut Criterion) {
    let mut group = c.benchmark_group("process_proj_hits");
    group.sample_size(20);
    // Scale by active projectile count
    const PROJ_COUNTS: &[usize] = &[1_000, 50_000];
    for &proj_count in PROJ_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(proj_count),
            &proj_count,
            |b, &proj_count| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 5_000);
                // Allocate projectile slots and set up hit state
                {
                    let world = app.world_mut();
                    let mut proj_alloc = world.resource_mut::<ProjSlotAllocator>();
                    for _ in 0..proj_count {
                        proj_alloc.alloc();
                    }
                    // Set up ProjBufferWrites with active projectiles
                    let mut proj_writes = world.resource_mut::<ProjBufferWrites>();
                    proj_writes.active.resize(proj_count, 1);
                    proj_writes.damages.resize(proj_count, 10.0);
                    proj_writes.shooters.resize(proj_count, 0);
                    proj_writes.factions.resize(proj_count, 1);
                }
                let _ = app.world_mut().run_system_once(process_proj_hits);
                b.iter(|| {
                    // Populate hit state: 10% of projectiles hit a target
                    {
                        let world = app.world_mut();
                        let mut hit_state = world.resource_mut::<ProjHitState>();
                        hit_state.0.resize(proj_count, [0i32; 2]);
                        for i in 0..proj_count {
                            if i % 10 == 0 {
                                // Hit target NPC slot i % 5000
                                hit_state.0[i] = [(i % 5000) as i32, 0];
                            } else {
                                hit_state.0[i] = [-1, 0]; // no hit
                            }
                        }
                    }
                    let _ = app.world_mut().run_system_once(process_proj_hits);
                });
            },
        );
    }
    group.finish();
}

/// Benchmark `prune_town_equipment_system` at various item counts.
///
/// Growth rate context (50K NPCs, heavy combat):
///   - 25K enemy raiders with equipment_drop_rate = 0.30
///   - ~600 kills/hour -> 180 items/hour raw generation per town
///   - TOWN_EQUIPMENT_CAP = 200; prune fires hourly -> count capped at 200
///   - Memory at cap: ~120 bytes/item * 200 = ~24 KB per town (negligible)
///   - Without cap: 180 items/hr * 8 hours = 1,440 items -> ~170 KB (still small,
///     but O(n log n) prune cost grows with inventory backlog)
///   - Conclusion: inventory IS bounded by the prune cap; prune runs in <1ms even
///     at 10,000 items (50x cap), so the system is safe at 50K NPC scale.
fn bench_prune_town_equipment(c: &mut Criterion) {
    // Item counts: at cap, 2.5x, 5x, 10x, 50x cap
    const ITEM_COUNTS: &[usize] = &[200, 500, 1_000, 2_000, 10_000];
    let mut group = c.benchmark_group("prune_town_equipment");
    group.sample_size(20);

    for &item_count in ITEM_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(item_count),
            &item_count,
            |b, &item_count| {
                let mut app = build_bench_app();

                // Set up one town in WorldData
                {
                    let world = app.world_mut();
                    let mut wd = world.resource_mut::<world::WorldData>();
                    wd.towns.push(world::Town {
                        name: "PruneBenchTown".into(),
                        center: Vec2::ZERO,
                        faction: 1,
                        kind: TownKind::Player,
                    });
                }

                // Spawn town entity and register in TownIndex
                let entity = {
                    let world = app.world_mut();
                    world
                        .spawn((
                            TownMarker,
                            FoodStore(0),
                            GoldStore(0),
                            TownPolicy::default(),
                            TownUpgradeLevel::default(),
                            TownEquipment::default(),
                        ))
                        .id()
                };
                {
                    let world = app.world_mut();
                    let mut ti = world.resource_mut::<TownIndex>();
                    ti.0.insert(0, entity);
                }

                b.iter(|| {
                    // Reload inventory to item_count items before each prune
                    {
                        let world = app.world_mut();
                        let mut eq = world.get_mut::<TownEquipment>(entity).unwrap();
                        eq.0.clear();
                        for i in 0..item_count {
                            let rarity = match i % 4 {
                                0 => Rarity::Common,
                                1 => Rarity::Uncommon,
                                2 => Rarity::Rare,
                                _ => Rarity::Epic,
                            };
                            eq.0.push(LootItem {
                                id: i as u64,
                                kind: ItemKind::Weapon,
                                name: String::new(),
                                rarity,
                                stat_bonus: i as f32 * 0.001,
                                sprite: (0.0, 0.0),
                                weapon_type: None,
                            });
                        }
                        world.resource_mut::<GameTime>().hour_ticked = true;
                    }
                    let _ = app
                        .world_mut()
                        .run_system_once(stats::prune_town_equipment_system);
                });
            },
        );
    }
    group.finish();
}

// ── AI decision system benchmark (issue-192 stagger validation) ───

const AI_TOWN_COUNT: usize = 18;

/// Build a bench world with 18 AI towns and distributed NPCs.
fn spawn_ai_bench_world(app: &mut App, npc_count: usize) {
    let world = app.world_mut();

    {
        let mut grid = world.resource_mut::<world::WorldGrid>();
        grid.width = 100;
        grid.height = 100;
        grid.cell_size = TOWN_GRID_SPACING;
        grid.cells = vec![world::WorldCell::default(); 100 * 100];
        grid.init_town_buildable();
    }
    {
        let mut em = world.resource_mut::<EntityMap>();
        em.init_spatial(100.0 * TOWN_GRID_SPACING);
    }
    {
        let mut fl = world.resource_mut::<FactionList>();
        fl.factions.push(FactionData {
            kind: FactionKind::Neutral,
            name: "Neutral".into(),
            towns: vec![],
        });
        for i in 0..AI_TOWN_COUNT {
            fl.factions.push(FactionData {
                kind: FactionKind::AiBuilder,
                name: format!("AI_{i}"),
                towns: vec![i],
            });
        }
    }

    let personalities = [
        AiPersonality::Aggressive,
        AiPersonality::Balanced,
        AiPersonality::Economic,
    ];

    for i in 0..AI_TOWN_COUNT {
        let cx = (i % 6) as f32 * 1200.0 + 600.0;
        let cy = (i / 6) as f32 * 1200.0 + 600.0;
        let center = Vec2::new(cx, cy);
        let faction = (i + 1) as i32;

        let entity = world
            .spawn((
                TownMarker,
                FoodStore(100_000),
                GoldStore(100_000),
                WoodStore(0),
                StoneStore(0),
                TownPolicy::default(),
                TownUpgradeLevel::default(),
                TownEquipment::default(),
                TownAreaLevel(1),
            ))
            .id();

        world.resource_mut::<TownIndex>().0.insert(i as i32, entity);

        world
            .resource_mut::<world::WorldData>()
            .towns
            .push(world::Town {
                name: format!("Town_{i}"),
                center,
                faction,
                kind: TownKind::AiBuilder,
            });

        let fountain_slot = world.resource_mut::<GpuSlotPool>().alloc_reset().unwrap();
        world
            .resource_mut::<EntityMap>()
            .add_instance(endless::entity_map::BuildingInstance {
                kind: world::BuildingKind::Fountain,
                position: center,
                town_idx: i as u32,
                slot: fountain_slot,
                faction,
            });
    }

    {
        let mut ai_state = world.resource_mut::<AiPlayerState>();
        for i in 0..AI_TOWN_COUNT {
            ai_state.players.push(AiPlayer {
                town_data_idx: i,
                kind: AiKind::Builder,
                personality: personalities[i % 3],
                road_style: RoadStyle::None,
                last_actions: Default::default(),
                policy_defaults_logged: false,
                active: true,
                build_enabled: true,
                upgrade_enabled: true,
                squad_indices: Vec::new(),
                squad_cmd: Default::default(),
                decision_timer: 0.0,
            });
        }
    }

    let per_town = npc_count / AI_TOWN_COUNT;
    let mut slots = Vec::with_capacity(npc_count);
    {
        let mut pool = world.resource_mut::<GpuSlotPool>();
        for _ in 0..npc_count {
            if let Some(slot) = pool.alloc_reset() {
                slots.push(slot);
            }
        }
    }

    let mut entity_slots: Vec<(Entity, usize, i32)> = Vec::with_capacity(npc_count);
    for (idx, &slot) in slots.iter().enumerate() {
        let town_idx = (idx / per_town).min(AI_TOWN_COUNT - 1);
        let faction = (town_idx + 1) as i32;
        let cx = (town_idx % 6) as f32 * 1200.0 + 600.0;
        let cy = (town_idx / 6) as f32 * 1200.0 + 600.0;
        let local_i = idx % per_town;
        let x = cx + (local_i % 50) as f32 * 16.0 - 400.0;
        let y = cy + (local_i / 50) as f32 * 16.0 - 400.0;

        let entity = world
            .spawn((
                (
                    GpuSlot(slot),
                    Position { x, y },
                    Health(100.0),
                    Job::Farmer,
                    Faction(faction),
                    TownId(town_idx as i32),
                    Activity::default(),
                    CombatState::default(),
                    Energy(100.0),
                    Speed(60.0),
                    Home(Vec2::new(cx, cy)),
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
                        wood: 0,
                        stone: 0,
                        equipment: vec![],
                    },
                    Personality::default(),
                    FleeThreshold { pct: 0.2 },
                    LeashRange(400.0),
                    WoundedThreshold { pct: 0.3 },
                    HasEnergy,
                    NpcEquipment::default(),
                    SquadId(0),
                    NpcPath::default(),
                ),
            ))
            .id();
        entity_slots.push((entity, slot, town_idx as i32));
    }

    {
        let mut em = world.resource_mut::<EntityMap>();
        for &(entity, slot, town_idx) in &entity_slots {
            let faction = town_idx + 1;
            em.register_npc(slot, entity, Job::Farmer, faction, town_idx);
        }
    }

    let max_slot = slots.iter().copied().max().unwrap_or(0) + AI_TOWN_COUNT + 1;
    {
        let mut gpu_read = world.resource_mut::<GpuReadState>();
        gpu_read.positions.resize(max_slot * 2, 0.0);
        gpu_read.combat_targets.resize(max_slot, -1);
        gpu_read.health.resize(max_slot, 1.0);
        gpu_read.factions.resize(max_slot, 1);
        gpu_read.threat_counts.resize(max_slot, 0);
        gpu_read.npc_count = npc_count;
    }
    {
        let mut gpu_state = world.resource_mut::<EntityGpuState>();
        for (idx, &slot) in slots.iter().enumerate() {
            let town_idx = (idx / per_town).min(AI_TOWN_COUNT - 1);
            let cx = (town_idx % 6) as f32 * 1200.0 + 600.0;
            let cy = (town_idx / 6) as f32 * 1200.0 + 600.0;
            let local_i = idx % per_town;
            let x = cx + (local_i % 50) as f32 * 16.0 - 400.0;
            let y = cy + (local_i / 50) as f32 * 16.0 - 400.0;
            gpu_state.positions[slot * 2] = x;
            gpu_state.positions[slot * 2 + 1] = y;
            gpu_state.factions[slot] = (town_idx + 1) as i32;
            gpu_state.healths[slot] = 1.0;
            gpu_state.max_healths[slot] = 100.0;
            gpu_state.speeds[slot] = 60.0;
        }
    }
    {
        let mut pop = world.resource_mut::<PopulationStats>();
        for i in 0..AI_TOWN_COUNT {
            pop.0.insert(
                (Job::Farmer as i32, i as i32),
                PopStats {
                    alive: per_town as i32,
                    working: 0,
                    dead: 0,
                },
            );
        }
    }
}

fn bench_ai_decision_system(c: &mut Criterion) {
    let npc_count = 50_000;
    let mut group = c.benchmark_group("ai_decision_system");
    group.sample_size(20);

    // Staggered: only 1 town fires per tick (post-fix behavior)
    group.bench_with_input(
        BenchmarkId::new("staggered", npc_count),
        &npc_count,
        |b, &count| {
            let mut app = build_bench_app();
            app.init_resource::<AiSnapshotDirty>();
            spawn_ai_bench_world(&mut app, count);

            {
                let world = app.world_mut();
                let mut ai_state = world.resource_mut::<AiPlayerState>();
                for (i, p) in ai_state.players.iter_mut().enumerate() {
                    p.decision_timer = if i == 0 {
                        DEFAULT_AI_INTERVAL
                    } else {
                        i as f32 * DEFAULT_AI_INTERVAL / AI_TOWN_COUNT as f32
                    };
                }
            }

            // Prime snapshot cache
            let _ = app.world_mut().run_system_once(ai_decision_system);

            b.iter(|| {
                {
                    let world = app.world_mut();
                    let mut ai_state = world.resource_mut::<AiPlayerState>();
                    for (i, p) in ai_state.players.iter_mut().enumerate() {
                        p.decision_timer = if i == 0 {
                            DEFAULT_AI_INTERVAL
                        } else {
                            i as f32 * DEFAULT_AI_INTERVAL / AI_TOWN_COUNT as f32
                        };
                    }
                }
                let _ = app.world_mut().run_system_once(ai_decision_system);
            });
        },
    );

    // Burst: all 18 towns fire simultaneously (pre-fix regression)
    group.bench_with_input(
        BenchmarkId::new("burst", npc_count),
        &npc_count,
        |b, &count| {
            let mut app = build_bench_app();
            app.init_resource::<AiSnapshotDirty>();
            spawn_ai_bench_world(&mut app, count);

            {
                let world = app.world_mut();
                let mut ai_state = world.resource_mut::<AiPlayerState>();
                for p in ai_state.players.iter_mut() {
                    p.decision_timer = DEFAULT_AI_INTERVAL;
                }
            }

            let _ = app.world_mut().run_system_once(ai_decision_system);

            b.iter(|| {
                {
                    let world = app.world_mut();
                    let mut ai_state = world.resource_mut::<AiPlayerState>();
                    for p in ai_state.players.iter_mut() {
                        p.decision_timer = DEFAULT_AI_INTERVAL;
                    }
                }
                let _ = app.world_mut().run_system_once(ai_decision_system);
            });
        },
    );

    group.finish();
}

// ── Building grid rebuild benchmarks (issue-207 spike investigation) ──────────

/// Populate EntityMap with a mix of wall and tower buildings at realistic grid positions.
/// Used for rebuild_building_grid and sync_pathfind_costs benchmarks.
/// World is 200x200 cells at TOWN_GRID_SPACING = 64px.
fn populate_pathfind_buildings(app: &mut App, count: usize) {
    // Resize world to 200x200 for a realistic map size
    {
        let world = app.world_mut();
        let mut grid = world.resource_mut::<world::WorldGrid>();
        grid.width = 200;
        grid.height = 200;
        grid.cell_size = TOWN_GRID_SPACING;
        grid.cells = vec![world::WorldCell::default(); 200 * 200];
    }
    {
        let world = app.world_mut();
        let mut em = world.resource_mut::<EntityMap>();
        let world_size_px = 200.0 * TOWN_GRID_SPACING;
        em.init_spatial(world_size_px);
    }

    // 75% walls, 25% bow towers -- representative mix for pathfinding cost benchmarks
    let wall_count = count * 3 / 4;

    let world = app.world_mut();
    let mut building_slots = Vec::with_capacity(count);
    {
        let mut pool = world.resource_mut::<GpuSlotPool>();
        for _ in 0..count {
            if let Some(slot) = pool.alloc_reset() {
                building_slots.push(slot);
            }
        }
    }

    let mut building_entities = Vec::with_capacity(count);
    for (i, &slot) in building_slots.iter().enumerate() {
        // Spread buildings across the 200x200 grid
        let gc = (i % 200) as f32;
        let gr = (i / 200 % 200) as f32;
        let x = gc * TOWN_GRID_SPACING + 32.0;
        let y = gr * TOWN_GRID_SPACING + 32.0;
        let kind = if i < wall_count {
            world::BuildingKind::Wall
        } else {
            world::BuildingKind::BowTower
        };
        let entity = world
            .spawn((
                GpuSlot(slot),
                Position { x, y },
                Health(100.0),
                Faction(1),
                TownId(0),
                Building { kind },
            ))
            .id();
        building_entities.push((entity, slot, x, y, kind));
    }

    let mut em = world.resource_mut::<EntityMap>();
    for &(entity, slot, x, y, kind) in &building_entities {
        em.set_entity(slot, entity);
        em.add_instance(BuildingInstance {
            kind,
            position: Vec2::new(x, y),
            town_idx: 0,
            slot,
            faction: 1,
        });
    }
}

/// Benchmark `rebuild_building_grid_system` -- the spatial grid full rebuild.
/// Fires on every BuildingGridDirtyMsg (building placed, destroyed, or loaded).
/// Tests `entity_map.init_spatial() + rebuild_spatial()` cost at realistic building counts.
fn bench_rebuild_building_grid_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("rebuild_building_grid");
    group.sample_size(20);
    const BUILDING_COUNTS: &[usize] = &[500, 2_000, 5_000];
    for &bcount in BUILDING_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_pathfind_buildings(&mut app, bcount);
                // Warmup
                let _ = app.world_mut().run_system_once(
                    |mut writer: MessageWriter<BuildingGridDirtyMsg>| {
                        writer.write(BuildingGridDirtyMsg);
                    },
                );
                let _ = app
                    .world_mut()
                    .run_system_once(world::rebuild_building_grid_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(
                        |mut writer: MessageWriter<BuildingGridDirtyMsg>| {
                            writer.write(BuildingGridDirtyMsg);
                        },
                    );
                    let _ = app
                        .world_mut()
                        .run_system_once(world::rebuild_building_grid_system);
                });
            },
        );
    }
    group.finish();
}

/// Benchmark `sync_pathfind_costs_system` -- HPA* incremental chunk rebuild.
/// Fires on every BuildingGridDirtyMsg. Tests `grid.sync_building_costs()` + HPA*
/// `rebuild_chunks()` cost for wall/tower buildings spread across the grid.
fn bench_sync_pathfind_costs_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("sync_pathfind_costs");
    group.sample_size(20);
    const BUILDING_COUNTS: &[usize] = &[500, 2_000, 5_000];
    for &bcount in BUILDING_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_pathfind_buildings(&mut app, bcount);
                // Initialize terrain costs and HPA* cache
                {
                    let world = app.world_mut();
                    let mut grid = world.resource_mut::<world::WorldGrid>();
                    grid.init_pathfind_costs();
                }
                // Warmup
                let _ = app.world_mut().run_system_once(
                    |mut writer: MessageWriter<BuildingGridDirtyMsg>| {
                        writer.write(BuildingGridDirtyMsg);
                    },
                );
                let _ = app
                    .world_mut()
                    .run_system_once(endless::systems::pathfinding::sync_pathfind_costs_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(
                        |mut writer: MessageWriter<BuildingGridDirtyMsg>| {
                            writer.write(BuildingGridDirtyMsg);
                        },
                    );
                    let _ = app
                        .world_mut()
                        .run_system_once(endless::systems::pathfinding::sync_pathfind_costs_system);
                });
            },
        );
    }
    group.finish();
}

/// Benchmark farm_visual_system: event-driven marker spawn/despawn.
/// Tests steady-state (no events), post-load (Added), and harvest (FarmHarvestedMsg).
fn bench_farm_visual_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("farm_visual_system");
    group.sample_size(20);
    const BUILDING_COUNTS: &[usize] = &[1_000, 5_000];
    for &bcount in BUILDING_COUNTS {
        // Steady state: no events, all queries empty
        group.bench_with_input(
            BenchmarkId::new("steady_state", bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                app.add_message::<FarmReadyMsg>();
                app.add_message::<FarmHarvestedMsg>();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 100);
                populate_growable_buildings(&mut app, bcount);
                // Warmup: first run processes Added<ProductionState>
                let _ = app.world_mut().run_system_once(farm_visual_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(farm_visual_system);
                });
            },
        );

        // Burst ready: growth_system fires FarmReadyMsg for 25% of buildings
        group.bench_with_input(
            BenchmarkId::new("burst_ready", bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                app.add_message::<FarmReadyMsg>();
                app.add_message::<FarmHarvestedMsg>();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 100);
                populate_growable_buildings(&mut app, bcount);
                let _ = app.world_mut().run_system_once(farm_visual_system);
                let msg_count = bcount / 4;
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(
                        move |mut writer: MessageWriter<FarmReadyMsg>,
                              q: Query<&GpuSlot, With<Building>>| {
                            for slot in q.iter().take(msg_count) {
                                writer.write(FarmReadyMsg { slot: slot.0 });
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(farm_visual_system);
                });
            },
        );

        // Burst harvest: FarmHarvestedMsg for 25% of buildings
        group.bench_with_input(
            BenchmarkId::new("burst_harvest", bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                app.add_message::<FarmReadyMsg>();
                app.add_message::<FarmHarvestedMsg>();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 100);
                populate_growable_buildings(&mut app, bcount);
                let _ = app.world_mut().run_system_once(farm_visual_system);
                let msg_count = bcount / 4;
                b.iter(|| {
                    // First make them ready, then harvest
                    let _ = app.world_mut().run_system_once(
                        move |mut writer: MessageWriter<FarmReadyMsg>,
                              q: Query<&GpuSlot, With<Building>>| {
                            for slot in q.iter().take(msg_count) {
                                writer.write(FarmReadyMsg { slot: slot.0 });
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(farm_visual_system);
                    let _ = app.world_mut().run_system_once(
                        move |mut writer: MessageWriter<FarmHarvestedMsg>,
                              q: Query<&GpuSlot, With<Building>>| {
                            for slot in q.iter().take(msg_count) {
                                writer.write(FarmHarvestedMsg { slot: slot.0 });
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(farm_visual_system);
                });
            },
        );
    }
    group.finish();
}

/// Benchmark `build_building_body_instances` at realistic building counts.
///
/// Building slots start at MAX_NPC_COUNT (100K) in the unified slot namespace,
/// so each gpu_state array access was a scattered read into a 200K-element array.
/// The fix uses BuildingInstance.position/faction (compact DenseSlotMap) and
/// a pre-built construction index, reducing scattered reads from 5 to 2 per building.
fn bench_build_building_body_instances(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_building_body_instances");
    group.sample_size(20);
    // Realistic range: 500 to 5K buildings. Issue observed at ~1111 NPCs + buildings.
    const BUILDING_COUNTS: &[usize] = &[500, 2_000, 5_000];
    for &bcount in BUILDING_COUNTS {
        group.bench_with_input(
            BenchmarkId::from_parameter(bcount),
            &bcount,
            |b, &bcount| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                app.init_resource::<BuildingBodyInstances>();
                app.insert_resource(BuildingBodyDirty {
                    dirty: true,
                    had_building_flash: false,
                    last_building_count: 0,
                });

                // Allocate slots in the building range (after NPC slots)
                let mut building_slots = Vec::with_capacity(bcount);
                {
                    let world = app.world_mut();
                    let mut pool = world.resource_mut::<GpuSlotPool>();
                    for _ in 0..bcount {
                        if let Some(slot) = pool.alloc_reset() {
                            building_slots.push(slot);
                        }
                    }
                }

                // Set sprite indices so col >= 0.0 (skip-guard passes for all buildings)
                {
                    let world = app.world_mut();
                    let mut gpu = world.resource_mut::<EntityGpuState>();
                    for &slot in &building_slots {
                        gpu.sprite_indices[slot * 4] = 3.0; // col
                        gpu.sprite_indices[slot * 4 + 1] = 0.0; // row
                        gpu.sprite_indices[slot * 4 + 2] = 1.0; // atlas
                        gpu.healths[slot] = 1.0;
                    }
                }

                // Register buildings in EntityMap with realistic positions
                {
                    let world = app.world_mut();
                    let mut em = world.resource_mut::<EntityMap>();
                    for (i, &slot) in building_slots.iter().enumerate() {
                        let x = 400.0 + (i % 100) as f32 * 64.0;
                        let y = 400.0 + (i / 100) as f32 * 64.0;
                        em.add_instance(endless::entity_map::BuildingInstance {
                            kind: world::BuildingKind::BowTower,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: crate::FACTION_PLAYER,
                        });
                    }
                }

                // Warmup (sets dirty=false; reset before each iter)
                let _ = app
                    .world_mut()
                    .run_system_once(build_building_body_instances);
                b.iter(|| {
                    // Reset dirty so the system doesn't skip on subsequent iterations
                    app.world_mut().resource_mut::<BuildingBodyDirty>().dirty = true;
                    let _ = app
                        .world_mut()
                        .run_system_once(build_building_body_instances);
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
    bench_growth_system,
    bench_construction_tick_system,
    bench_energy_system,
    bench_arrival_system,
    bench_gpu_position_readback,
    bench_advance_waypoints_system,
    bench_cooldown_system,
    bench_npc_regen_system,
    bench_on_duty_tick_system,
    bench_spawn_npc_system,
    bench_process_proj_hits,
    bench_prune_town_equipment,
    bench_ai_decision_system,
    bench_rebuild_building_grid_system,
    bench_sync_pathfind_costs_system,
    bench_farm_visual_system,
    bench_sync_sleeping_system,
    bench_build_building_body_instances,
    bench_resolve_work_targets,
);
criterion_main!(benches);

/// Populate farm/mine buildings for work targeting benchmarks.
fn populate_work_buildings(app: &mut App, farm_count: usize, mine_count: usize) {
    let total = farm_count + mine_count;
    let world = app.world_mut();
    let mut building_slots = Vec::with_capacity(total);
    {
        let mut pool = world.resource_mut::<GpuSlotPool>();
        for _ in 0..total {
            if let Some(slot) = pool.alloc_reset() {
                building_slots.push(slot);
            }
        }
    }
    let mut building_entities = Vec::with_capacity(total);
    for (i, &slot) in building_slots.iter().enumerate() {
        let x = 200.0 + (i % 100) as f32 * 32.0;
        let y = 200.0 + (i / 100) as f32 * 32.0;
        let is_farm = i < farm_count;
        let kind = if is_farm {
            world::BuildingKind::Farm
        } else {
            world::BuildingKind::GoldMine
        };
        let mut cmd = world.spawn((
            GpuSlot(slot),
            Position { x, y },
            Health(100.0),
            Faction(1),
            TownId(0),
            Building { kind },
            ProductionState {
                ready: i % 2 == 0,
                progress: if i % 2 == 0 { 1.0 } else { 0.5 },
            },
        ));
        if is_farm {
            cmd.insert(FarmModeComp(FarmMode::Crops));
        }
        let entity = cmd.id();
        building_entities.push((entity, slot, x, y, kind));
    }
    let mut em = world.resource_mut::<EntityMap>();
    for &(entity, slot, x, y, kind) in &building_entities {
        em.set_entity(slot, entity);
        em.add_instance(endless::entity_map::BuildingInstance {
            kind,
            position: Vec2::new(x, y),
            town_idx: 0,
            slot,
            faction: 1,
        });
    }
}

fn bench_resolve_work_targets(c: &mut Criterion) {
    let mut group = c.benchmark_group("resolve_work_targets");
    group.sample_size(20);

    const BUILDING_COUNTS: &[usize] = &[500, 2_000, 1_000];
    const CLAIM_COUNTS: &[usize] = &[50, 200, 200];
    const RESOURCE_NODE_COUNTS: &[usize] = &[0, 0, 65_000];

    for (idx, (&building_count, &claim_count)) in
        BUILDING_COUNTS.iter().zip(CLAIM_COUNTS.iter()).enumerate()
    {
        let resource_nodes = RESOURCE_NODE_COUNTS[idx];
        let label = if resource_nodes > 0 {
            format!("{}_bld_{}k_nodes", building_count, resource_nodes / 1000)
        } else {
            format!("{}", building_count)
        };

        group.bench_with_input(
            BenchmarkId::new("burst_claim", &label),
            &building_count,
            |b, &bcount| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, claim_count);
                populate_work_buildings(&mut app, bcount / 2, bcount / 2);
                if resource_nodes > 0 {
                    let world = app.world_mut();
                    let mut node_slots = Vec::with_capacity(resource_nodes);
                    {
                        let mut pool = world.resource_mut::<GpuSlotPool>();
                        for _ in 0..resource_nodes {
                            if let Some(slot) = pool.alloc_reset() {
                                node_slots.push(slot);
                            }
                        }
                    }
                    let mut em = world.resource_mut::<EntityMap>();
                    for (i, &slot) in node_slots.iter().enumerate() {
                        let x = 50.0 + (i % 400) as f32 * 40.0;
                        let y = 50.0 + (i / 400) as f32 * 40.0;
                        let kind = if i % 2 == 0 {
                            world::BuildingKind::TreeNode
                        } else {
                            world::BuildingKind::RockNode
                        };
                        em.add_instance(endless::entity_map::BuildingInstance {
                            kind,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: 0,
                        });
                    }
                }
                app.world_mut().flush();

                let npc_entities: Vec<Entity> = app
                    .world_mut()
                    .run_system_once(
                        |q: Query<Entity, (Without<Building>, With<NpcWorkState>)>| {
                            q.iter().collect::<Vec<_>>()
                        },
                    )
                    .unwrap_or_default();

                let _ = app.world_mut().run_system_once(resolve_work_targets);

                b.iter(|| {
                    let entities = npc_entities.clone();
                    let _ = app.world_mut().run_system_once(
                        move |mut writer: MessageWriter<WorkIntentMsg>| {
                            for &entity in &entities {
                                writer.write(WorkIntentMsg(WorkIntent::Claim {
                                    entity,
                                    kind: world::BuildingKind::Farm,
                                    town_idx: 0,
                                    from: Vec2::new(800.0, 800.0),
                                }));
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(resolve_work_targets);
                    let _ = app
                        .world_mut()
                        .run_system_once(|mut em: ResMut<EntityMap>| {
                            for slot in em.iter_instances().map(|i| i.slot).collect::<Vec<_>>() {
                                em.set_occupancy(slot, 0);
                            }
                        });
                });
            },
        );

        group.bench_with_input(
            BenchmarkId::new("release_only", &label),
            &building_count,
            |b, &bcount| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, claim_count);
                populate_work_buildings(&mut app, bcount / 2, bcount / 2);
                if resource_nodes > 0 {
                    let world = app.world_mut();
                    let mut node_slots = Vec::with_capacity(resource_nodes);
                    {
                        let mut pool = world.resource_mut::<GpuSlotPool>();
                        for _ in 0..resource_nodes {
                            if let Some(slot) = pool.alloc_reset() {
                                node_slots.push(slot);
                            }
                        }
                    }
                    let mut em = world.resource_mut::<EntityMap>();
                    for (i, &slot) in node_slots.iter().enumerate() {
                        let x = 50.0 + (i % 400) as f32 * 40.0;
                        let y = 50.0 + (i / 400) as f32 * 40.0;
                        let kind = if i % 2 == 0 {
                            world::BuildingKind::TreeNode
                        } else {
                            world::BuildingKind::RockNode
                        };
                        em.add_instance(endless::entity_map::BuildingInstance {
                            kind,
                            position: Vec2::new(x, y),
                            town_idx: 0,
                            slot,
                            faction: 0,
                        });
                    }
                }
                app.world_mut().flush();

                let npc_entities: Vec<Entity> = app
                    .world_mut()
                    .run_system_once(
                        |q: Query<Entity, (Without<Building>, With<NpcWorkState>)>| {
                            q.iter().collect::<Vec<_>>()
                        },
                    )
                    .unwrap_or_default();

                let _ = app.world_mut().run_system_once(resolve_work_targets);

                b.iter(|| {
                    let entities = npc_entities.clone();
                    let _ = app.world_mut().run_system_once(
                        move |mut writer: MessageWriter<WorkIntentMsg>| {
                            for &entity in &entities {
                                writer.write(WorkIntentMsg(WorkIntent::Release {
                                    entity,
                                    worksite: None,
                                }));
                            }
                        },
                    );
                    let _ = app.world_mut().run_system_once(resolve_work_targets);
                });
            },
        );
    }
    group.finish();
}

fn bench_build_building_body_instances(c: &mut Criterion) {
    let mut group = c.benchmark_group("build_building_body_instances");
    group.sample_size(20);
    const INSTANCE_COUNT: usize = 68_000;

    let setup_app = || {
        let mut app = build_bench_app();
        app.insert_resource(BuildingBodyInstances::default());
        app.insert_resource(BuildingBodyDirty::default());

        let world = app.world_mut();
        let mut slots = Vec::with_capacity(INSTANCE_COUNT);
        {
            let mut pool = world.resource_mut::<GpuSlotPool>();
            for _ in 0..INSTANCE_COUNT {
                if let Some(slot) = pool.alloc_reset() {
                    slots.push(slot);
                }
            }
        }
        {
            let mut em = world.resource_mut::<EntityMap>();
            for (i, &slot) in slots.iter().enumerate() {
                let x = 400.0 + (i % 260) as f32 * 32.0;
                let y = 400.0 + (i / 260) as f32 * 32.0;
                em.add_instance(BuildingInstance {
                    kind: world::BuildingKind::TreeNode,
                    position: Vec2::new(x, y),
                    town_idx: 0,
                    slot,
                    faction: 0,
                });
            }
        }
        {
            let mut gpu = world.resource_mut::<EntityGpuState>();
            for &slot in &slots {
                gpu.positions[slot * 2] = 400.0 + (slot % 260) as f32 * 32.0;
                gpu.positions[slot * 2 + 1] = 400.0 + (slot / 260) as f32 * 32.0;
                gpu.sprite_indices[slot * 4] = 1.0;
                gpu.sprite_indices[slot * 4 + 1] = 0.0;
                gpu.sprite_indices[slot * 4 + 2] = 1.0;
                gpu.healths[slot] = 1.0;
            }
        }
        app
    };

    group.bench_with_input(
        BenchmarkId::new("dirty", INSTANCE_COUNT),
        &INSTANCE_COUNT,
        |b, _| {
            let mut app = setup_app();
            b.iter(|| {
                app.world_mut().resource_mut::<BuildingBodyDirty>().dirty = true;
                let _ = app
                    .world_mut()
                    .run_system_once(build_building_body_instances);
            });
        },
    );

    group.bench_with_input(
        BenchmarkId::new("clean", INSTANCE_COUNT),
        &INSTANCE_COUNT,
        |b, _| {
            let mut app = setup_app();
            app.world_mut().resource_mut::<BuildingBodyDirty>().dirty = false;
            b.iter(|| {
                let _ = app
                    .world_mut()
                    .run_system_once(build_building_body_instances);
            });
        },
    );

    group.finish();
}

// ── sync_sleeping_system benchmark (issue-188, event-driven) ──────

/// Populate `count` TreeNode/RockNode buildings with Sleeping marker.
fn populate_resource_nodes(app: &mut App, count: usize) -> Vec<usize> {
    let world = app.world_mut();
    let mut slots = Vec::with_capacity(count);
    {
        let mut pool = world.resource_mut::<GpuSlotPool>();
        for _ in 0..count {
            if let Some(slot) = pool.alloc_reset() {
                slots.push(slot);
            }
        }
    }
    let mut building_entities = Vec::with_capacity(count);
    for (i, &slot) in slots.iter().enumerate() {
        let x = 100.0 + (i % 224) as f32 * 32.0;
        let y = 100.0 + (i / 224) as f32 * 32.0;
        let kind = if i % 2 == 0 {
            world::BuildingKind::TreeNode
        } else {
            world::BuildingKind::RockNode
        };
        let entity = world
            .spawn((
                GpuSlot(slot),
                Position { x, y },
                Health(100.0),
                Faction(1),
                TownId(0),
                Building { kind },
                Sleeping,
            ))
            .id();
        building_entities.push((entity, slot, x, y, kind));
    }
    let mut em = world.resource_mut::<EntityMap>();
    for &(entity, slot, x, y, kind) in &building_entities {
        em.set_entity(slot, entity);
        em.add_instance(BuildingInstance {
            kind,
            position: Vec2::new(x, y),
            town_idx: 0,
            slot,
            faction: 1,
        });
    }
    slots
}

fn bench_sync_sleeping_system(c: &mut Criterion) {
    let mut group = c.benchmark_group("sync_sleeping_system");
    group.sample_size(20);
    const NODE_COUNTS: &[usize] = &[1_000, 65_000];

    for &node_count in NODE_COUNTS {
        group.bench_with_input(
            BenchmarkId::new("idle", node_count),
            &node_count,
            |b, &node_count| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 100);
                let _slots = populate_resource_nodes(&mut app, node_count);
                let _ = app.world_mut().run_system_once(sync_sleeping_system);
                b.iter(|| {
                    let _ = app.world_mut().run_system_once(sync_sleeping_system);
                });
            },
        );
    }

    const DIRTY_COUNTS: &[usize] = &[5, 100, 1_000];
    for &dirty_count in DIRTY_COUNTS {
        group.bench_with_input(
            BenchmarkId::new("dirty", dirty_count),
            &dirty_count,
            |b, &dirty_count| {
                let mut app = build_bench_app();
                spawn_bench_town(&mut app);
                populate_npcs(&mut app, 100);
                let slots = populate_resource_nodes(&mut app, 65_000);
                let _ = app.world_mut().run_system_once(sync_sleeping_system);
                b.iter(|| {
                    {
                        let world = app.world_mut();
                        let mut em = world.resource_mut::<EntityMap>();
                        for &slot in slots.iter().take(dirty_count) {
                            em.sleeping_dirty.push(slot);
                            em.set_present(slot, 1);
                        }
                    }
                    let _ = app.world_mut().run_system_once(sync_sleeping_system);
                    app.world_mut().flush();
                    let _ = app.world_mut().run_system_once(
                        move |mut commands: Commands,
                              q: Query<(Entity, &GpuSlot), (With<Building>, Without<Sleeping>)>,
                              mut em: ResMut<EntityMap>| {
                            let mut restored = 0usize;
                            for (entity, gpu_slot) in q.iter() {
                                if restored >= dirty_count {
                                    break;
                                }
                                commands.entity(entity).insert(Sleeping);
                                em.set_present(gpu_slot.0, 0);
                                restored += 1;
                            }
                        },
                    );
                    app.world_mut().flush();
                });
            },
        );
    }
    group.finish();
}
