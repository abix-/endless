# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
2. **During coding**: Follow the patterns documented here (job-as-template spawn, chained combat, GPU buffer layout). If you need to deviate, update the doc first.
3. **After coding**: Update the doc if the architecture changed. Add new known issues discovered during implementation.
4. **Code comments** stay educational (explain what code does for someone learning Rust/WGSL). Architecture diagrams, data flow, buffer layouts, and system interaction docs live here, not in code.
5. **README.md** is the game intro (description, gameplay, controls). **docs/roadmap.md** has stages (priority order) and capabilities (feature inventory) — read its maintenance guide before editing.

## Test Framework

UI-selectable integration tests run inside the full Bevy app via a bevy_egui menu. From `AppState::MainMenu`, click "Debug Tests" to enter `AppState::TestMenu` which shows the test list; clicking a test transitions to `AppState::Running` where game systems execute normally. "Back to Menu" returns to MainMenu. Tests use **phased assertions** — each phase checks one pipeline layer and fails fast with diagnostic values.

**Architecture** (`rust/src/tests/`):
- `TestState` resource: shared phase tracking, counters, flags, results
- `TestRegistry`: registered test entries (name, description, phase_count, time_scale)
- `TestSetupParams`: SystemParam bundle for test setup (slot alloc, spawn, world data, food, factions, game time, test state, spawner state)
- `test_is("name")` run condition gates per-test setup/tick systems
- Each test exports `setup` (OnEnter Running) + `tick` (Update after Behavior)
- Helpers: `tick_elapsed()`, `require_entity()` reduce boilerplate
- Cleanup on OnExit(Running): despawn all NPC + FarmReadyMarker + tilemap chunk entities, reset all resources (including TilemapSpawned)
- Run All: sequential execution via `RunAllState` queue (auto-advances after 1.5s)
- Single tests stay running after pass/fail — user clicks Back in HUD to return

**HUD**: Phase checklist overlay during test execution — gray `○` pending, yellow `▶` active, green `✓` passed, red `✗` failed. Back/Cancel button at bottom.

**Tests** (`src/tests/`):

| Test | Phases | What it validates |
|------|--------|-------------------|
| `vertical-slice` | 8 | Full core loop: spawn → work → raid → combat → death → respawn |
| `spawning` | 4 | Spawn entities, kill via health=0, slot freed, slot reused |
| `energy` | 3 | Energy starts at 100, drains over time, reaches ENERGY_HUNGRY |
| `movement` | 3 | Transit activity set, GPU positions update, AtDestination on arrival |
| `archer-patrol` | 5 | OnDuty → Patrolling → OnDuty → rest when tired → resume |
| `farmer-cycle` | 5 | GoingToWork → Working → tired → rest → recover → return |
| `raider-cycle` | 5 | Dispatch group → arrive at farm → steal → return → deliver |
| `combat` | 6 | GPU targeting → Fighting → damage → health drop → death → slot freed |
| `projectiles` | 4 | Ranged targeting → projectile spawn → hit + damage → slot freed |
| `healing` | 3 | Damaged NPC near town → Healing marker → health recovers to max |
| `economy` | 5 | Farm growing → ready → harvest → camp forage → tent spawner respawn |
| `world-gen` | 6 | Grid dimensions, town placement, buildings, terrain, camps |
| `sleep-visual` | 3 | Resting NPC gets sleep icon (atlas_id=3.0) on status layer, cleared on wake |
| `farm-visual` | 3 | Ready farm spawns FarmReadyMarker, removed on harvest |
| `heal-visual` | 3 | Healing NPC gets halo (atlas_id=2.0) on healing layer, cleared when healed |
| `npc-visuals` | 1 | Visual showcase: all NPC types in labeled grid with individual layer breakdown |
| `terrain-visual` | 1 | Visual showcase: all terrain biomes and building types in labeled grid |
| `friendly-fire-buildings` | 4 | Ranged shooter fires through friendly farm wall without damaging same-faction buildings |

## System Map

