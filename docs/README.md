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
- `TestSetupParams`: SystemParam bundle for test setup (slot alloc, spawn, world data, food, factions, game time, test state)
- `test_is("name")` run condition gates per-test setup/tick systems
- Each test exports `setup` (OnEnter Running) + `tick` (Update after Behavior)
- Helpers: `tick_elapsed()`, `require_entity()` reduce boilerplate
- Cleanup on OnExit(Running): despawn all NPC + FarmReadyMarker entities, reset all resources
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
| `guard-patrol` | 5 | OnDuty → Patrolling → OnDuty → rest when tired → resume |
| `farmer-cycle` | 5 | GoingToWork → Working → tired → rest → recover → return |
| `raider-cycle` | 5 | Dispatch group → arrive at farm → steal → return → deliver |
| `combat` | 6 | GPU targeting → Fighting → damage → health drop → death → slot freed |
| `projectiles` | 4 | Ranged targeting → projectile spawn → hit + damage → slot freed |
| `healing` | 3 | Damaged NPC near town → Healing marker → health recovers to max |
| `economy` | 5 | Farm growing → ready → harvest → camp forage → raider respawn |
| `world-gen` | 6 | Grid dimensions, town placement, buildings, terrain, camps |
| `sleep-visual` | 3 | Resting NPC gets SLEEP_SPRITE on status layer, cleared on wake |
| `farm-visual` | 3 | Ready farm spawns FarmReadyMarker, removed on harvest |
| `heal-visual` | 3 | Healing NPC gets HEAL_SPRITE on healing layer, cleared when healed |

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
    │   ├─ Main menu: world config sliders + Play / Debug Tests
    │   ├─ Game startup: world gen + NPC spawn (OnEnter Playing)
    │   ├─ Top bar: panel toggles left, town name + time center, stats right
    │   ├─ Bottom panel: NPC inspector (left) + combat log with filters (right)
    │   ├─ Left panel: floating Window with Roster (R) / Upgrades (U) / Policies (P)
    │   ├─ FPS overlay: bottom-right corner, EMA-smoothed, always visible (all states)
    │   ├─ Build menu: right-click context menu (Farm/Bed/GuardPost/Destroy/Unlock/Turret toggle)
    │   ├─ Pause menu (ESC): Resume, Settings (scroll speed + log/debug filters), Exit to Main Menu
    │   └─ Game cleanup: despawn + reset (OnExit Playing)
    │
    ├─ Messages (static queues) ───────────▶ [messages.md]
    │
    ├─ GPU Compute (gpu.rs) ───────────────▶ [gpu-compute.md]
    │   ├─ Bevy render graph integration
    │   └─ WGSL shader (shaders/npc_compute.wgsl)
    │
    ├─ NPC Instanced Rendering (npc_render.rs) ─▶ [rendering.md]
    │   ├─ RenderCommand + Transparent2d phase
    │   ├─ Two vertex buffers (quad + per-instance)
    │   └─ WGSL shader (shaders/npc_render.wgsl)
    │
    ├─ Sprite Rendering (render.rs) ───────────▶ [rendering.md]
    │   ├─ 2D camera, texture atlases
    │   └─ Character + world sprite sheets
    │
    ├─ Bevy Systems (gated on AppState::Playing | Running)
    │   ├─ Spawn systems ──────────────────▶ [spawn.md]
    │   ├─ Combat pipeline ────────────────▶ [combat.md]
    │   ├─ Behavior systems ───────────────▶ [behavior.md]
    │   └─ Economy systems ────────────────▶ [economy.md]
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
| [gpu-compute.md](gpu-compute.md) | Compute shaders, spatial grid, separation physics, combat targeting, GPU→ECS readback | 9/10 |
| [rendering.md](rendering.md) | TilemapChunk terrain, GPU instanced buildings/NPCs/equipment, dual atlas, RenderCommand pipeline, camera controls, health bars, FPS overlay | 9/10 |
| [combat.md](combat.md) | Attack → damage → death → XP grant → cleanup, slot recycling | 8/10 |
| [spawn.md](spawn.md) | Single spawn path, job-as-template, slot allocation | 7/10 |
| [behavior.md](behavior.md) | Decision system, utility AI, state machine, energy, patrol, flee/leash | 8/10 |
| [economy.md](economy.md) | Farm growth, food theft, starvation, camp foraging, raider respawning, FarmYield upgrade | 8/10 |
| [messages.md](messages.md) | Static queues, GpuUpdateMsg messages, GPU_READ_STATE | 7/10 |
| [resources.md](resources.md) | Bevy resources, game state ownership, UI caches, world data | 7/10 |
| [projectiles.md](projectiles.md) | GPU projectile compute, hit detection, instanced rendering, slot allocation | 7/10 |
| [concepts.md](concepts.md) | Foundational patterns (DOD, spatial grid, compute shaders, ECS) | - |
| [roadmap.md](roadmap.md) | Feature tracking, migration plan | - |

