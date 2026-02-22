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
| `economy` | 5 | Farm growing → ready → harvest → raider forage → tent spawner respawn |
| `world-gen` | 6 | Grid dimensions, town placement, buildings, terrain, raider towns |
| `sleep-visual` | 3 | Resting NPC gets sleep icon (atlas_id=3.0) on status layer, cleared on wake |
| `farm-visual` | 3 | Ready farm spawns FarmReadyMarker, removed on harvest |
| `heal-visual` | 3 | Healing NPC gets halo (atlas_id=2.0) on healing layer, cleared when healed |
| `npc-visuals` | 1 | Visual showcase: all NPC types in labeled grid with individual layer breakdown |
| `terrain-visual` | 1 | Visual showcase: all terrain biomes and building types in labeled grid |
| `friendly-fire-buildings` | 4 | Ranged shooter fires through friendly farm wall without damaging same-faction buildings |
| `endless-mode` | 16 | Builder + raider fountain destroyed → spawn queued → boat migration → settle (both AI kinds) |
| `ai-building` | 2 | AI town building observation: pick personality, watch it build with 100K food+gold |

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
    │   ├─ Main menu: sectioned layout (World / Difficulty / Options / Debug), difficulty presets (Easy/Normal/Hard) control endless mode + NPC counts, tooltips on all sliders, Play / Load Game / Debug Tests / Restart Tutorial
    │   ├─ Game startup: world gen + NPC spawn (OnEnter Playing)
    │   ├─ Top bar: panel toggles left, town name + time center, stats (food + gold) + FPS right
    │   ├─ Floating windows: NPC/building/DC-group inspector with combat stats + equipment + status + keep-fighting toggle (bottom-left) + combat log with faction filter + type filters (bottom-right)
    │   ├─ Left panel: floating Window with Roster (R) / Upgrades (U) / Policies (P) / Patrols (T) / Squads (Q) / Factions (I) / Help (H)
    │   ├─ Jukebox overlay: top-right, track picker dropdown + pause/skip/loop/speed controls
    │   ├─ Build menu: bottom-center horizontal bar with building sprites + help text + hover tooltips; click-to-place with grid-snapped ghost preview; Road drag-line placement; destroy mode in bar + inspector
    │   ├─ Tutorial: 20-step guided walkthrough (camera → building → NPC interaction → upgrades → policies → patrols → squads); condition-driven auto-advance + manual Next/Skip + 10-min auto-end
    │   ├─ Pause menu (ESC): Resume, Settings (UI scale, scroll speed, background FPS, music/SFX volume, log/debug filters, AI decision logging), Exit to Main Menu — available in both Playing and Running (test scenes)
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
    │   └─ Character + world + extras (heal/sleep/arrow/boat) sprite sheets
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
| [rendering.md](rendering.md) | TilemapChunk terrain, GPU instanced buildings/NPCs/equipment, 4-atlas pipeline (char/world/extras/building), explicit sort-key pass ordering, RenderCommand pattern, camera controls, health bars | 8/10 |
| [combat.md](combat.md) | Attack → damage → death → XP grant → cleanup, slot recycling | 8/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation, DRY save-load via materialize_npc | 8/10 |
| [behavior.md](behavior.md) | Decision system, utility AI, state machine, energy, patrol, flee/leash | 7/10 |
| [ai-player.md](ai-player.md) | AI decision loop, hunger system, building scoring, slot placement, squad commander, migration | 8/10 |
| [economy.md](economy.md) | Farm growth, food theft, starvation, raider foraging, unified building spawners, dynamic raider town migration (spawn→boat→disembark→walk→settle) | 7/10 |
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
  src/gpu.rs            # GPU compute via Bevy render graph, RenderFrameConfig (single ExtractResource bundling NpcGpuData + ProjGpuData + NpcSpriteTexture (extras_handle for consolidated atlas) + ReadbackHandles + tile_flags), populate_tile_flags system (terrain+building bitfield from WorldGrid)
  src/npc_render.rs     # GPU NPC rendering (storage buffers) + misc/projectile rendering (instance buffers)
  src/render.rs         # 2D camera, texture atlases (SpriteAssets with registry-driven external_textures Vec), TilemapChunk spawning (terrain only), TerrainChunk sync, building atlas + extras atlas creation (build_extras_atlas), click_to_select_system (ClickSelectParams SystemParam, double-click fountain → Factions tab, filters building GPU slots, dead NPC guard via NpcEntityMap, right-click: placing_target mode sets squad.target, default mode commands DirectControl NPCs only with ManualTarget enum + direct GPU SetTarget writes + wakes Resting/GoingToRest NPCs to Idle), box_select_system (drag-rectangle multi-select of player military NPCs, inserts DirectControl + SquadId, clears SelectedNpc + SelectedBuilding so inspector shows DC group view)
  src/messages.rs       # Static queues (GpuUpdate), Message types
  src/components.rs     # ECS components (NpcIndex, Job, Energy, Health, LastHitBy, BaseAttackType, CachedStats, Activity/CombatState enums, SquadId, CarriedGold, MiningProgress, ManualTarget enum (Npc/Building/Position — per-NPC DirectControl target), DirectControl marker (box-selected, skips decision_system), Archer/Farmer/Miner/Crossbow markers, SquadUnit (military marker for squad queries), Migrating, Job methods: color/label/sprite/is_patrol_unit/is_military from NPC_REGISTRY)
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates, TowerStats struct + FOUNTAIN_TOWER const, squad limits, mining, MAX_MINE_OCCUPANCY, 8x8 base build area, WAYPOINT_COVER_RADIUS, MAX_NPC_COUNT=100K, ATLAS_* IDs), NPC_REGISTRY (single source of truth: NpcDef with job/label/label_plural/sprite/color/ui_color/stats/attack_override/classification flags/spawn component flags/home_building/is_raider_unit/default_count/upgrade_category/upgrade_stats), npc_def(job), UpgradeStatKind enum, UpgradeStatDef struct (kind/pct/cost/label/tooltip/display/prereqs/flags), EffectDisplay enum, const upgrade arrays (MILITARY_RANGED_UPGRADES/MILITARY_MELEE_UPGRADES/FARMER_UPGRADES/MINER_UPGRADES/TOWN_UPGRADES), BUILDING_REGISTRY (single source of truth: 13 BuildingDef entries with kind/display(DisplayCategory)/tile/hp/cost/label/tooltip/spawner/placement/tower_stats + fn pointers), BuildingDef::loot_drop() method (derives cost/2 as food), building_def(kind), tileset_index(kind), building_cost(kind), TileSpec, SpawnerDef, SpawnBehavior, PlacementMode, OnPlace, DisplayCategory (Hidden/Economy/Military), TILE_* bitfield constants (terrain: GRASS/FOREST/WATER/ROCK/DIRT bits 0-4, building: ROAD bit 5), ROAD_SPEED_MULT, Road is player_buildable + raider_buildable
  src/resources.rs      # Bevy resources (SlotAllocator, GameTime, FactionStats, TowerState/TowerKindState, SquadState/SquadOwner (Squad has wave_active/wave_start_count/wave_min_start/wave_retreat_below_pct/hold_fire), SquadState has drag_start/box_selecting for box-select, dc_no_return for DC keep-fighting toggle), GoldStorage, MineStates, MinerProgressRender, BuildingHpState (towns: Vec<f32> + hps: BTreeMap<BuildingKind, Vec<f32>> with custom serde for save compat), MiningPolicy, BuildingSlotMap, BuildMenuContext (selected_build: Option<BuildingKind>, destroy_mode: bool), HelpCatalog, DifficultyPreset (farms/ai_towns/raider_towns/gold_mines/npc_counts/endless_mode/endless_strength), GameConfig (npc_counts: BTreeMap<Job, i32>), etc.)
  src/save.rs            # Save/load system (F5/F9 quicksave/load, autosave with 3 rotating slots, save file picker via list_saves/read_save_from, SaveData serialization (GridBuilding tuples for grid cells with LegacyBuilding deserialization compat, building vecs via #[serde(flatten)] HashMap + BUILDING_REGISTRY save_key/save_vec/load_vec fn pointers, BuildingHpState serialized directly), SystemParam bundles, SAVE_VERSION with version-gated migration in apply_save, spawn_npcs_from_save delegates to materialize_npc() via NpcSpawnOverrides)
  src/settings.rs       # UserSettings persistence (serde JSON save/load, version migration v7, npc_counts BTreeMap<String,usize> with legacy farmers/archers/raiders migration, auto_upgrades, upgrade_expanded (Vec<String> for collapsible branch persistence), autosave_hours, music/sfx volume, music speed, tutorial_completed, log_faction_filter, debug_ai_decisions)
  src/world.rs          # World data structs (PlacedBuilding unified struct with position/town_idx + optional patrol_order/assigned_mine/manual_mine, type aliases Farm=Bed=Waypoint=UnitHome=MinerHome=GoldMine=PlacedBuilding — all derive Serialize/Deserialize with vec2_as_array serde module for Vec2↔[f32;2]), WorldData (buildings: BTreeMap<BuildingKind, Vec<PlacedBuilding>> unified storage, legacy accessors farms()/beds()/waypoints()/miner_homes()/gold_mines() + _mut() variants), world grid (WorldCell.building: Option<GridBuilding> where GridBuilding = (BuildingKind, u32)), procedural generation (mine placement), tileset builder (build_tileset for terrain array, build_building_atlas for building strip, build_extras_atlas for extras horizontal grid), town grid, building placement/removal (place/tombstone/find_index delegated to registry fn pointers), BuildingSpatialGrid (CPU spatial grid for O(1) building lookups, faction-aware, rebuilt via single BUILDING_REGISTRY loop), building GPU slot allocation (allocate_building_slot with tileset_idx + tower flag, free_building_slot, allocate_all_building_slots via single registry loop), BuildingKind enum (Fountain/Bed/Waypoint/Farm/FarmerHome/ArcherHome/Tent/GoldMine/MinerHome/CrossbowHome/FighterHome/Road), building_tiles() (tile specs from registry), shared helpers: is_alive(), empty_slots(), place_building() (unified placement: validate+pay+grid+WorldData+spawner+HP+GPU slot+dirty flags, takes world_pos, rejects water/foreign territory via in_foreign_build_area), register_spawner(kind), resolve_spawner_npc() (uses SpawnBehavior from registry), destroy_building(), find_nearest_enemy_building(), WorldData::building_pos_town/building_len/building_counts (all delegate to BUILDING_REGISTRY fn pointers), miner_home_at()/gold_mine_at()
  src/ui/
    mod.rs              # register_ui(), game startup (+ policy load), cleanup, pause menu (+ debug settings + UI scale + audio volume + AI decision logging), escape/time controls (Playing + Running states), keyboard toggles (Q=squads, H=help), build ghost preview (Road drag-line with ghost trail), slot indicators, process_destroy_system, apply_ui_scale, test scene systems (bottom panel + selection overlay + pause menu in Running state)
    main_menu.rs        # Main menu with sectioned layout (World / Difficulty / Options / Debug Options), difficulty presets (Easy/Normal/Hard) drive NPC counts + endless mode + strength, "Per Town (player & AI)" group for farms/mines/NPC homes, AI Builder Towns + AI Raider Towns count sliders, tooltips on all controls, Play / Load Game / Debug Tests, restart tutorial button
    game_hud.rs         # Top bar (food + gold + FPS), jukebox overlay (track picker + pause/skip/loop/speed), floating inspector with combat stats/equipment/status/carried loot + clickable faction link (bottom-left) + DC group inspector (unit count/HP/job breakdown/keep-fighting toggle when box-selected) + combat log with faction filter dropdown + clickable ">>" camera-pan on location entries (bottom-right), registry-driven spawner inspector (def.spawner + npc_def for labels, linked NPC state/squad/patrol/position via NpcStateQuery), mine assignment UI (click-to-assign), target overlay, squad overlay (numbered target circles hidden during box-select, DirectControl crosshair from ManualTarget::Npc/Building queries), selection overlay (cyan brackets for selected NPC, green brackets for DirectControl NPCs), expanded Copy Debug Info (NPC: XP/traits/stats/equipment/squad/DirectControl/gold/miner details; Building: faction/per-type details)
    left_panel.rs       # Tabbed floating Window: Roster (R) / Upgrades (U) / Policies (P) / Patrols (T) / Squads (Q) / Factions (I) / Help (H) — Upgrades tab: Economy/Military section headers with collapsible per-NPC branches (Farmer/Miner/Town under Economy, Archer/Fighter/Crossbow under Military), driven by UPGRADES.branches with section field, CollapsingState with expand/collapse persisted to UserSettings.upgrade_expanded (default collapsed); policy persistence on tab leave, Factions tab shows squad commander details per faction with per-unit stat breakdowns (Archer/Fighter/Crossbow separate), "Copy Debug" button (builds comprehensive debug string via build_faction_debug_string → clipboard), registry-driven intel panel (DisplayCategory Economy/Military columns, label_plural from NPC_REGISTRY), registry-driven roster (job filter buttons + row colors from NPC_REGISTRY, military-first ordering), registry-driven squad UI (per-job recruit rows from NPC_REGISTRY is_military, job-colored member list, hold-fire checkbox, attack target display)
    build_menu.rs       # Bottom-center build bar: data-driven from BUILDING_REGISTRY (player_buildable/raider_buildable filter), cached atlas extraction, click-to-place, destroy mode, cursor hint, hover tooltips from BuildingDef.tooltip
    tutorial.rs         # 20-step guided tutorial: condition-driven hints (action triggers + info-only Next steps), skip per-step or all, 10-minute auto-end timeout, persisted completion in UserSettings
  src/tests/
    mod.rs              # Test framework (TestState, menu UI, HUD, cleanup, time controls: Space=pause, +/-=speed)
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
    endless_mode.rs   # Endless mode test — builder + raider fountain destroy → migration → settle (16 phases)
    ai_building.rs    # AI building observation test — pick personality, watch AI build with 100K food+gold (2 phases)
  src/systems/
    spawn.rs            # Spawn system (MessageReader<SpawnNpcMsg>), materialize_npc() shared helper (single source of truth for fresh spawn + save-load), NpcSpawnOverrides
    stats.rs            # CombatConfig (jobs + attacks + crossbow_attack base), TownUpgrades (Vec<Vec<u8>>), UpgradeQueue, dynamic UpgradeRegistry via LazyLock (UpgradeNode/UpgradeBranch with section grouping, built from NPC_REGISTRY upgrade_stats + TOWN_UPGRADES at init), UPGRADES static, resolve_combat_stats() (uses npc_def.upgrade_category + UPGRADES.stat_mult), xp_grant_system, process_upgrades_system, auto_upgrade_system, upgrade helpers (upgrade_unlocked/upgrade_available/deduct_upgrade_cost/format_upgrade_cost/missing_prereqs/upgrade_effect_summary/branch_total/expansion_cost)
    drain.rs            # Queue drain systems, reset, collect_gpu_updates
    movement.rs         # GPU position readback, arrival detection
    combat.rs           # Attack cooldown, targeting, building attack fallback, building_tower_system (fountain towers via shared fire_towers helper, GPU combat targeting via npc_flags bit 1), building_damage_system (building_pos_town dispatch, sets buildings_need_healing dirty flag, GPU HP sync via BuildingSlotMap, skips indestructible: GoldMine + Road)
    health.rs           # Damage, death, cleanup, healing
    behavior.rs         # Unified decision system, arrivals, squad sync (all SquadUnit NPCs)
    economy.rs          # Game time, farm growth, mine regen, mining_policy_system (auto-discovery + miner distribution), respawning, building spawners, squad_cleanup_system (SquadUnit query, wave-aware), migration spawn/attach/settle (wipeout → respawn after 4h delay)
    ai_player.rs        # AI decision system with personalities (Aggressive/Balanced/Economic), weighted random scoring with retry loop (failed actions removed via discriminant, re-pick from remaining), AiBuildRes SystemParam bundle, TownContext per-tick bundle (center/food/slots/mines via TownContext::build()), AiTownSnapshot caching with smart slot scoring (NeighborCounts incl. crossbow_homes, balanced_farm_ray_score, balanced_house_side_score), MineAnalysis single-pass (analyze_mines with all_positions), Option<MineAnalysis> type-safe builder-only enforcement, try_build_scored/try_build_inner/try_build_miner_home unified build helpers, waypoint_ring_slots (personality-driven outer ring: block corners on perimeter adjacent to road intersections, min spacing), sync_patrol_perimeter_system (dirty-flag-gated waypoint pruning against ideal ring with full destroy_building teardown), ai_squad_commander_system (wave-based attack cycle for Builder+Raider AIs, SquadUnit query, SquadRole Attack/Reserve/Idle, defense_share_pct + attack_split_weight per personality, wave gather→threshold→dispatch→retreat model, pick_raider_farm_target for raider towns, self-healing ownership scan), BuildCrossbowHome action (scored after 2+ archer homes), BuildRoads action (personality grid patterns, batch placement, Chebyshev distance ≤ 2 adjacency, grid-snapped positions), Phase 1/Phase 2 split (building + upgrade per tick), economy desire signal (slot fullness floors other desires), road+waypoint-aware slot placement (is_road_slot + waypoint_ring_slots filter), debug_ai_decisions logging (failed actions with scores to last_actions)
    audio.rs            # Music jukebox (22 tracks, random no-repeat, volume + speed control), SFX scaffold
    energy.rs           # Energy drain/recovery
    sync.rs             # GPU state sync

  assets/                 # Standard Bevy asset dir (embedded in release builds via bevy_embedded_assets)
    sprites/
      roguelikeChar_transparent.png   # Character sprites (54x12 grid, 16px + 1px margin)
      roguelikeSheet_transparent.png  # World sprites (57x31 grid, 16px + 1px margin)
      heal.png                        # Heal halo sprite (single 16x16, atlas_id=2.0)
      sleep.png                       # Sleep icon sprite (single 16x16, atlas_id=3.0)
      arrow.png                       # Arrow projectile sprite (single texture, white, extras atlas col 2)
      boat.png                        # Boat migration sprite (32x32, extras atlas col 3)
      house.png                       # Farmer Home building sprite (32x32, External tileset)
      barracks.png                    # Archer Home building sprite (32x32, External tileset)
      waypoint.png                    # Waypoint building sprite (32x32, External tileset)
      miner_house.png                 # Miner Home building sprite (32x32, External tileset)
      fighter_home.png                # Fighter Home building sprite (32x32, External tileset)
    sounds/
      music/not-jam-music/  # 22 .ogg tracks (Not Jam Music Pack, CC0)
    shaders/
      npc_compute.wgsl      # WGSL compute shader (movement + spatial grid + combat targeting + tile_flags road system: speed bonus + collision bypass + attraction)
      npc_render.wgsl       # WGSL render shader (dual vertex path: storage buffer + instanced)
      projectile_compute.wgsl # WGSL compute shader (projectile movement + collision)
```
