# Endless Architecture Docs

## How to Use These Docs

These docs are the **source of truth** for system architecture. When building new features or modifying existing systems:

1. **Before coding**: Read the relevant doc to understand the current architecture, data flow, and known issues.
   For hot-path changes (per-frame/per-tick code), also apply [performance.md](performance.md).
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
- `TestSetupParams`: SystemParam bundle for test setup (slot alloc, spawn, world data, town_access, factions, game time, test state, commands, gpu_updates, dirty_writers) — `add_town` auto-inits pathfind costs, `add_building`/`add_waypoint` fire `BuildingGridDirtyMsg` via dirty_writers (same pipeline as main game)
- `test_is("name")` run condition gates per-test setup/tick systems
- Each test exports `setup` (OnEnter Running) + `tick` (FixedUpdate after Behavior)
- Helpers: `tick_elapsed()`, `require_entity()` reduce boilerplate
- Cleanup on OnExit(Running): shared `game_cleanup_system` (same as OnExit Playing) — despawn all entities, reset all resources
- Run All: sequential execution via `RunAllState` queue (auto-advances after 1.5s, instant in CLI mode)
- Single tests stay running after pass/fail — user clicks Back in HUD to return
- CLI mode: `--test all` or `--test <name>` runs tests headless-style and exits with pass/fail summary (exit code 0/1)

**HUD**: Phase checklist overlay during test execution — gray `○` pending, yellow `▶` active, green `✓` passed, red `✗` failed. Back/Cancel button at bottom.

**Tests** (`src/tests/`):

| Test | Phases | What it validates |
|------|--------|-------------------|
| `vertical-slice` | 8 | Full core loop: spawn → work → raid → combat → death → respawn |
| `spawning` | 4 | Spawn entities, kill via health=0, slot freed, slot reused |
| `energy` | 3 | Energy starts at 100, drains over time, reaches ENERGY_HUNGRY |
| `movement` | 3 | Path-driven waypoint advancement, GPU positions update, AtDestination on arrival |
| `archer-patrol` | 5 | Patrol(guard) → Patrol(walk) → Patrol(guard) → rest when tired → resume |
| `farmer-cycle` | 5 | Work(transit) → Work(at_dest) → tired → rest → recover → return |
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
| `endless-mode` | 14 | Builder + raider fountain destroyed → spawn queued → boat migration → settle (both AI kinds) |
| `ai-building` | 2 | AI town building observation: pick personality, watch it build with 100K food+gold on a 10000x10000 map |
| `miner-cycle` | 5 | Miner: walk to mine → tend → harvest gold → deliver → rest |
| `archer-tent-reliability` | 5 | Archer vs enemy tent: target lock, projectile activity, sustained tent damage, destruction |
| `slot-reuse-wave` | 5 | AI wave vs destroyed building: Entity identity prevents ABA slot reuse (wave correctly ends) |
| `coalesce-movement` | 2 | GPU-authoritative position safety: SetPosition on foreign slot doesn't teleport other NPCs |
| `coalesce-arrival` | 2 | GPU-authoritative arrival safety: arrived NPC stays stable after unrelated SetTarget |
| `pathfind-maze` | 5 | NPCs navigate serpentine wall maze via A* pathfinding (configurable count 1-5000, slider UI) |
| `sandbox` | 1 | Human player sandbox: 1 player + 1 AI town, 100K food+gold, no raiders — auto-completes for free play |

## System Map

