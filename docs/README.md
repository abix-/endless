# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
   For hot-path changes (per-frame/per-tick code), also apply [performance-review.md](performance-review.md).
   Hot-path default: Bevy `Query` for filtered scans, `EntityMap` for keyed lookups/indexes.
2. **During coding**: Follow the patterns documented here (job-as-template spawn, chained combat, GPU buffer layout). If you need to deviate, update the doc first.
3. **After coding**: Update the doc if the architecture changed. Add new known issues discovered during implementation.
4. **Code comments** stay educational (explain what code does for someone learning Rust/WGSL). Architecture diagrams, data flow, buffer layouts, and system interaction docs live here, not in code.
5. **README.md** is the game intro (description, gameplay, controls). **docs/roadmap.md** has stages (priority order) and capabilities (feature inventory) — read its maintenance guide before editing.
6. **Write docs in present tense**: explain what the system currently does. Put historical change logs ("was/used to/fixed") in **completed.md** or **CHANGELOG.md**, not architecture docs.

## Test Framework

UI-selectable integration tests run inside the full Bevy app via a bevy_egui menu. From `AppState::MainMenu`, click "Debug Tests" to enter `AppState::TestMenu` which shows the test list; clicking a test transitions to `AppState::Running` where game systems execute normally. "Back to Menu" returns to MainMenu. Tests use **phased assertions** — each phase checks one pipeline layer and fails fast with diagnostic values.

**Architecture** (`rust/src/tests/`):
- `TestState` resource: shared phase tracking, counters, flags, results
- `TestRegistry`: registered test entries (name, description, phase_count, time_scale)
- `TestSetupParams`: SystemParam bundle for test setup (slot alloc, spawn, world data, food, factions, game time, test state)
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
| `farm-visual` | 3 | Ready farm spawns FarmReadyMarker, cleared on harvest |
| `heal-visual` | 3 | Healing NPC gets halo (atlas_id=2.0) on healing layer, cleared when healed |
| `npc-visuals` | 1 | Visual showcase: all NPC types in labeled grid with individual layer breakdown |
| `terrain-visual` | 1 | Visual showcase: all terrain biomes and building types in labeled grid |
| `friendly-fire-buildings` | 4 | Ranged shooter fires through friendly farm wall without damaging same-faction buildings |
| `endless-mode` | 16 | Builder + raider fountain destroyed → spawn queued → boat migration → settle (both AI kinds) |
| `ai-building` | 2 | AI town building observation: pick personality, watch it build with 100K food+gold on a 10000x10000 map |
| `miner-cycle` | 5 | Miner: walk to mine → tend → harvest gold → deliver → rest |
| `archer-tent-reliability` | 5 | Archer vs enemy tent: target lock, projectile activity, sustained tent damage, destruction |

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
    │   ├─ Build menu: bottom-center horizontal bar with building sprites + help text + hover tooltips; click-to-place with grid-snapped ghost preview; Road drag-line placement; Wall placement (town grid, blocks enemy NPCs); destroy mode in bar + inspector
    │   ├─ Tutorial: 20-step guided walkthrough (camera → building → NPC interaction → upgrades → policies → patrols → squads); condition-driven auto-advance + manual Next/Skip + 10-min auto-end
    │   ├─ Pause menu (ESC): Resume, Settings (UI scale, scroll speed, zoom & LOD (speed/min/max/transition), background FPS, music/SFX volume, log/debug filters, AI decision logging), Exit to Main Menu — available in both Playing and Running (test scenes)
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
    │   ├─ Storage buffer path (NPCs only): 2 shader-def variants via StorageDrawMode
    │   ├─ Instance buffer path (building bodies, building overlays, projectiles)
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
| [behavior.md](behavior.md) | Decision system, utility AI, state machine, energy, patrol, flee/leash | 8/10 |
| [ai-player.md](ai-player.md) | AI decision loop, hunger system, building scoring, slot placement, squad commander, migration | 8/10 |
| [economy.md](economy.md) | Farm growth, food theft, starvation, raider foraging, spawner respawn (BuildingInstance fields), dynamic raider town migration (spawn→boat→disembark→walk→settle) | 8/10 |
| [messages.md](messages.md) | Message flow, GpuUpdateMsg, GAME_CONFIG_STAGING, readback resources | 7/10 |
| [resources.md](resources.md) | Bevy resources, game state ownership, UI caches, world data | 7/10 |
| [projectiles.md](projectiles.md) | GPU projectile compute, hit detection, instanced rendering, slot allocation | 7/10 |
| [authority.md](authority.md) | GPU readback vs ECS authority contract, source-of-truth rules, validation patterns | - |
| [performance-review.md](performance-review.md) | Hot-path perf anti-pattern checklist, review procedure, benchmark guardrails | - |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, migration plan | - |

