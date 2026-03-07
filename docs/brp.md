# Bevy Remote Protocol (BRP)

Live game state access via HTTP JSON-RPC. Query any ECS component or resource while the game is running.

## Overview

Bevy 0.18's built-in `bevy_remote` crate runs an HTTP server on **localhost:15702**. All reflected components and resources are queryable via `curl` or any HTTP client. Zero performance impact — BRP runs on a background thread and only does work when queried.

**Setup** (`lib.rs`):
```rust
use bevy::remote::{RemotePlugin, http::RemoteHttpPlugin};

app.add_plugins(RemotePlugin::default())
   .add_plugins(RemoteHttpPlugin::default())
   .register_type::<Position>()  // each type needs register_type
   // ...
```

**Requirements** for a type to be BRP-queryable:
1. `derive(Reflect)` on the type and all nested types
2. `#[reflect(Component)]` for components, `#[reflect(Resource)]` for resources
3. `.register_type::<T>()` in `build_app()`

## AI Model Integration

The BRP endpoints exist so any AI model can play Endless as an opponent or ally. The model acts exactly like a human player — reading game state to understand the situation, then taking actions through the same controls available in the UI.

**Token efficiency matters.** AI tokens cost money. The model doesn't need to act every tick or even every few seconds. Call `endless/summary` periodically to assess the situation, then make strategic decisions in batches. A cheap model (Haiku-class) is more than sufficient — the game isn't complex enough to need a frontier model, but even the cheapest LLM makes better strategic decisions than 10K lines of hardcoded if/else.

**Delegation, not micromanagement.** The model controls high-level strategy: personality, policies, squad targets, upgrade priorities. The in-game AI Manager handles the grunt work — building placement, road layout, combat pathing. This is the same split a human player uses: you set the AI Manager's personality and toggles in the Policies tab, then intervene only when you want to override or react to events.

**Read-heavy, write-sparse.** Most interactions are reads — `endless/summary` for a game overview, `world.query` for specific entity data, `endless/debug` for deep NPC/building inspection. Write actions (`endless/policy`, `endless/ai_manager`, `endless/squad_target`, `endless/build`, `endless/upgrade`, `endless/chat`) are infrequent strategic decisions, not per-frame commands.

**Model-agnostic.** Any HTTP client works — curl from Claude Code, Python scripts, MCP tools, OpenAI function calling, etc. The JSON-RPC interface doesn't care what model or framework is driving it.

**Access control.** The main menu has a WC3-style player lobby — each AI slot has a Builder/Raider dropdown and an LLM checkbox. Write endpoints (`build`, `upgrade`, `policy`, `ai_manager`, `squad_target`) are server-side gated to only allow towns marked as LLM-controlled. Read endpoints (`summary`, `world.query`) are unrestricted — full situational awareness. The `RemoteAllowedTowns` resource holds the allowed town indices; if empty, all towns are allowed (legacy/debug mode).

## Methods

| Method | Params | Returns |
|--------|--------|---------|
| `world.query` | `data` (components/option/has), `filter` (with/without) | Array of `{entity, components}` |
| `world.get_components` | `entity`, `components` | Single entity's component values |
| `world.get_resources` | `resource` (singular string) | Resource value |
| `world.list_components` | `entity` | All component type paths on an entity |
| `world.list_resources` | (none) | All registered resource type paths |
| `world.insert_components` | `entity`, `components` | Live-modify entity components |
| `world.mutate_resources` | `resource`, `value` | Live-modify resource values |
| `world.spawn_entity` | `components` | Create new entity |
| `world.despawn_entity` | `entity` | Remove entity |

## Type Paths

BRP uses full Rust module paths. All types are in the `endless` crate:

### Components (`endless::components::`)