```
Pure Bevy App (main.rs)
    │
    ▼
Bevy ECS (lib.rs build_app)
    │
    ├─ AppState: MainMenu → Playing | TestMenu → Running
    │
    ├─ UI (ui/) ──────────────────────▶ main_menu, game_hud, panels, startup, cleanup
    │   ├─ Main menu, game startup, pause menu (ESC), game over screen
    │   ├─ Top bar (stats + UPS/FPS), jukebox, build menu, tutorial
    │   ├─ Left panel: Roster / Upgrades / Policies / Patrols / Squads / Inventory / Factions / Help
    │   ├─ Inspector (NPC/building/DC-group) + combat log + casino popup
    │   └─ Game cleanup: despawn + reset (shared by OnExit Playing + OnExit Running)
    │
    ├─ Messages (static queues) ──────▶ [messages.md]
    ├─ GPU Compute (gpu.rs) ──────────▶ [gpu-compute.md]
    ├─ Rendering (npc_render + render) ▶ [rendering.md]
    │
    ├─ Bevy Systems (FixedUpdate 60 Hz, gated on Playing | Running)
    │   ├─ Spawn ─────────────────────▶ [spawn.md]
    │   ├─ Combat ────────────────────▶ [combat.md]
    │   ├─ Behavior ──────────────────▶ [behavior.md]
    │   ├─ Economy ───────────────────▶ [economy.md]
    │   ├─ AI player ─────────────────▶ [ai-player.md]
    │   ├─ Audio (jukebox + SFX)
    │   └─ LLM player ───────────────▶ [llm-player.md]
    │
    ├─ BRP (bevy_remote) ─────────────▶ [brp.md]
    └─ Test Framework (tests/)

Frame execution order ────────────────▶ [frame-loop.md]
```

## Documents

| Doc | What it covers | Rating |
|-----|---------------|--------|
| [frame-loop.md](frame-loop.md) | Fixed 60 UPS game loop, FixedUpdate/Update split, main/render world timing | 9/10 |
| [gpu-compute.md](gpu-compute.md) | Compute shaders, spatial grid, separation physics, combat targeting, GPU→ECS readback | 8/10 |
| [rendering.md](rendering.md) | TilemapChunk terrain, GPU instanced buildings/NPCs/equipment, 4-atlas pipeline (char/world/extras/building), explicit sort-key pass ordering, RenderCommand pattern, camera controls, health bars | 8/10 |
| [combat.md](combat.md) | Attack → damage → death → XP grant → cleanup, slot recycling | 8/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation, DRY save-load via materialize_npc | 8/10 |
| [behavior.md](behavior.md) | Decision system (fieldless ActivityKind + ActivityDef registry + Distraction enum), utility AI, at_destination flag, energy, patrol, flee/leash (bucketing formulas → performance.md) | 8/10 |
| [ai-player.md](ai-player.md) | AI decision loop, hunger system, building scoring, slot placement, squad commander, migration | 8/10 |
| [economy.md](economy.md) | Farm growth, food theft, starvation, raider foraging, spawner respawn (ECS ProductionState/SpawnerState/ConstructionProgress), dynamic raider town migration (spawn→boat→disembark→walk→settle) | 8/10 |
| [messages.md](messages.md) | Message flow, GpuUpdateMsg, GAME_CONFIG_STAGING, readback resources (authority → [authority.md](authority.md)) | 7/10 |
| [resources.md](resources.md) | Bevy resources, game state ownership, UI caches, world data | 7/10 |
| [projectiles.md](projectiles.md) | GPU projectile compute, hit detection, instanced rendering, slot allocation | 7/10 |
| [authority.md](authority.md) | Complete data ownership (GPU/CPU/render-only), hard rules, staleness budget, slot namespace | 9/10 |
| [performance.md](performance.md) | Complete perf authority: GPU patterns, CPU cadencing, data access rules, anti-patterns, PR review | 9/10 |
| [brp.md](brp.md) | Live game data access via HTTP JSON-RPC (localhost:15702), reflected types, query examples | 9/10 |
| [llm-player.md](llm-player.md) | Built-in LLM player (claude --print), external player setup, data model, token budget | - |
| [k8s.md](k8s.md) | CRD architecture (Def→Instance→Controller), K8s mapping, compliance checklist | - |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, migration plan | - |

Ratings reflect system quality, not doc accuracy.

## File Map