Ratings reflect system quality, not doc accuracy.

## File Map

```
rust/
  Cargo.toml            # Pure Bevy 0.18 + bevy_egui + bytemuck + rand + noise + bevy_embedded_assets
  src/main.rs           # Bevy App entry point, EmbeddedAssetPlugin (release), maximize window on startup, crash handler (panic hook → clipboard + crash.log + native dialog)
  src/lib.rs            # build_app(), AppState enum, system scheduling, helpers
  src/gpu.rs            # GPU compute via Bevy render graph, RenderFrameConfig (single ExtractResource bundling EntityGpuData + ProjGpuData + NpcSpriteTexture (extras_handle for consolidated atlas) + ReadbackHandles + tile_flags), EntityGpuBuffers (unified entity buffers sized to MAX_ENTITIES=200K), EntityGpuState (unified CPU-side state for all entities: positions/factions/healths/entity_flags/sprites/flash/targets/speeds/arrivals + per-buffer dirty flags + per-index dirty tracking (position/arrival/target_dirty_indices, hidden_indices for stale visual cleanup, target_buffer_size for full-upload fallback) + visual_dirty_indices/visual_full_rebuild for event-driven visual upload), populate_gpu_state routes all GpuUpdate variants to EntityGpuState (Hide clears sprite_indices+flash, pushes hidden_indices+visual_dirty; SetSpriteFrame/SetDamageFlash/MarkVisualDirty push visual_dirty; flash decay marks dirty), build_visual_upload: event-driven dirty-slot-only update of persistent NpcVisualUpload (full rebuild on startup/load, query-first ECS iteration for full path, EntityMap lookup for dirty path, stale slots cleared to sentinels), populate_tile_flags system (terrain bitfield from WorldGrid + building flags from EntityMap iteration, encodes wall faction in bits 8-11)
  src/npc_render.rs     # GPU NPC rendering (storage buffers) + building body/overlay/projectile rendering (instance buffers), BuildingBodyInstances from EntityGpuState via EntityMap.iter_instances(), extract_npc_data uploads unified EntityGpuState to EntityGpuBuffers (no offset arithmetic)
  src/render.rs         # 2D camera, texture atlases (SpriteAssets with registry-driven external_textures Vec), TilemapChunk spawning (terrain only), TerrainChunk sync, building atlas + extras atlas creation (build_extras_atlas), click_to_select_system (ClickSelectParams SystemParam, double-click fountain → Factions tab, NPC selection via GPU readback positions, building selection via authoritative EntityMap positions (not GPU readback — deterministic placement), dead NPC guard via EntityMap.entities, right-click: placing_target mode sets squad.target, default mode commands DirectControl NPCs only with manual_target + MovementIntents.submit(DirectControl) + wakes Resting/GoingToRest NPCs to Idle), box_select_system (drag-rectangle multi-select via query-first `(&EntitySlot, &Job, &Faction, &Position)` with `Without<Building>, Without<Dead>`, sets NpcFlags.direct_control + inserts SquadId via Commands, HashSet for O(1) membership checks, clears SelectedNpc + SelectedBuilding so inspector shows DC group view)
  src/messages.rs       # Static queues (GpuUpdate — unified variants for NPCs+buildings incl. MarkVisualDirty, no Bld* variants), Message types (DamageMsg unified NPC+building), DirtyWriters SystemParam (dirty signal messages), CombatLogMsg
  src/components.rs     # ECS components for all entities: buildings (EntitySlot, Position, Health, Faction, TownId, Building, FarmReadyMarker, Dead, LastHitBy) + NPC components (Job, Activity (with visual_key() for dirty tracking), CombatState, Energy, Speed, Home, Personality, CachedStats, BaseAttackType, AttackTimer, ManualTarget, PatrolRoute, NpcWorkState (always-present: occupied_slot + work_target, replaces optional AssignedFarm/WorkPosition), CarriedGold, EquippedWeapon/Helmet/Armor, FleeThreshold, LeashRange, WoundedThreshold, NpcFlags, SquadId, Stealer, HasEnergy — all derive Component), TraitKind/TraitInstance for personality system
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates, TowerStats struct + FOUNTAIN_TOWER const, squad limits, mining, MAX_MINE_OCCUPANCY, 8x8 base build area, WAYPOINT_COVER_RADIUS, MAX_NPC_COUNT=100K, MAX_BUILDINGS=100K, MAX_ENTITIES=200K, ENTITY_FLAG_COMBAT/ENTITY_FLAG_BUILDING/ENTITY_FLAG_UNTARGETABLE, ATLAS_* IDs), NPC_REGISTRY (single source of truth: NpcDef with job/label/label_plural/sprite/color/ui_color/stats/attack_override/classification flags/spawn component flags/home_building/is_raider_unit/default_count/upgrade_category/upgrade_stats/loot_drop slice), npc_def(job), UpgradeStatKind enum, UpgradeStatDef struct (kind/pct/cost/label/tooltip/display/prereqs/flags), EffectDisplay enum, const upgrade arrays (MILITARY_RANGED_UPGRADES/MILITARY_MELEE_UPGRADES/FARMER_UPGRADES/MINER_UPGRADES/TOWN_UPGRADES), BUILDING_REGISTRY (14 BuildingDef entries with kind/display(DisplayCategory)/tile/hp/cost/label/tooltip/spawner/placement/tower_stats/save_key/is_unit_home — pure static definitions, no fn pointers), BuildingDef::loot_drop() method (derives cost/2 as food), building_def(kind), tileset_index(kind), building_cost(kind), TileSpec, SpawnerDef, SpawnBehavior, PlacementMode, OnPlace, DisplayCategory (Hidden/Economy/Military), TILE_* bitfield constants (terrain: GRASS/FOREST/WATER/ROCK/DIRT bits 0-4, building: ROAD bit 5, WALL bit 6 + faction bits 8-11), ROAD_SPEED_MULT, WALL_TIER_HP/WALL_TIER_NAMES/WALL_UPGRADE_COSTS (3-tier wall progression), Road is player_buildable + raider_buildable
  src/resources.rs      # Bevy resources (SlotPool shared inner type, EntitySlots wraps SlotPool for all entities max=MAX_ENTITIES=200K (unified NPC+building namespace), EntityMap: unified entities HashMap<usize,Entity> + npcs: HashMap<usize,NpcEntry> (6-field index: slot, entity, job, faction, town_idx, dead — all gameplay state lives in ECS components; register_npc/unregister_npc for index management) + npc_by_town secondary index + building instance data/spatial grid/indexes (sole source of truth for all slot→entity lookups AND building instances; BuildingInstance has npc_slot/respawn_timer for spawner state + occupants: i16 for slot-indexed occupancy; grid-coord accessors: has_building_at(gc,gr)/get_at_grid(gc,gr); occupancy methods: claim/release/occupant_count/is_occupied/slot_at_position; kind-filtered spatial: spatial_kind_town + spatial_kind_cell per-cell buckets with SpatialBucketRef back-index for O(1) swap-remove, for_each_nearby_kind_town/for_each_nearby_kind/for_each_ring_kind_town/for_each_ring_kind cell-ring queries, find_nearest_worksite (min-order tuple scoring + WorksiteFallback), try_claim_worksite (authoritative validate+claim)), GameTime, FactionStats, TowerState/TowerKindState, SquadState/SquadOwner (Squad has wave_active/wave_start_count/wave_min_start/wave_retreat_below_pct/hold_fire), SquadState has drag_start/box_selecting for box-select, dc_no_return for DC keep-fighting toggle), GoldStorage, MineStates, MinerProgressRender, MiningPolicy (mine_enabled keyed by GPU slot), SelectedBuilding (kind + slot), BuildMenuContext (selected_build: Option<BuildingKind>, destroy_mode: bool), BuildingHealState (persistent needs_healing flag for building healing), HelpCatalog, DifficultyPreset, GameConfig, MovementIntents (HashMap<Entity, MovementIntent> with MovementPriority arbitration — sole source of NPC movement targeting), etc.)
  src/save.rs            # Save/load system (F5/F9 quicksave/load, autosave with 3 rotating slots, save file picker via list_saves/read_save_from, SaveData serialization (terrain-only grid cells, building instances serialized from EntityMap as Vec<PlacedBuilding> per save_key for backward compat, building HP as HashMap<String, Vec<f32>> collected from entity Health, NPC data collected via SaveNpcQueries SystemParam from ECS queries including NpcWorkState for work_target), SAVE_VERSION with version-gated migration in apply_save, spawn_npcs_from_save delegates to materialize_npc() via NpcSpawnOverrides, restore_world_from_save() centralizes load-restore path (takes &mut EntityMap separately))
  src/settings.rs       # UserSettings persistence (serde JSON save/load, version migration v9, npc_counts BTreeMap<String,usize> with legacy farmers/archers/raiders migration, auto_upgrades, upgrade_expanded (Vec<String> for collapsible branch persistence), autosave_hours, music/sfx volume, music speed, tutorial_completed, log_faction_filter, debug_ai_decisions, zoom_speed/zoom_min/zoom_max/lod_transition, npc_log_mode (NpcLogMode: All/Faction/SelectedOnly))
  src/world.rs          # World data structs (PlacedBuilding struct for save/load backward compat only, WorldData contains only towns: Vec<Town>), world grid (WorldCell has terrain: Biome only — no building field, building presence queried via EntityMap), procedural generation (mine placement, building instances created directly in EntityMap), tileset builder (build_tileset for terrain array, build_building_atlas for building strip + wall auto-tile layers, build_extras_atlas for extras horizontal grid), wall auto-tile (wall_autotile_variant(building_map) neighbor-based offset selection, update_wall_sprites_around for placement/removal, update_all_wall_sprites for load, extract_sprite_32 + rotate_90_cw helpers), town grid, building placement/removal (place_building creates BuildingInstance directly in EntityMap with spawner fields, validates via building_map.has_building_at()), building GPU slot allocation (allocate_building_slot with tileset_idx + tower flag, free_building_slot), BuildingKind enum (Fountain/Bed/Waypoint/Farm/FarmerHome/ArcherHome/Tent/GoldMine/MinerHome/CrossbowHome/FighterHome/Road/Wall), building_tiles() (tile specs from registry), shared helpers: is_alive(), empty_slots(tg, center, grid, building_map), place_building() (unified placement: validate+pay+EntityMap+HP+GPU slot+dirty flags+wall auto-tile update, takes world_pos, rejects water/foreign territory via in_foreign_build_area), find_nearest_free(from, entity_map, kind, town_idx) → Option<(usize, Vec2)> (returns slot+position, checks inst.occupants), resolve_spawner_npc() (takes &BuildingInstance, uses SpawnBehavior from registry, returns 9-tuple with work_slot: Option<usize>), destroy_building() (cleanup only: combat log + wall auto-tile neighbor update; callers send DamageMsg for entity death — single Dead writer is death_system), materialize_generated_world() (shared generated-world startup path)
  src/ui/
    mod.rs              # register_ui(), game startup (+ policy load), cleanup, pause menu (+ debug settings + UI scale + audio volume + AI decision logging), escape/time controls (Playing + Running states), keyboard toggles (Q=squads, H=help), build ghost preview (Road drag-line with ghost trail), slot indicators, process_destroy_system (resolves exact building slot by kind+town+grid before lethal DamageMsg), apply_ui_scale, test scene systems (bottom panel + selection overlay + target overlay + pause menu in Running state)
    main_menu.rs        # Main menu with sectioned layout (World / Difficulty / Options / Debug Options), difficulty presets (Easy/Normal/Hard) drive NPC counts + endless mode + strength, "Per Town (player & AI)" group for farms/mines/NPC homes, AI Builder Towns + AI Raider Towns count sliders, tooltips on all controls, Play / Load Game / Debug Tests, restart tutorial button
    game_hud.rs         # Top bar (food + gold + FPS), jukebox overlay (track picker + pause/skip/loop/speed), floating inspector with combat stats/equipment/status/carried loot + clickable faction link (bottom-left) + DC group inspector (unit count/HP/job breakdown/keep-fighting toggle when box-selected) + combat log with faction filter dropdown + clickable ">>" camera-pan on location entries (bottom-right), registry-driven spawner inspector (def.spawner + npc_def for labels, linked NPC state/squad/patrol/position/NpcWorkState via ECS queries), wall inspector (tier name/HP bar/upgrade button with cost check + deduction), mine assignment UI (slot-based via EntityMap building data), target overlay, squad overlay (numbered target circles hidden during box-select, DirectControl crosshair from ManualTarget::Npc/Building queries), selection overlay (cyan brackets for selected NPC, green brackets for DirectControl NPCs), BuildingInspectorData SystemParam (18 fields incl. ECS queries for Personality/Home/Equipment/CarriedGold/PatrolRoute), expanded Copy Debug Info (NPC: XP/traits/stats/equipment/NpcFlags dump/Activity debug repr/CombatState/ManualTarget/WorkState/squad/CarriedGold/Returning loot/PatrolRoute/miner details; Building: faction/per-type details)
    left_panel.rs       # Tabbed floating Window: Roster (R) / Upgrades (U) / Policies (P) / Patrols (T) / Squads (Q) / Factions (I) / Help (H) — Upgrades tab: Economy/Military section headers with collapsible per-NPC branches (Farmer/Miner/Town under Economy, Archer/Fighter/Crossbow under Military), driven by UPGRADES.branches with section field, CollapsingState with expand/collapse persisted to UserSettings.upgrade_expanded (default collapsed); policy persistence on tab leave, Factions tab shows policy snapshot + squad commander details per faction with per-unit stat breakdowns (Archer/Fighter/Crossbow separate), food/military desire bars via shared `debug_food_military_desire` (same logic as live AI), "Copy Debug" button (builds comprehensive debug string via build_faction_debug_string → clipboard), registry-driven intel panel (DisplayCategory Economy/Military columns, label_plural from NPC_REGISTRY), registry-driven roster (job filter buttons + row colors from NPC_REGISTRY, military-first ordering, RosterParams SystemParam with Personality query for trait display), registry-driven squad UI (per-job recruit rows from NPC_REGISTRY is_military, job-colored member list, hold-fire checkbox, attack target display)
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
    ai_building.rs    # AI building observation test — pick personality, watch AI build with 100K food+gold on 10000x10000 world (2 phases)
    miner_cycle.rs    # Miner work cycle test (5 phases)
    archer_tent_reliability.rs # Archer vs tent targeting + destruction test (5 phases)
  src/systems/
    spawn.rs            # Spawn system (MessageReader<SpawnNpcMsg>), materialize_npc() shared helper (single source of truth for fresh spawn + save-load, spawns ECS entity with full component set via nested tuple bundles + conditional inserts, registers slot→entity index in EntityMap), NpcSpawnOverrides, build_patrol_route(&EntityMap)
    stats.rs            # CombatConfig (jobs + attacks + crossbow_attack base), TownUpgrades (Vec<Vec<u8>>), UpgradeMsg message flow, dynamic UpgradeRegistry via LazyLock (UpgradeNode/UpgradeBranch with section grouping, built from NPC_REGISTRY upgrade_stats + TOWN_UPGRADES at init), UPGRADES static, resolve_combat_stats() (uses npc_def.upgrade_category + UPGRADES.stat_mult), level_from_xp(), process_upgrades_system (reads Personality from ECS query for stat re-resolve on upgrade), auto_upgrade_system, upgrade helpers (upgrade_unlocked/upgrade_available/deduct_upgrade_cost/format_upgrade_cost/missing_prereqs/upgrade_effect_summary/branch_total/expansion_cost)
    drain.rs            # Queue drain systems (drain_combat_log collects CombatLogMsg → CombatLog)
    movement.rs         # GPU position readback (writes ECS Position + NpcFlags.at_destination via Query), arrival detection, resolve_movement_system (sole SetTarget emitter via MovementIntents priority arbitration)
    combat.rs           # Attack cooldown, GPU-unified targeting (query-first NPC iteration + EntityMap for building target resolution, NPC target faction from ECS not GPU readback), process_proj_hits (unified DamageMsg), building_tower_system (fountain towers via GPU combat_targets readback at unified slot)
    health.rs           # Unified damage_system (NPC + building damage via EntityMap routing — get_instance for buildings, entities for NPCs, SetDamageFlash), unified death_system (mark dead + XP grant + building destruction + NPC cleanup + despawn + loot drop MarkVisualDirty, DeathResources SystemParam with ECS queries for Home/Personality/NpcWorkState, hide_npc/hide_building helpers, guards against double-release when work_target == occupied_slot), healing_system (query-first, emits MarkVisualDirty on healing flag changes)
    behavior.rs         # Unified decision system (query-first outer loop + DecisionNpcState/NpcDataQueries SystemParam bundles for per-entity mutable access, no Commands — NpcWorkState always-present, conditional writeback via original-value comparison + visual_key() dirty tracking via GpuUpdate::MarkVisualDirty, patrol route read inline without Vec clone), DecisionExtras includes gpu_updates MessageWriter for visual dirty signals, find_farmer_farm_target (delegates to EntityMap.find_nearest_worksite with kind-filtered cell-ring spatial expansion, WorksiteFallback::TownOnly, min-order tuple scoring, try_claim_worksite for authoritative claim), miner worksite selection (find_nearest_worksite with WorksiteFallback::AnyTown), worksite instrumentation (ws_queries/ws_fallbacks/ws_stale counters), arrivals (query-first ECS iteration, emits MarkVisualDirty on farm delivery), on_duty_tick (query-first), patrol rebuild (query-first)
    economy.rs          # Game time, farm growth, mine regen, mining_policy_system (auto-discovery + miner distribution), spawner_respawn_system (iterates EntityMap spawner instances), starvation_system (query-first), squad_cleanup_system (query-first recruit pool, wave-aware), migration spawn/attach/settle (MigrationResources SystemParam with NpcFlags + Home queries, wipeout → respawn after 4h delay)
    ai_player.rs        # AI decision system with personalities (Aggressive/Balanced/Economic), weighted random scoring with retry loop (failed actions filtered via discriminant, re-pick from remaining), AiBuildRes SystemParam bundle, TownContext per-tick bundle (center/food/slots/mines via TownContext::build()), AiTownSnapshot caching with smart slot scoring (NeighborCounts incl. crossbow_homes, balanced_farm_ray_score, balanced_house_side_score), MineAnalysis single-pass (analyze_mines with all_positions), Option<MineAnalysis> type-safe builder-only enforcement, try_build_scored/try_build_inner/try_build_miner_home unified build helpers, waypoint_ring_slots (perimeter walk: corners always included, min 5 Manhattan spacing, skips road slots except corners), sync_patrol_perimeter_system (PerimeterSyncDirty flag-gated waypoint pruning against ideal ring, blocked slots count as covered, full destroy_building teardown), ai_squad_commander_system (wave-based attack cycle for Builder+Raider AIs, SquadUnit query, SquadRole Attack/Reserve/Idle, defense_share_pct + attack_split_weight per personality, wave gather→threshold→dispatch→retreat model, pick_raider_farm_target for raider towns, self-healing ownership scan), BuildCrossbowHome action (scored after 2+ archer homes), BuildRoads action (personality grid patterns, batch placement, Chebyshev distance ≤ 2 adjacency, grid-snapped positions), Phase 1/Phase 2 split (building + upgrade per tick), economy desire signal (slot fullness floors other desires), road+waypoint-aware slot placement (is_road_slot + waypoint_ring_slots filter), debug_ai_decisions logging (failed actions with scores to last_actions), drain systems (ai_dirty_drain_system → AiSnapshotDirty, perimeter_dirty_drain_system → PerimeterSyncDirty) resolve MessageReader/Writer conflicts
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
      wood_walls_131x32.png            # Wall sprites (131×32 strip: E-W straight, cross, BR corner, T-junction; rotated at runtime for auto-tile)
      house.png                       # Farmer Home building sprite (32x32, External tileset)
      barracks.png                    # Archer Home building sprite (32x32, External tileset)
      waypoint.png                    # Waypoint building sprite (32x32, External tileset)
      miner_house.png                 # Miner Home building sprite (32x32, External tileset)
      fighter_home.png                # Fighter Home building sprite (32x32, External tileset)
    sounds/
      music/not-jam-music/  # 22 .ogg tracks (Not Jam Music Pack, CC0)
    shaders/
      npc_compute.wgsl      # WGSL compute shader (unified entity grid: NPCs + buildings, entity_flags bitmask for type differentiation, movement + spatial grid + combat targeting for NPCs + towers, tile_flags road system: speed bonus + collision bypass + attraction + wall collision: faction-aware enemy blocking)
      npc_render.wgsl       # WGSL render shader (dual vertex path: storage buffer + instanced)
      projectile_compute.wgsl # WGSL compute shader (projectile movement + entity collision: hits both NPCs and buildings via unified spatial grid)
```
