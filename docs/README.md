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
    │   ├─ Floating windows: NPC/building inspector with combat stats + equipment + status (bottom-left) + combat log with filters (bottom-right)
    │   ├─ Left panel: floating Window with Roster (R) / Upgrades (U) / Policies (P) / Patrols (T) / Squads (Q) / Factions (I) / Help (H)
    │   ├─ Jukebox overlay: top-right, track picker dropdown + pause/skip/loop/speed controls
    │   ├─ Build menu: bottom-center horizontal bar with building sprites + help text; click-to-place with grid-snapped ghost preview; destroy mode in bar + inspector
    │   ├─ Tutorial: 20-step guided walkthrough (camera → building → NPC interaction → upgrades → policies → patrols → squads); condition-driven auto-advance + manual Next/Skip
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
    │   ├─ RenderCommand + Transparent2d phase
    │   ├─ Storage buffer path (NPCs): vertex shader reads compute output directly
    │   ├─ Instance buffer path (farms, BHP, projectiles)
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
    │   ├─ AI player system ─────────────▶ personality-driven build/unlock/upgrade for non-player factions; active flag for deferred migration camps
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
| [rendering.md](rendering.md) | TilemapChunk terrain, GPU instanced buildings/NPCs/equipment, dual atlas, RenderCommand pipeline, camera controls, health bars, FPS overlay | 8/10 |
| [combat.md](combat.md) | Attack → damage → death → XP grant → cleanup, slot recycling | 8/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | Decision system, utility AI, state machine, energy, patrol, flee/leash | 7/10 |
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
  src/main.rs           # Bevy App entry point, EmbeddedAssetPlugin (release), maximize window on startup
  src/lib.rs            # build_app(), AppState enum, system scheduling, helpers
  src/gpu.rs            # GPU compute via Bevy render graph
  src/npc_render.rs     # GPU NPC rendering (storage buffers) + misc/projectile rendering (instance buffers)
  src/render.rs         # 2D camera, texture atlases, TilemapChunk spawning, TerrainChunk + BuildingChunk sync
  src/messages.rs       # Static queues (GpuUpdate), Message types
  src/components.rs     # ECS components (NpcIndex, Job, Energy, Health, LastHitBy, BaseAttackType, CachedStats, Activity/CombatState enums, SquadId, CarriedGold, Archer/Farmer/Miner markers, Migrating)
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates, guard post turret, squad limits, mining, building HP, building costs, 8x8 base build area)
  src/resources.rs      # Bevy resources (SlotAllocator, GameTime, FactionStats, GuardPostState, SquadState, GoldStorage, MineStates, BuildingHpState, HelpCatalog, etc.)
  src/save.rs            # Save/load system (F5/F9 quicksave/load, autosave with 3 rotating slots, save file picker via list_saves/read_save_from, SaveData serialization, SystemParam bundles)
  src/settings.rs       # UserSettings persistence (serde JSON save/load, version migration v4, auto_upgrades, autosave_hours, music/sfx volume, music speed, tutorial_completed)
  src/world.rs          # World data structs (GoldMine, MinerHome, FarmerHome, ArcherHome), world grid, procedural generation (mine placement), tileset builder, town grid, building placement/removal, BuildingSpatialGrid (CPU spatial grid for O(1) building lookups, faction-aware), shared helpers: build_and_pay(), register_spawner(), resolve_spawner_npc(), destroy_building(), find_nearest_enemy_building(), Building::kind()/spawner_kind()
  src/ui/
    mod.rs              # register_ui(), game startup (+ policy load), cleanup, pause menu (+ debug settings + UI scale + audio volume), escape/time controls, keyboard toggles (Q=squads, H=help), build ghost preview, slot indicators, process_destroy_system, apply_ui_scale
    main_menu.rs        # Main menu with difficulty presets (Easy/Normal/Hard), world config sliders (farms + gold mines top-level, farmer/archer homes under AI Towns, tents under Raider Camps), Play / Load Game / Debug Tests, restart tutorial button
    game_hud.rs         # Top bar (food + gold + FPS), jukebox overlay (track picker + pause/skip/loop/speed), floating inspector with combat stats/equipment/status (bottom-left) + combat log (bottom-right), target overlay, squad overlay
    left_panel.rs       # Tabbed floating Window: Roster (R) / Upgrades (U) / Policies (P) / Patrols (T) / Squads (Q) / Factions (I) / Help (H) — policy persistence on tab leave
    build_menu.rs       # Bottom-center build bar: building sprites with cached atlas extraction, click-to-place, destroy mode, cursor hint
    tutorial.rs         # 20-step guided tutorial: condition-driven hints (action triggers + info-only Next steps), skip per-step or all, persisted completion in UserSettings
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
  src/systems/
    spawn.rs            # Spawn system (MessageReader<SpawnNpcMsg>)
    stats.rs            # CombatConfig, TownUpgrades, UpgradeQueue, UPGRADE_REGISTRY (16 upgrades, prereqs + multi-resource cost), UPGRADE_RENDER_ORDER (tree UI layout), resolve_combat_stats(), xp_grant_system, process_upgrades_system, auto_upgrade_system, upgrade helpers (upgrade_unlocked/upgrade_available/deduct_upgrade_cost/format_upgrade_cost/missing_prereqs/upgrade_effect_summary/branch_total/expansion_cost)
    drain.rs            # Queue drain systems, reset, collect_gpu_updates
    movement.rs         # GPU position readback, arrival detection
    combat.rs           # Attack cooldown, targeting, building attack fallback, guard post turret, building_damage_system
    health.rs           # Damage, death, cleanup, healing
    behavior.rs         # Unified decision system, arrivals
    economy.rs          # Game time, farm growth, mine regen, respawning, building spawners, squad cleanup, migration spawn/attach/settle
    ai_player.rs        # AI decision system with personalities (Aggressive/Balanced/Economic), weighted random scoring, AiBuildRes SystemParam bundle
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
      guard_post.png                  # Guard post building sprite (32x32, External tileset)
    sounds/
      music/not-jam-music/  # 22 .ogg tracks (Not Jam Music Pack, CC0)
    shaders/
      npc_compute.wgsl      # WGSL compute shader (movement + spatial grid + combat targeting)
      npc_render.wgsl       # WGSL render shader (dual vertex path: storage buffer + instanced)
      projectile_compute.wgsl # WGSL compute shader (projectile movement + collision)
```
