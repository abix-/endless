# Changelog

## 2026-01-24
- add start menu with world size, town count, farmers/guards/raiders sliders (max 500)
- add GPU compute shader for NPC separation forces
- add parallel processing thread-safe state transitions (pending arrivals)
- add co-movement separation reduction (groups move without oscillation)
- add velocity damping for smooth collision avoidance
- add drift detection (working NPCs walk back if pushed off position)
- add flee/leash checks to parallel fighting path
- fix guard patrol route (clockwise perimeter, not diagonal through town)
- fix guard schedule (work 24/7, rest only when energy low)
- reduce guard posts from 6 to 4 (corner perimeter)
- reduce max farmers per farm from 4 to 1
- add farm click menu (shows occupant name)
- fix farmers entering FARMING state without farm reservation
- disable NPC size scaling with levels
- add Rust/Bevy ECS POC: 10,000 NPCs @ 140fps (release build, 2x debug)
- add GPU compute shader separation with spatial grid (Phase 1 complete)

## 2026-01-20
- add noise-based terrain with grass, forest, water, rock biomes
- add sprite tiling: water (2x2), dirt (2x2), forest (6 tree types 2x2)
- add terrain tile inspection on click
- fix rock sprites (2x2, variable sprite sizes)

## 2026-01-19
- add 8000x8000 world with visible border and corner markers
- add 7 named towns (Florida cities) with 1200px minimum spacing
- add farm tracking (max 4 farmers per farm, nearest free farm)
- add bed tracking (NPCs reserve closest free bed)
- add guard patrol between 6 posts with day/night shifts for even coverage
- add guard post turrets (individually upgradeable, 9999 levels, exponential cost)
- add flee behavior (guards <33% HP, raiders <50% HP, flee to home base)
- add target switching (stop chasing fleeing enemies if closer threat exists)
- add TCP-like collision avoidance (head-on, crossing, overtaking)
- add fountain 2x2 with 10x healing multiplier and halo shader
- add raider camp 5x regen (raiders excluded from fountain healing)
- add balanced combat stats: guards/raiders 120hp, 15dmg, 150 range
- add NPC names (55 first x 100 last = 5500 combinations) with rename feature
- add personality traits (9 types, 40% chance: brave, coward, efficient, hardy, lazy, strong, swift, sharpshot, berserker)
- add faction color tinting (guards blue, raiders red, farmers green)
- add faction policies panel (P key): flee thresholds, recovery, leash, off-duty behavior
- add roster panel (R key) with sorting, filtering, auto-upgrade checkboxes
- add upgrade menu: guard stats, economy, utility (10 levels each)
- add build menu with 6x6 grid slots (farms, beds, guard posts)
- add destroy buildings (right-click slot)
- add expandable building grid (double-click to unlock adjacent slots, up to 100x100)
- add town circle expands with building range
- add WANDERING state for off-duty NPCs
- add activity-specific NPC states (no translation layer)
- add per-NPC arrival radius based on building sprite size
- add loot icon for raiders carrying food
- add passive HP regen (2/hr awake, 6/hr sleeping)
- add mouse wheel zoom centers on cursor position
- add settings menu (ESC) with HP bar modes (off/damaged/always)
- add scroll speed setting (100-2000)
- add resizable combat log, consolidated UI panels, population caps
- add food tracking per town/camp with HUD display
- add sprite composition system (farm 3x3, house 2x2, camp 2x2)
- remove player sprite (camera-only control)
- fix camp placement (pick direction with most room, away from all towns)
- fix combat log freeze (batch updates)
- fix smooth separation (velocity-based instead of position jumps)
- fix raiders stuck at camp / not delivering food
- fix wounded raiders causing stack overflow (rest at camp instead of looping)

## 2026-01-18
- add projectile system with faction-colored projectiles (blue guards, red raiders)
- add level/XP system (sqrt scaling, level 9999 = 100x stats)
- add raider camps with per-camp spawning
- add attack flash effect
- add combat log with level-up notifications
- add raider AI: steal food from farms, return to camp
- add staggered separation (1/4 NPCs per frame)
- add camera culling (only render visible NPCs)
- add scan stagger (1/8 NPCs per frame for combat)
- refactor: extract config.gd, split NPC systems into manager/state/nav/combat/needs/grid/renderer

## 2026-01-01
- revive project: add HUD, basic NPC behavior

## 2025-03-02
- initial prototype: persistent state, NPC system