| Type | Fields / Variants | Notes |
|------|-------------------|-------|
| `EntityUid` | `u64` | Stable NPC/building identity |
| `GpuSlot` | `usize` | GPU buffer index |
| `Position` | `x: f32, y: f32` | World-space position |
| `Job` | Farmer, Archer, Raider, Fighter, Miner, Crossbow, Boat | Enum, serializes as string |
| `Speed` | `f32` | Pixels/sec |
| `TownId` | `i32` | Town index |
| `Energy` | `f32` | 0-100 |
| `Home` | `Vec2` | Rest position, (-1,-1) = homeless |
| `Health` | `f32` | Current HP |
| `Faction` | `i32` | 0=player, 1+=AI/raider |
| `Activity` | Idle, Working, OnDuty, Patrolling, GoingToWork, GoingToRest, Resting, GoingToHeal, HealingAtFountain, Wandering, Raiding, Returning, Mining, MiningAtMine | Current behavior state |
| `CombatState` | None, Fighting{origin}, Fleeing | Orthogonal to Activity |
| `BaseAttackType` | Melee, Ranged | |
| `CachedStats` | damage, range, cooldown, projectile_speed, projectile_lifetime, max_health, speed, stamina, hp_regen, berserk_bonus | Resolved combat stats |
| `NpcFlags` | healing, starving, direct_control, migrating, at_destination | Boolean flags |
| `NpcWorkState` | worksite: Option\<EntityUid\> | Claimed worksite |
| `CarriedLoot` | food, gold, equipment | What NPC is carrying |
| `NpcEquipment` | helm, armor, weapon, shield, gloves, boots, belt, amulet, ring1, ring2 | All Option\<LootItem\> |
| `Personality` | trait1, trait2: Option\<TraitInstance\> | 0-2 spectrum traits |
| `PatrolRoute` | posts: Vec\<Vec2\>, current: usize | Guard waypoints |
| `NpcPath` | waypoints, current, goal_world, path_cooldown | A* path state |
| `SquadId` | `i32` | Squad assignment |
| `Building` | kind: BuildingKind | Building marker |
| `AttackTimer` | `f32` | Cooldown remaining |
| `FleeThreshold` | pct: f32 | Flee HP % |
| `LeashRange` | `f32` | Max chase distance |
| `WoundedThreshold` | pct: f32 | Recovery HP % |
| `Dead` | (unit) | Pending removal marker |
| `Stealer` | (unit) | Can steal from farms |
| `HasEnergy` | (unit) | Energy system active |
| `LastHitBy` | `i32` | Attacker slot (for XP) |
| `FarmReadyMarker` | farm_slot: usize | Farm has food ready |
| `ManualTarget` | Npc(usize), Building(Vec2), Position(Vec2) | DC target |

### Resources (`endless::resources::`)

| Type | Fields | Notes |
|------|--------|-------|
| `GameTime` | total_seconds, seconds_per_hour, start_hour, time_scale, paused, last_hour, hour_ticked | Derive day/hour/minute |
| `UpsCounter` | ticks_this_second, display_ups | Updates per second |
| `KillStats` | archer_kills, villager_kills | UI kill counters |
| `FoodStorage` | food: Vec\<i32\> | Per-town food |
| `GoldStorage` | gold: Vec\<i32\> | Per-town gold |
| `FactionStats` | stats: Vec\<FactionStat\> | Per-faction alive/dead/kills |
| `TownPolicies` | policies: Vec\<PolicySet\> | Per-town behavior config |
| `Difficulty` | Easy, Normal, Hard | Game difficulty |

### Nested Types

| Path | Type |
|------|------|
| `endless::constants::LootItem` | id, slot, rarity, stat_bonus, sprite, name |
| `endless::constants::ItemKind` | Food, Gold |
| `endless::constants::EquipmentSlot` | Helm, Armor, Weapon, Shield, Gloves, Boots, Belt, Amulet, Ring |
| `endless::constants::Rarity` | Common, Uncommon, Rare, Epic |
| `endless::world::BuildingKind` | Fountain, Bed, Waypoint, Farm, FarmerHome, ArcherHome, Tent, GoldMine, MinerHome, CrossbowHome, FighterHome, Road, Wall, Tower, Merchant, Casino |

## Query Examples

### Resources

```bash
# Game time
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.get_resources","params":{"resource":"endless::resources::GameTime"},"id":1}'
# → {"value":{"total_seconds":136.8,"seconds_per_hour":5.0,"start_hour":6,"time_scale":1.0,"paused":false,...}}

# Food per town
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.get_resources","params":{"resource":"endless::resources::FoodStorage"},"id":1}'

# Gold per town
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.get_resources","params":{"resource":"endless::resources::GoldStorage"},"id":1}'

# Town policies (flee thresholds, schedules, off-duty behavior)
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.get_resources","params":{"resource":"endless::resources::TownPolicies"},"id":1}'

# Faction stats (alive/dead/kills per faction)
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.get_resources","params":{"resource":"endless::resources::FactionStats"},"id":1}'

# Difficulty
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.get_resources","params":{"resource":"endless::resources::Difficulty"},"id":1}'
```

### Entity Queries