```
Pure Bevy App (main.rs)
    │
    ▼
Bevy ECS (lib.rs build_app)
    │
    ├─ AppState: MainMenu → Playing | TestMenu → Running
    │
    ├─ UI (ui/) ─────────────────────────────▶ main_menu, game_hud, panels, startup, cleanup
    │   ├─ Main menu: difficulty presets + world config sliders + Play / Load Game / Debug Tests / Restart Tutorial
    │   ├─ Game startup: world gen + NPC spawn (OnEnter Playing)
    │   ├─ Top bar: panel toggles left, town name + time center, stats (food + gold) + FPS right
    │   ├─ Floating windows: NPC/building inspector with combat stats + equipment + status (bottom-left) + combat log with faction filter + type filters (bottom-right)
    │   ├─ Left panel: floating Window with Roster (R) / Upgrades (U) / Policies (P) / Patrols (T) / Squads (Q) / Factions (I) / Help (H)
    │   ├─ Jukebox overlay: top-right, track picker dropdown + pause/skip/loop/speed controls
    │   ├─ Build menu: bottom-center horizontal bar with building sprites + help text; click-to-place with grid-snapped ghost preview; destroy mode in bar + inspector
    │   ├─ Tutorial: 20-step guided walkthrough (camera → building → NPC interaction → upgrades → policies → patrols → squads); condition-driven auto-advance + manual Next/Skip + 10-min auto-end
    │   ├─ Pause menu (ESC): Resume, Settings (UI scale, scroll speed, background FPS, music/SFX volume, log/debug filters), Exit to Main Menu
    │   └─ Game cleanup: despawn + reset (OnExit Playing)
    │
    ├─ Messages (static queues) ───────────▶ [messages.md]
    │
    ├─ GPU Compute (gpu.rs) ───────────────▶ [gpu-compute.md]
    │   ├─ Bevy render graph integration
    │   └─ WGSL shader (shaders/npc_compute.wgsl)
    │
    ├─ NPC Rendering (npc_render.rs) ─────────▶ [rendering.md]
    │   ├─ RenderCommand + Transparent2d phase, explicit sort-key ordering
    │   ├─ Storage buffer path (buildings + NPCs): 3 shader-def variants via StorageDrawMode
    │   ├─ Instance buffer path (building overlays, projectiles)
    │   └─ WGSL shader (shaders/npc_render.wgsl)
    │
    ├─ Sprite Rendering (render.rs) ───────────▶ [rendering.md]
    │   ├─ 2D camera, texture atlases
    │   └─ Character + world + heal + sleep sprite sheets
    │
    ├─ Bevy Systems (gated on AppState::Playing | Running)
    │   ├─ Spawn systems ──────────────────▶ [spawn.md]
    │   ├─ Combat pipeline ────────────────▶ [combat.md]
    │   ├─ Behavior systems ───────────────▶ [behavior.md]
    │   ├─ Economy systems ────────────────▶ [economy.md]
    │   ├─ AI player system ─────────────▶ [ai-player.md]
    │   └─ Audio (systems/audio.rs) ───▶ music jukebox (22 tracks, random no-repeat, speed control), SFX scaffold
    │
    └─ Test Framework (tests/)
        ├─ bevy_egui menu (EguiPrimaryContextPass)
        ├─ AppState: TestMenu ↔ Running
        └─ Per-test setup + tick systems

Frame execution order ─────────────────────▶ [frame-loop.md]
```

## Documents

| Doc | What it covers | Rating |
|-----|---------------|--------|
| [frame-loop.md](frame-loop.md) | Per-frame execution order, main/render world timing | 8/10 |
| [gpu-compute.md](gpu-compute.md) | Compute shaders, spatial grid, separation physics, combat targeting, GPU→ECS readback | 8/10 |
| [rendering.md](rendering.md) | TilemapChunk terrain, GPU instanced buildings/NPCs/equipment, 6-atlas pipeline, explicit sort-key pass ordering, RenderCommand pattern, camera controls, health bars | 8/10 |
| [combat.md](combat.md) | Attack → damage → death → XP grant → cleanup, slot recycling | 8/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | Decision system, utility AI, state machine, energy, patrol, flee/leash | 7/10 |
| [ai-player.md](ai-player.md) | AI decision loop, hunger system, building scoring, slot placement, squad commander, migration | 8/10 |
| [economy.md](economy.md) | Farm growth, food theft, starvation, camp foraging, unified building spawners, dynamic raider camp migration (spawn→wander→settle) | 7/10 |
| [messages.md](messages.md) | Static queues, GpuUpdateMsg messages, GPU_READ_STATE | 7/10 |
| [resources.md](resources.md) | Bevy resources, game state ownership, UI caches, world data | 7/10 |
| [projectiles.md](projectiles.md) | GPU projectile compute, hit detection, instanced rendering, slot allocation | 7/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, migration plan | - |