```
rust/
  Cargo.toml              # Bevy 0.18 + bevy_egui + bytemuck + rand + noise; benches: hashmap_bench, system_bench
  src/
    main.rs               # App entry, crash handler, --autostart / --test CLI flags
    lib.rs                # build_app(), AppState, system scheduling, autostart_system
    tracing_layer.rs      # Per-system EMA timing + rolling peak spike detection for profiler UI
    gpu.rs                # GPU compute pipeline, buffer management, populate/extract → [gpu-compute.md]
    npc_render.rs         # Storage + instance buffer rendering, coalescing upload → [rendering.md]
    render.rs             # Camera, texture atlases, click/box select, tilemap chunks → [rendering.md]
    messages.rs           # Static message queues, GpuUpdate variants, DirtyWriters → [messages.md]
    components.rs         # All ECS components (NPC, building, town, equipment, traits)
    constants/
      mod.rs              # Tuning constants (behavior, projectile, raider, tower, tile flags, town registry)
      upgrades.rs         # UpgradeStatDef, ResourceKind, upgrade tables (military, farmer, miner, tower, town)
      npcs.rs             # NPC_REGISTRY, ACTIVITY_REGISTRY, equipment/loot types, roll_loot_item
      buildings.rs        # BUILDING_REGISTRY, BuildingDef, TileSpec, autotile helpers
    entity_map.rs         # DenseSlotMap, EntityMap (slot↔entity index + building spatial grid)
    resources.rs          # Bevy resources (GpuSlotPool, GameTime, UiState, squads, factions, reputation)
    systemparams.rs       # TownAccess and other shared SystemParam bundles
    save.rs               # Save/load (quicksave, autosave, named saves, version migration)
    settings.rs           # UserSettings persistence (serde JSON, version migration, key bindings)
    world.rs              # WorldGrid, procedural gen, place/destroy_building, auto-tile, BuildingKind
    ui/
      mod.rs              # UI registration, startup/cleanup, pause menu, settings panel, game over
      main_menu.rs        # World/difficulty config, AI lobby, play/load/settings/exit
      game_hud.rs         # Top bar, inspector, combat log, jukebox, build ghost, squad overlay
      left_panel/
        mod.rs            # Tab dispatch + Policies/Patrols/Squads/Factions/Profiler/Help content
        roster_ui.rs      # NPC roster table with job filters
        upgrades_ui.rs    # Economy/Military upgrade branches (collapsible)
        tech_tree.rs      # Visual top-down tech tree window
        inventory_ui.rs   # Equipment inventory with slot filters, comparison, bulk sell
      blackjack.rs        # Casino blackjack minigame popup
      build_menu.rs       # Bottom build bar (Economy/Military/Tower tabs, click-to-place)
      tutorial.rs         # 24-step guided tutorial with condition-driven advance
    systems/
      spawn.rs            # materialize_npc() single spawn path → [spawn.md]
      stats.rs            # UpgradeRegistry, resolve_combat_stats, auto-upgrade/equip systems
      drain.rs            # Queue drain (CombatLogMsg → CombatLog)
      movement.rs         # GPU position readback, HPA* path routing, MovementIntent resolution
      combat.rs           # Attack cooldown, GPU targeting, projectile fire, tower system → [combat.md]
      health.rs           # Damage, death (XP/loot/cleanup), healing, HP regen → [combat.md]
      behavior.rs         # NPC decision system, utility AI, patrol, flee/leash → [behavior.md]
      work_targeting.rs   # Centralized worksite claim/release/retarget resolver
      economy/            # Farm/mine growth, construction, spawner respawn, migration → [economy.md]
      ai_player.rs        # AI personalities, building scoring, squad commander → [ai-player.md]
      audio.rs            # Music jukebox (22 tracks) + spatial SFX
      remote.rs           # Custom BRP endpoints (summary, build, upgrade, etc.) → [brp.md]
      llm_player.rs       # Built-in claude --print LLM player → [llm-player.md]
      energy.rs           # Energy drain/recovery
      sync.rs             # GPU state sync
    tests/
      mod.rs              # Test framework (TestState, menu, HUD, CLI runner)
      (25 test files)     # See test table above for full list
  assets/
    sprites/              # Character/world sprite sheets (16px grid), building sprites (32-64px), FX
    sounds/               # 22 music tracks (CC0) + SFX (shoot, 24 death variants)
    shaders/              # npc_compute.wgsl, npc_render.wgsl, projectile_compute.wgsl
```