```bash
# All NPCs: job, faction, position, activity
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.query","params":{"data":{"components":["endless::components::Job","endless::components::Faction","endless::components::Position","endless::components::Activity"]}},"id":1}'

# All buildings: kind, position, town
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.query","params":{"data":{"components":["endless::components::Building","endless::components::Position","endless::components::TownId"]}},"id":1}'

# Military NPCs only (have SquadId)
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.query","params":{"data":{"components":["endless::components::Job","endless::components::Faction","endless::components::CombatState"],"has":["endless::components::SquadId"]}},"id":1}'

# NPCs in combat
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.query","params":{"data":{"components":["endless::components::Job","endless::components::Faction","endless::components::Health","endless::components::CombatState"]},"filter":{"without":["endless::components::Dead"]}},"id":1}'

# Deep inspect a single entity (replace ENTITY_ID)
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.get_components","params":{"entity":ENTITY_ID,"components":["endless::components::Job","endless::components::Position","endless::components::Health","endless::components::Faction","endless::components::TownId","endless::components::Activity","endless::components::CombatState","endless::components::CachedStats","endless::components::NpcFlags","endless::components::Personality","endless::components::NpcEquipment","endless::components::Energy"]},"id":1}'

# List all components on an entity (discover what it has)
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"world.list_components","params":{"entity":ENTITY_ID},"id":1}'
```

### Query Patterns

**`data.components`** — required components, values returned:
```json
{"data":{"components":["endless::components::Job","endless::components::Position"]}}
```

**`data.has`** — required components, values NOT returned (filter only):
```json
{"data":{"components":["endless::components::Job"],"has":["endless::components::SquadId"]}}
```

**`data.option`** — optional components, null when absent:
```json
{"data":{"components":["endless::components::Job"],"option":["endless::components::SquadId"]}}
```

**`filter.without`** — exclude entities with these components:
```json
{"data":{"components":["endless::components::Job"]},"filter":{"without":["endless::components::Dead"]}}
```

## Serialization Format

- Resources: `{"value": {...fields...}}`
- Queries: `[{"entity": u64, "components": {...}}, ...]`
- Simple enums serialize as strings: `"Farmer"`, `"Idle"`, `"Melee"`
- Data enums serialize as objects: `{"Raiding":{"target":{"x":1.0,"y":2.0}}}`, `{"Fighting":{"origin":{"x":5.0,"y":10.0}}}`
- Tuple structs serialize as their inner value: `Health` → `70.5`, `Faction` → `3`
- Entity IDs are Bevy-internal u64 — not EntityUid or GpuSlot

## Custom Action Endpoints

Game-specific JSON-RPC methods for live control. Registered via `RemotePlugin::default().with_method()` in `lib.rs`, implemented in `systems/remote.rs`. All responses are **TOON-formatted strings** (returned as JSON string values through BRP). Typed Rust structs serialized via `serde_toon2`.

### endless/summary

Get a high-level game state overview. Auto-filters to the LLM-controlled town (shows one town, not all).

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `town` | usize | no | Filter to a single town index |

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/summary","params":{},"id":1}'
```

Returns TOON with: day, hour, minute, paused, time_scale, town_idx, town_name, faction, food, gold, factions (tuple rows), buildings (kind,col,row — world grid coords), squads (idx,members,target_x,target_y), upgrades (idx,name,level,pct,cost), combat_log (day,hour,min,msg), inbox (from_town,message,day,hour,min), npcs (compact per-job counts).

- `inbox`: read-only — messages persist in `ChatInbox` across reads (flag-based `sent_to_llm` dedup for LLM delivery)
- `combat_log`: last 20 events from `RemoteCombatLogRing` resource, filtered to town's faction
- `upgrades`: per-town levels from `TownUpgrades`, costs from `UPGRADES` registry
- `npcs`: compact format — `Archer: 8 (Patrolling:5 OnDuty:3)` collapsed from verbose per-activity keys

### endless/build

Queue a building placement. Executes next FixedUpdate tick via drain system.

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `town` | usize | yes | Town index |
| `kind` | string | yes | BuildingKind name (e.g. "Farm", "Wall", "Tower") |
| `col` | usize | yes | World grid column |
| `row` | usize | yes | World grid row |

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/build","params":{"town":0,"kind":"Farm","col":172,"row":125},"id":1}'
```

### endless/upgrade

Queue a town upgrade purchase. Executes next FixedUpdate tick.

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `town` | usize | yes | Town index |
| `upgrade_idx` | usize | yes | Index into UPGRADES array |

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/upgrade","params":{"town":0,"upgrade_idx":3},"id":1}'
```

### endless/policy

Set town behavior policies. Only provided fields are changed.

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `town` | usize | yes | Town index |
| `eat_food` | bool | no | Allow NPCs to eat food |
| `archer_aggressive` | bool | no | Archers attack on sight |
| `archer_leash` | bool | no | Archers return after chase |
| `farmer_fight_back` | bool | no | Farmers fight instead of flee |
| `prioritize_healing` | bool | no | Heal before working |
| `farmer_flee_hp` | f32 | no | Farmer flee HP threshold |
| `archer_flee_hp` | f32 | no | Archer flee HP threshold |
| `recovery_hp` | f32 | no | HP % to resume work after healing |
| `mining_radius` | f32 | no | Gold mine discovery radius |

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/policy","params":{"town":0,"archer_aggressive":true,"farmer_flee_hp":0.3},"id":1}'
```