Ratings reflect system quality, not doc accuracy.

## File Map

```
rust/
  Cargo.toml            # Pure Bevy 0.18 + bevy_egui + bytemuck + rand + noise + bevy_embedded_assets
  src/main.rs           # Bevy App entry point, EmbeddedAssetPlugin (release), maximize window on startup, crash handler (panic hook → clipboard + crash.log + native dialog)
  src/lib.rs            # build_app(), AppState enum, system scheduling, helpers
  src/gpu.rs            # GPU compute via Bevy render graph, RenderFrameConfig (single ExtractResource bundling NpcGpuData + ProjGpuData + NpcSpriteTexture + ReadbackHandles)
  src/npc_render.rs     # GPU NPC rendering (storage buffers) + misc/projectile rendering (instance buffers)
  src/render.rs         # 2D camera, texture atlases (SpriteAssets with registry-driven external_textures Vec), TilemapChunk spawning (terrain only), TerrainChunk sync, building atlas creation, click_to_select_system (ClickSelectParams SystemParam, double-click fountain → Factions tab, filters building GPU slots, dead NPC guard via NpcEntityMap)
  src/messages.rs       # Static queues (GpuUpdate), Message types
  src/components.rs     # ECS components (NpcIndex, Job, Energy, Health, LastHitBy, BaseAttackType, CachedStats, Activity/CombatState enums, SquadId, CarriedGold, MiningProgress, Archer/Farmer/Miner/Crossbow markers, SquadUnit (military marker for squad queries), Migrating, Job methods: color/label/sprite/is_patrol_unit/is_military from NPC_REGISTRY)
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates, TowerStats struct + FOUNTAIN_TOWER const, squad limits, mining, MAX_MINE_OCCUPANCY, 8x8 base build area, WAYPOINT_COVER_RADIUS, MAX_NPC_COUNT=100K, ATLAS_* IDs), NPC_REGISTRY (single source of truth: NpcDef with job/label/label_plural/sprite/color/stats/attack_override/classification flags/spawn component flags), npc_def(job), BUILDING_REGISTRY (single source of truth: 12 BuildingDef entries with kind/display(DisplayCategory)/tile/hp/cost/label/spawner/placement/tower_stats + fn pointers: build/len/pos_town/count_for_town/hps/hps_mut/town_idx), building_def(kind), tileset_index(kind), building_cost(kind), TileSpec (External(&'static str) carries asset path), SpawnerDef, SpawnBehavior, PlacementMode, OnPlace, DisplayCategory (Hidden/Economy/Military)
  src/resources.rs      # Bevy resources (SlotAllocator, GameTime, FactionStats, TowerState/TowerKindState, SquadState/SquadOwner (Squad has wave_active/wave_start_count/wave_min_start/wave_retreat_below_pct), GoldStorage, MineStates, MinerProgressRender, BuildingHpState, MiningPolicy, BuildingSlotMap, BuildMenuContext (selected_build: Option<BuildingKind>, destroy_mode: bool), HelpCatalog, etc.)
  src/save.rs            # Save/load system (F5/F9 quicksave/load, autosave with 3 rotating slots, save file picker via list_saves/read_save_from, SaveData serialization (Building enum serialized directly via serde derives), SystemParam bundles, SAVE_VERSION with version-gated migration in apply_save)
  src/settings.rs       # UserSettings persistence (serde JSON save/load, version migration v4, auto_upgrades, autosave_hours, music/sfx volume, music speed, tutorial_completed, log_faction_filter)
  src/world.rs          # World data structs (GoldMine, MinerHome{assigned_mine}, FarmerHome, ArcherHome, CrossbowHome, FighterHome, Waypoint), world grid, procedural generation (mine placement), tileset builder (build_tileset for terrain array, build_building_atlas for building strip), town grid, building placement/removal, BuildingSpatialGrid (CPU spatial grid for O(1) building lookups, faction-aware), building GPU slot allocation (allocate_building_slot with tileset_idx + tower flag, free_building_slot, allocate_all_building_slots), BuildingKind enum (Fountain/Bed/Waypoint/Farm/Camp/FarmerHome/ArcherHome/Tent/GoldMine/MinerHome/CrossbowHome/FighterHome), Building enum (serde Serialize/Deserialize, instances with per-building data), building_tiles() (tile specs from registry), shared helpers: is_alive(), empty_slots(), build_and_pay() (includes dirty flag), place_waypoint_at_world_pos(), register_spawner(), resolve_spawner_npc() (uses SpawnBehavior from registry), destroy_building(), find_nearest_enemy_building(), Building::kind()/spawner_kind()/is_tower()/tileset_index() (all delegate to registry), WorldData::building_pos_town/building_len/building_counts (all delegate to BUILDING_REGISTRY fn pointers), miner_home_at()/gold_mine_at()
  src/ui/
    mod.rs              # register_ui(), game startup (+ policy load), cleanup, pause menu (+ debug settings + UI scale + audio volume), escape/time controls, keyboard toggles (Q=squads, H=help), build ghost preview, slot indicators, process_destroy_system, apply_ui_scale
    main_menu.rs        # Main menu with difficulty presets (Easy/Normal/Hard), world config sliders (farms + gold mines top-level, farmer/archer homes under AI Towns, tents under Raider Camps), Play / Load Game / Debug Tests, restart tutorial button
    game_hud.rs         # Top bar (food + gold + FPS), jukebox overlay (track picker + pause/skip/loop/speed), floating inspector with combat stats/equipment/status + clickable faction link (bottom-left) + combat log with faction filter dropdown + clickable ">>" camera-pan on location entries (bottom-right), mine assignment UI (click-to-assign), target overlay, squad overlay
    left_panel.rs       # Tabbed floating Window: Roster (R) / Upgrades (U) / Policies (P) / Patrols (T) / Squads (Q) / Factions (I) / Help (H) — policy persistence on tab leave, Factions tab shows squad commander details per faction, registry-driven intel panel (DisplayCategory Economy/Military columns, label_plural from NPC_REGISTRY)
    build_menu.rs       # Bottom-center build bar: data-driven from BUILDING_REGISTRY (player_buildable/camp_buildable filter), cached atlas extraction, click-to-place, destroy mode, cursor hint
    tutorial.rs         # 20-step guided tutorial: condition-driven hints (action triggers + info-only Next steps), skip per-step or all, 10-minute auto-end timeout, persisted completion in UserSettings
  src/tests/
    mod.rs              # Test framework (TestState, menu UI, HUD, cleanup)
    vertical_slice.rs   # Full core loop test (8 phases, spawn→combat→death→respawn)
    spawning.rs         # Spawn + slot reuse test (4 phases)
    energy.rs           # Energy drain test (3 phases)
    movement.rs         # Movement + arrival test (3 phases)
    archer_patrol.rs    # Archer patrol cycle (5 phases)
    farmer_cycle.rs     # Farmer work cycle (5 phases)
    raider_cycle.rs     # Raider raid cycle (5 phases)
    combat.rs           # Combat pipeline test (6 phases)
    projectiles.rs      # Projectile pipeline test (4 phases)
    healing.rs          # Healing aura test (3 phases)
    economy.rs          # Economy test (5 phases)
    world_gen.rs        # World generation test (6 phases)
    sleep_visual.rs     # Sleep icon visual test (3 phases)
    farm_visual.rs      # Farm ready marker visual test (3 phases)
    heal_visual.rs      # Heal icon visual test (3 phases)
    npc_visuals.rs    # NPC visual showcase — all types × all layers in labeled grid (1 phase)
    terrain_visual.rs # Terrain + building visual showcase — all biomes and building types (1 phase)
    friendly_fire_buildings.rs # Friendly fire regression — shooter through farm wall (4 phases)
  src/systems/
    spawn.rs            # Spawn system (MessageReader<SpawnNpcMsg>)
    stats.rs            # CombatConfig (jobs + attacks + crossbow_attack base), TownUpgrades, UpgradeQueue, UPGRADE_REGISTRY (25 upgrades incl. 5 crossbow, prereqs + multi-resource cost), UPGRADE_RENDER_ORDER (tree UI with Crossbow branch), resolve_combat_stats() (crossbow job overrides attack base), xp_grant_system, process_upgrades_system, auto_upgrade_system, upgrade helpers (upgrade_unlocked/upgrade_available/deduct_upgrade_cost/format_upgrade_cost/missing_prereqs/upgrade_effect_summary/branch_total/expansion_cost)
    drain.rs            # Queue drain systems, reset, collect_gpu_updates
    movement.rs         # GPU position readback, arrival detection
    combat.rs           # Attack cooldown, targeting, building attack fallback, building_tower_system (fountain towers via shared fire_towers helper, GPU combat targeting via npc_flags bit 1), building_damage_system (building_pos_town dispatch, sets buildings_need_healing dirty flag, GPU HP sync via BuildingSlotMap)
    health.rs           # Damage, death, cleanup, healing
    behavior.rs         # Unified decision system, arrivals, squad sync (all SquadUnit NPCs)
    economy.rs          # Game time, farm growth, mine regen, mining_policy_system (auto-discovery + miner distribution), respawning, building spawners, squad_cleanup_system (SquadUnit query, wave-aware), migration spawn/attach/settle
    ai_player.rs        # AI decision system with personalities (Aggressive/Balanced/Economic), weighted random scoring, AiBuildRes SystemParam bundle, TownContext per-tick bundle (center/food/slots/mines via TownContext::build()), AiTownSnapshot caching with smart slot scoring (NeighborCounts incl. crossbow_homes, balanced_farm_ray_score, balanced_house_side_score), MineAnalysis single-pass (analyze_mines with all_positions), Option<MineAnalysis> type-safe builder-only enforcement, try_build_scored/try_build_inner/try_build_miner_home unified build helpers, territory_building_sets! macro (single building-type list incl. crossbow_homes), wilderness waypoint placement near uncovered mines, sync_patrol_perimeter_system (dirty-flag-gated waypoint pruning with full destroy_building teardown), ai_squad_commander_system (wave-based attack cycle for Builder+Raider AIs, SquadUnit query, SquadRole Attack/Reserve/Idle, defense_share_pct + attack_split_weight per personality, wave gather→threshold→dispatch→retreat model, pick_raider_farm_target for raider camps, self-healing ownership scan), BuildCrossbowHome action (scored after 2+ archer homes)
    audio.rs            # Music jukebox (22 tracks, random no-repeat, volume + speed control), SFX scaffold
    energy.rs           # Energy drain/recovery
    sync.rs             # GPU state sync

  assets/                 # Standard Bevy asset dir (embedded in release builds via bevy_embedded_assets)
    sprites/
      roguelikeChar_transparent.png   # Character sprites (54x12 grid, 16px + 1px margin)
      roguelikeSheet_transparent.png  # World sprites (57x31 grid, 16px + 1px margin)
      heal.png                        # Heal halo sprite (single 16x16, atlas_id=2.0)
      sleep.png                       # Sleep icon sprite (single 16x16, atlas_id=3.0)
      arrow.png                       # Arrow projectile sprite (single texture, white)
      house.png                       # Farmer Home building sprite (32x32, External tileset)
      barracks.png                    # Archer Home building sprite (32x32, External tileset)
      waypoint.png                    # Waypoint building sprite (32x32, External tileset)
      miner_house.png                 # Miner Home building sprite (32x32, External tileset)
      fighter_home.png                # Fighter Home building sprite (32x32, External tileset)
    sounds/
      music/not-jam-music/  # 22 .ogg tracks (Not Jam Music Pack, CC0)
    shaders/
      npc_compute.wgsl      # WGSL compute shader (movement + spatial grid + combat targeting)
      npc_render.wgsl       # WGSL render shader (dual vertex path: storage buffer + instanced)
      projectile_compute.wgsl # WGSL compute shader (projectile movement + collision)
```