Ratings reflect system quality, not doc accuracy.

## File Map

```
rust/
  Cargo.toml            # Pure Bevy 0.18 + bevy_egui + bytemuck + rand + noise
  src/main.rs           # Bevy App entry point, asset root = project root, maximize window on startup
  src/lib.rs            # build_app(), AppState enum, system scheduling, helpers
  src/gpu.rs            # GPU compute via Bevy render graph
  src/npc_render.rs     # GPU instanced NPC rendering (RenderCommand + Transparent2d)
  src/render.rs         # 2D camera, texture atlases, TilemapChunk spawning, BuildingChunk sync
  src/messages.rs       # Static queues (GpuUpdate), Message types
  src/components.rs     # ECS components (NpcIndex, Job, Energy, Health, LastHitBy, BaseAttackType, CachedStats, Activity/CombatState enums)
  src/constants.rs      # Tuning parameters (grid size, separation, energy rates, guard post turret)
  src/resources.rs      # Bevy resources (NpcCount, GameTime, FactionStats, GuardPostState, ReassignQueue, etc.)
  src/settings.rs       # UserSettings persistence (serde JSON save/load)
  src/world.rs          # World data structs, world grid, procedural generation, tileset builder, town grid, building placement/removal
  src/ui/
    mod.rs              # register_ui(), game startup (+ policy load), cleanup, pause menu (+ debug settings), escape/time controls, keyboard toggles, slot right-click, slot indicators
    main_menu.rs        # Main menu with world config sliders + Play / Debug Tests buttons + settings persistence
    game_hud.rs         # Top bar, bottom panel (inspector + combat log), target overlay, FPS counter
    right_panel.rs      # Tabbed floating Window: Roster (R) / Upgrades (U) / Policies (P) — policy persistence on tab leave
    build_menu.rs       # Right-click context menu: build/destroy/unlock town slots, turret toggle
  src/tests/
    mod.rs              # Test framework (TestState, menu UI, HUD, cleanup)
    vertical_slice.rs   # Full core loop test (8 phases, spawn→combat→death→respawn)
    spawning.rs         # Spawn + slot reuse test (4 phases)
    energy.rs           # Energy drain test (3 phases)
    movement.rs         # Movement + arrival test (3 phases)
    guard_patrol.rs     # Guard patrol cycle (5 phases)
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
  src/systems/
    spawn.rs            # Spawn system (MessageReader<SpawnNpcMsg>), reassign_npc_system (Farmer↔Guard)
    stats.rs            # CombatConfig, TownUpgrades, UpgradeQueue, resolve_combat_stats(), xp_grant_system, process_upgrades_system
    drain.rs            # Queue drain systems, reset, collect_gpu_updates
    movement.rs         # GPU position readback, arrival detection
    combat.rs           # Attack cooldown, targeting, guard post turret auto-attack
    health.rs           # Damage, death, cleanup, healing
    behavior.rs         # Unified decision system, arrivals
    economy.rs          # Game time, farm growth, respawning
    energy.rs           # Energy drain/recovery
    sync.rs             # GPU state sync

shaders/
  npc_compute.wgsl      # WGSL compute shader (movement + spatial grid + combat targeting)
  npc_render.wgsl       # WGSL render shader (instanced quad + sprite atlas)
  projectile_compute.wgsl # WGSL compute shader (projectile movement + collision)

assets/
  roguelikeChar_transparent.png   # Character sprites (54x12 grid, 16px + 1px margin)
  roguelikeSheet_transparent.png  # World sprites (57x31 grid, 16px + 1px margin)
```