### endless/time

Control game time — pause/unpause and set time scale.

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `paused` | bool | no | Pause/unpause |
| `time_scale` | f32 | no | Speed multiplier (clamped 0–20) |

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/time","params":{"time_scale":5.0},"id":1}'
```

### endless/squad_target

Set a movement target for a military squad.

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `squad` | usize | yes | Squad index |
| `x` | f32 | yes | Target X position |
| `y` | f32 | yes | Target Y position |

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/squad_target","params":{"squad":0,"x":500.0,"y":300.0},"id":1}'
```

### endless/ai_manager

Configure the AI Manager for a town. Only provided fields are changed.

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `town` | usize | yes | Town index (must have an AI player) |
| `active` | bool | no | Enable/disable AI Manager |
| `build_enabled` | bool | no | Allow AI to place buildings |
| `upgrade_enabled` | bool | no | Allow AI to buy upgrades |
| `personality` | string | no | "Aggressive", "Balanced", or "Economic" |
| `road_style` | string | no | "None", "Cardinal", "Grid4", or "Grid5" |

```bash
# Enable AI Manager with Aggressive personality
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/ai_manager","params":{"town":1,"active":true,"personality":"Aggressive"},"id":1}'
```

### endless/chat

Send a chat message to another town. Messages appear in the recipient's `inbox` field in the next summary response and are logged to the combat log.

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `town` | usize | yes | Sender town index (must be LLM-controlled) |
| `to` | usize | yes | Recipient town index |
| `message` | string | yes | Chat message text |

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/chat","params":{"town":1,"to":0,"message":"lets team up"},"id":1}'
```

The player can also send messages to LLM towns via the chat input in the combat log UI.

### endless/perf

Get performance metrics — FPS, frame time, UPS, NPC/entity counts. Includes per-system timings when profiling is enabled.

```bash
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/perf","id":1}'
```

Returns: `fps`, `frame_ms`, `ups`, `npc_count`, `entity_count`, and optionally `timings` (BTreeMap of system name → ms).

### endless/debug

Deep-inspect entities by EntityUid or resources by kind+index. Returns full data in TOON format.

**UID mode** (auto-detects NPC vs building):

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `uid` | u64 | yes | EntityUid value |

**Kind+index mode** (resource-based lookups):

| Param | Type | Required | Description |
|-------|------|----------|-------------|
| `kind` | string | yes | "squad", "town", or "policy" |
| `index` | usize | yes | Index into the resource array |

**NPC returns:** uid, slot, job, activity, combat_state, hp, max_hp, energy, home, faction, town, personality traits, equipment slots (with rarity/bonus), flags, manual_target, squad, patrol, carried loot, cached stats, kill/death counts.

**Building returns:** uid, slot, kind, label, town, faction, grid position, hp, max_hp, occupants, growth, under_construction, respawn_timer, worksite info, wall level, assigned mine.

**Squad returns:** squad_index, members (with uid/name/job/activity/hp/energy), target, patrol_enabled, rest_when_tired, wave settings, owner, hold_fire.

**Town returns:** town_index, name, faction, center, area_level, food, gold, npcs (job counts), buildings (kind counts), squads, policy, faction_stats.

**Policy returns:** town_index, town_name, all policy fields.

```bash
# By UID (auto-detects NPC vs building)
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/debug","params":{"uid":450},"id":1}'

# By kind+index
curl -s -X POST http://localhost:15702 -H "Content-Type: application/json" \
  -d '{"jsonrpc":"2.0","method":"endless/debug","params":{"kind":"squad","index":0},"id":1}'
```

## Notes

- Large queries (50K NPCs) return megabytes of JSON — use `has`/`filter` to narrow results
- BRP runs on a background thread — zero game performance impact unless actively queried
- `world.insert_components` and `world.mutate_resources` can live-modify game state (powerful for debugging)
- Custom endpoints (`endless/*`) use queue resources for writes needing SystemParams (build, upgrade) and direct `resource_mut` for simple mutations (policy, time, squad_target, ai_manager)
- `endless/perf` returns FPS/UPS/NPC counts (no params needed), optionally includes per-system timings when profiling is enabled
- Port 15702 is Bevy's default, hardcoded in `RemoteHttpPlugin::default()`
