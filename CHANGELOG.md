# Changelog

## 2026-02-21n

- **settle on land** — migration town creation now uses verified `settle_target` (guaranteed land cell from `pick_settle_site()`) instead of NPC centroid `avg_pos` (which could be over water) for all placement: `create_ai_town`, `place_buildings`, `stamp_dirt`, NPC `Home`, combat log location
- **migration wipeout respawn** — if all NPCs in a migration group die before settling, the migration is cleared and a replacement `PendingAiSpawn` is queued with 4h delay (same `ENDLESS_RESPAWN_DELAY_HOURS` used by defeated-town respawn); replacement inherits original strength/resources; previously this case left `migration_state.active` permanently stuck
- **farmer dynamic farm scanning** — farmers now dynamically scan same-faction farms each work decision instead of requiring a pre-assigned `WorkPosition`; priority: ready farms > unoccupied growing (closest within tier); mirrors miner dynamic mine selection
- **BuildingSpatialGrid init** — `setup_world` now calls `bgrid.rebuild()` before initial NPC spawn so `BuildingSpatialGrid` lookups work from frame 1

## 2026-02-21m

- **extras atlas consolidation** — merged 3 individual sprite textures (heal, sleep, arrow) + new boat into 1 horizontal grid atlas (4×32px cells) via `build_extras_atlas()`; reduced texture bindings from 6 pairs to 4 pairs in the render pipeline; `extras_cols` camera uniform replaces `textureDimensions()` in shader (avoids VERTEX visibility requirement on texture binding); shader `calc_uv` extras branch selects column by atlas_id
- **boat migration** — migration groups now spawn a boat entity at the map edge that sails toward `settle_target` at `BOAT_SPEED` (150px/s); boat disembarks NPCs on land; `pick_settle_site()` selects settlement position farthest from all existing towns; `MigrationGroup` restructured with `boat_slot`, `boat_pos`, `settle_target`, `faction` fields; `RAIDER_SETTLE_RADIUS` reduced from 3000→500 (distance to settle_target, not any town); entity despawn guard via `commands.get_entity()` for stale npc_map entries; SETTLE count==0 guard only cancels when `member_slots.is_empty()`
- **DC group inspector** — when DirectControl (box-selected) units exist but no single NPC is selected, the bottom panel inspector shows a group summary: unit count, total/average HP, job breakdown; inspector now opens when `dc_count > 0`
- **endless mode test extended** — test expanded from 8 to 16 phases; phases 1-8 test builder AI town destruction, phase 9 waits 1 game hour, phases 10-16 test raider AI town destruction; validates both AI kinds go through the full fountain destroy → spawn queued → boat migration → settle pipeline; uses `WorldGenStyle::Continents` for water

## 2026-02-21l

- **fix migration stall** — `migration_spawn_system` now counts only alive raider towns (`is_alive(t.center)`) when checking if new migrations are needed; defeated towns with tombstoned centers no longer block new raider spawns

## 2026-02-21k

- **ground-move crosshair** — `ManualTarget::Position` now draws the same green crosshair as Npc/Building targets in DirectControl mode

## 2026-02-21j

- **main menu redesign** — replaced flat layout with sectioned UI (World / Difficulty / Options / Debug Options) using separators + bold section labels; difficulty presets (Easy/Normal/Hard) now control endless mode and replacement strength in addition to NPC counts; "Per Town (player & AI)" sub-group makes it clear that farms, gold mines, and NPC homes apply to both player and AI builder towns; renamed "AI Towns" → "AI Builder Towns" and "Raider Towns" → "AI Raider Towns" for consistent terminology; added `.on_hover_text()` tooltips to all 15+ sliders/controls; Debug Options collapsed by default; `DifficultyPreset` extended with `endless_mode`/`endless_strength` fields; `WorldGenStyle` default changed from Classic → Continents

## 2026-02-21i

- **directcontrol right-click rework** — right-click now has two modes: in `placing_target` mode (hotkey 1-0 or "Set Target" button) right-click sets `squad.target` for the whole squad; in default mode right-click only commands DirectControl (box-selected) NPCs with direct GPU `SetTarget` writes. `ManualTarget` converted from `struct(usize)` to `enum { Npc, Building, Position }` — single source of truth for all DirectControl targets. green bracket overlay now driven by `DirectControl` component query instead of `squad.members`. crosshair overlay queries per-NPC `ManualTarget::Npc`/`Building` on DirectControl entities. removed dead `AttackTarget` enum and `squad.attack_target` field. inspector Copy Debug Info includes DirectControl status + squad details.

## 2026-02-21h

- **road attraction + collision bypass** — GPU compute shader steers off-road NPCs toward nearby roads via 4-cardinal-ray × 3-tile gradient sampling of `tile_flags` buffer; inverse-distance gradient → lateral-only pull at 35% speed; disabled when on-road or within 96px of destination; NPCs on road tiles skip separation force against other road NPCs for smooth traffic flow; pre-computed `my_on_road` bool reused by speed bonus, collision bypass, and attraction

## 2026-02-21g

- **road multibuild** — click-drag to place road lines using existing drag infrastructure (`drag_start_slot`/`drag_current_slot`, `slots_on_line` Bresenham, `BuildGhostTrail`); ghost trail preview shows green/red affordability per cell; reuses `place_wilderness_building` for each cell on the line

## 2026-02-21f

- **build menu tooltips** — added `tooltip: &'static str` field to `BuildingDef` in BUILDING_REGISTRY; each buildable building has detailed hover tooltip with stats (HP, damage, range, cooldown, growth rates, costs) explaining why a player should build it; wired via `on_hover_text(def.tooltip)` in build_menu.rs

## 2026-02-21e

- **DRY spawn: materialize_npc()** — extracted shared `materialize_npc()` helper in `spawn.rs` that both `spawn_npc_system` (fresh spawn) and `spawn_npcs_from_save` (save-load) call; `NpcSpawnOverrides` struct carries optional saved state (health, energy, activity, personality, name, level, equipment, squad); eliminates ~80 lines of duplicated entity creation, GPU init, and tracking registration from `save.rs`; `FactionStats.inc_alive()` stays spawn-only (save restores faction stats from file)

## 2026-02-21d

- **kill "camp" — unify under "town"** — removed `BuildingKind::Camp` (merged into `Fountain`; `sprite_type` distinguishes rendering), collapsed ~15 `Fountain | Camp` match arms across the codebase, extracted `create_ai_town()` DRY helper in economy.rs (eliminates ~40 duplicated lines between migration and endless replacement), renamed all camp terminology: `CampState` → `RaiderState`, `SpawnBehavior::CampRaider` → `Raider`, `is_camp_unit` → `is_raider_unit`, `camp_buildable` → `raider_buildable`, all `CAMP_*` constants → `RAIDER_*`, `raider_camps` config → `raider_towns`, `camp_forage_system` → `raider_forage_system`; fountain towers now fire for all alive town centers (not just sprite_type==0); save backward compat via `LegacyBuilding::Camp` serde variant and `#[serde(alias)]` on renamed fields

## 2026-02-21c

- **MVP roads** — `BuildingKind::Road` (BUILDING_REGISTRY[12]) as player-buildable wilderness building with 1.5× NPC speed bonus via GPU compute; `tile_flags` buffer (binding 18) stores per-world-grid-cell bitfield with terrain bits 0-4 (Grass/Forest/Water/Rock/Dirt) and building bits 5+ (Road=32); `populate_tile_flags` system rebuilds from WorldGrid biome + buildings on dirty flag; `place_waypoint_at_world_pos` generalized to `place_wilderness_building(kind)` handling both Road and Waypoint; building atlas layer count now dynamic via `camera.bldg_layers` (from `BUILDING_REGISTRY.len()`) instead of hardcoded `BLDG_LAYERS` constant — prevents sprite overlap when adding new building types

## 2026-02-21b

- **farmer delivery (visible food transport)** — farmers now physically carry food home after harvesting instead of instantly crediting storage; `harvest()` simplified from dual-path (instant credit vs theft-only) to single DRY path that resets growth and returns yield; all 5 harvest callers (farmers, raiders, miners) use the same carry-home pattern via `Activity::Returning`; farmers show food sprite while carrying (layer 3 in GPU visual pipeline); on delivery, farmers return to `GoingToWork` instead of Idle for continuous work→carry→deliver cycle

## 2026-02-21a

- **endless mode** — toggle in main menu enables replacement of defeated AI towns; when enemy fountain/camp HP reaches 0, AI brain deactivates (NPCs + buildings persist as leaderless remnant); with endless mode on, a new AI migrates from the map edge after 4 game-hours, scaled to player's upgrade levels × configurable strength fraction (25%–150%, default 75%); new AI gets random personality (Aggressive/Balanced/Economic) and matching kind (Raider/Builder based on defeated town type); reuses existing migration system for edge spawn → walk → settle → place_buildings flow
- **destructible enemy town centers** — enemy fountains and camps can now be targeted and destroyed by player NPCs and AI squads; player's own fountain remains protected; `place_buildings` consolidated from two separate functions into one unified `place_buildings(..., is_camp: bool)`
- **save/load endless state** — endless mode toggle, strength fraction, and pending AI spawns all persist across save/load

## 2026-02-18aa

- **projectile extract optimization** — `ProjBufferWrites.active_set` tracks live projectile indices incrementally (push on spawn, swap_remove on deactivate); `extract_proj_data` iterates only active slots instead of scanning 0..high_water_mark; eliminates O(50K) per-frame scan when most slots are dead
- **typed attack targets** — `Squad.attack_target` changed from `Option<Vec2>` to `Option<AttackTarget>` enum (`Npc(slot)` / `Building(pos)`); crosshair overlay follows NPC targets via GPU readback positions instead of static coordinates; squad UI displays target type (NPC slot# vs building position)
- **squad overlay cleanup** — removed arrow line from group centroid to target; target marker circles hidden during box-select to reduce visual clutter

## 2026-02-18z

- **direct unit micromanagement (RTS-style)** — box-select (click-drag rectangle) to select player military NPCs on the map; right-click ground to move selected squad, right-click enemy NPC to focus-fire (ManualTarget component overrides GPU auto-targeting), right-click enemy building to attack it; per-squad "hold fire" toggle (members only attack when given a manual target); green selection brackets on all selected squad members; green drag-rectangle overlay during box-select; ManualTarget auto-clears when target dies; ESC cancels box-select; integrates with existing squad system (box-select populates active squad's members, right-click sets squad target)

## 2026-02-18y

- **fix: AI desire-driven building scoring** — building need formulas changed from additive (constant floor + desire bonus) to multiplicative (desire × deficit); food_desire gates farm/house construction, military_desire gates barracks/crossbow/waypoint construction; when desire is 0 the corresponding building category scores 0 instead of maintaining a constant floor; fixes Economic AI building farms forever with 0% food desire and 65k food surplus

## 2026-02-18x

- **loot system fixes** — three bugs preventing player NPCs from keeping loot on kill: (1) squad sync catch-all was overwriting `Activity::Returning` with `Patrolling`, discarding loot; (2) `xp_grant_system` didn't clear `CombatState`, so flee/leash could wipe loot; (3) flee and leash paths replaced `Returning{loot}` with `Returning{loot: []}`. Fix: squad sync preserves Returning, xp_grant clears CombatState on loot (immediate disengage), flee/leash preserve existing loot.
- **building loot via method** — `BuildingDef::loot_drop()` derives loot from `cost / 2` as food; replaces hardcoded `cost / 2` in `building_damage_system`; buildings with cost 0 (Fountain, Bed, GoldMine) drop nothing
- **inspector shows carried loot** — gold-colored "Loot: N food/gold" line in NPC inspector when carrying loot home

## 2026-02-18w

- **fix: carried gold tint** — gold sprite on returning miners was grayscale-tinted instead of natural colors; added early-return branch in `npc_render.wgsl` fragment shader for carried items (`atlas_id >= 0.5` on equipment layers) that bypasses the grayscale-then-tint pipeline
- **delivery radius 150→50px** — NPCs must walk closer to home building before delivering food/gold; `DELIVERY_RADIUS` in `behavior.rs` reduced from 150px to 50px

## 2026-02-18v

- **ranged/melee upgrade split** — split `MILITARY_UPGRADES` into `MILITARY_RANGED_UPGRADES` (9 stats: HP, Attack, Detection Range, Attack Speed, Move Speed, Alert, Dodge, Arrow Speed, Arrow Range) and `MILITARY_MELEE_UPGRADES` (6 stats: no projectile/range stats); Archer and Crossbow share `MILITARY_RANGED_UPGRADES`, Fighter uses `MILITARY_MELEE_UPGRADES`; removed `CROSSBOW_UPGRADES` (Crossbow now shares ranged tree); renamed Range label to "Detection Range"; made Detection Range, Attack Speed, Arrow Speed, Arrow Range all root upgrades (no prerequisites)
- **collapsible upgrade persistence** — upgrade branches start collapsed by default; expand/collapse state persisted to `UserSettings.upgrade_expanded` via `CollapsingState` API; saved to settings.json on toggle
- **combat log priority split** — `CombatLog` now has separate `priority_entries` ring buffer (max 200) for Raid/Ai events, preventing high-frequency combat events from evicting strategic entries; `iter_all()` chains both buffers for display

## 2026-02-18u

- **registry-driven upgrade system** — replaced hardcoded `UpgradeType` enum (25 variants) and fixed `[u8; 25]` arrays with dynamic `UpgradeRegistry` built from `NPC_REGISTRY` at init via `LazyLock`; each `NpcDef` declares `upgrade_category: Option<&'static str>` and `upgrade_stats: &'static [UpgradeStatDef]`; NPCs sharing the same `upgrade_stats` array but with different category names get fully independent upgrade branches (e.g. Archer and Fighter both reference `MILITARY_UPGRADES` but track levels separately); `UpgradeStatKind` enum replaces positional indices with semantic stat lookups; `UPGRADES.stat_mult(levels, category, stat)` replaces all hardcoded `match job` arms in `resolve_combat_stats`; `TownUpgrades.levels` and `AutoUpgrade.flags` switched from fixed arrays to `Vec<Vec<u8>>`/`Vec<Vec<bool>>`; upgrade UI grouped into Economy/Military sections with collapsible per-NPC branches; AI upgrade weights use dynamic category/stat_kind lookups; adding a new NPC type now automatically generates its upgrade branch by setting `upgrade_category` and `upgrade_stats` on the `NpcDef`

## 2026-02-18t

- **grid building simplification** — replaced `Building` enum in `WorldCell.building` with `GridBuilding` type alias `(BuildingKind, u32)` tuple; `Building` enum removed entirely; `ai_player` build functions take `BuildingKind` directly instead of `Building` variants; `register_spawner` takes `BuildingKind`; `recalc_waypoint_patrol_order_clockwise` no longer needs `WorldGrid` param (patrol_order only lives in `WorldData` now); `LegacyBuilding` enum in save.rs handles backward-compatible deserialization of old save format
- **fix: pause menu blocking settings clicks** — dim overlay changed from `Sense::click()` to `Sense::hover()` with `Order::Background`, preventing it from consuming click events meant for settings widgets underneath
- **expanded copy debug info** — NPC debug copy now includes XP, traits, combat stats, equipment, attack type, squad, gold, town name, miner details (mine assignment, productivity, mode); building debug copy now includes faction, per-type details (farm growth/status, waypoint patrol order, fountain tower stats, camp food, mine status/occupancy)
- **github action default target** — changed from `all` to `windows`

## 2026-02-18s

- **NPC registry drives start menu** — main menu sliders for NPC home counts are now generated dynamically from `NPC_REGISTRY` instead of hardcoded Farmer/Archer/Raider; `NpcDef` gains `home_building`, `is_camp_unit`, `default_count` fields; `WorldGenConfig` replaces `farmers_per_town`/`archers_per_town`/`raiders_per_camp` with `npc_counts: BTreeMap<Job, usize>`; `GameConfig` similarly uses `npc_counts: BTreeMap<Job, i32>`; `UserSettings` adds `npc_counts: BTreeMap<String, usize>` with automatic migration from legacy fields; `Difficulty::presets()` returns `DifficultyPreset` struct with npc_counts map; `build_town`/`place_camp_buildings` iterate NPC_REGISTRY to place homes; adding a new NPC type now automatically gets a start menu slider, world gen support, and settings persistence

## 2026-02-18r

- **unified building storage** — replaced all separate building structs (`Farm`, `Bed`, `Waypoint`, `MinerHome`, `GoldMine`, `UnitHome`) with single `PlacedBuilding` struct; `WorldData.buildings: BTreeMap<BuildingKind, Vec<PlacedBuilding>>` replaces 6 separate vecs; `BuildingHpState` simplified to `towns: Vec<f32>` + `hps: BTreeMap<BuildingKind, Vec<f32>>`; legacy accessors (`farms()`, `beds()`, `waypoints()`, `miner_homes()`, `gold_mines()` + `_mut()` variants) preserved for call-site compatibility; type aliases maintain backward compat in type positions; `#[serde(default)]` on optional fields (`patrol_order`, `assigned_mine`, `manual_mine`) ensures old saves load cleanly; adding any new building type now requires only a `BuildingKind` variant + registry entry (no new struct, no new WorldData field, no new HP vec)

## 2026-02-18q

- **unified unit-home buildings** — replaced 5 identical structs (`FarmerHome`, `ArcherHome`, `CrossbowHome`, `FighterHome`, `Tent`) with single `UnitHome` struct; WorldData uses `BTreeMap<BuildingKind, Vec<UnitHome>>` dynamic storage; BuildingHpState uses `BTreeMap<BuildingKind, Vec<f32>>` with custom serde for save-format compatibility; Building enum collapses 5 variants into `Home { kind, town_idx }` with `BuildingSerde` proxy for backward-compatible saves; `BuildingDef` gains `is_unit_home`/`place`/`tombstone`/`find_index` fn pointers; `place_building`/`remove_building`/`find_building_data_index`/`alloc_building_slots`/`BuildingSpatialGrid::rebuild` all collapsed from per-kind match arms to single registry-driven loops; adding a new unit-home building now requires only a `BuildingKind` variant + registry entry

## 2026-02-18p

- **fix: fighter energy** — fighter `has_energy` changed from `false` to `true`; without `Energy` component, fighters were excluded from `decision_system` query entirely (requires `&mut Energy`), causing them to sit at spawn in `OnDuty` forever

## 2026-02-18o

- **spawner inspector NPC state** — building inspector spawner section now shows linked NPC's activity, combat state, squad, patrol route (yes/none), GPU position, and home; Copy Debug Info button includes all spawner NPC data for troubleshooting; `NpcStateQuery` extended with `Option<SquadId>` and `Option<PatrolRoute>`
- **fix: patrol route insertion** — `rebuild_patrol_routes_system` now inserts `PatrolRoute` for patrol units that spawned before waypoints existed; previously only updated existing routes, leaving fighters/archers without patrol capability if they spawned first
- **squad UI hides raiders** — squad recruit loop skips `Job::Raider` since players can't recruit enemy units

## 2026-02-18n

- **per-job squad recruitment** — squad UI recruit controls now show one row per military NPC type from `NPC_REGISTRY` with colored label and `+N` buttons; players can compose squads with specific unit types (archers, crossbows, fighters); member list shows job-colored names with job labels; `SquadParams.squad_guards` query gains `Job` component; adding a new military NPC type to the registry automatically adds it to squad UI
- **fighter patrol support** — fighters now patrol waypoints and respond to squad targets like archers/crossbows (`decision_system` `can_work` and work action branches updated); raiders remain squad-only

## 2026-02-18m

- **NpcDef ui_color** — `NpcDef` gains `ui_color: (u8, u8, u8)` field with hand-tuned UI text colors per job (distinct from GPU sprite `color`); roster row colors use `ui_color` directly instead of brighten-from-GPU-color math
- **registry-driven spawner inspector** — building inspector spawner section uses `def.spawner` + `npc_def(job).label` + `tileset_index(def.kind)` from registries instead of 6-variant hardcoded match; adding a new spawner building type only requires setting `spawner: Some(SpawnerDef { .. })` in `BUILDING_REGISTRY`

## 2026-02-18l

- **registry-driven roster** — roster tab job filter buttons and row colors now driven by `NPC_REGISTRY` loop instead of hardcoded per-job matches; military jobs listed first then civilian; colors derived from `NpcDef.color` with 30% brighten for UI readability

## 2026-02-18k

- **BuildingDef get_building fn pointer** — `BuildingDef` gains `get_building: fn(&WorldData, usize) -> Option<(Building, Vec2)>` that reconstructs the full Building variant + position from WorldData at index; `building_from_kind_index` in game_hud.rs and `resolve_building_pos` in ai_player.rs both collapse from 12-arm matches to one-liner registry delegations; `resolve_building_pos` reuses existing `pos_town` fn pointer (no new field needed)

## 2026-02-18j

- **registry-driven building save/load** — `BuildingDef` gains `save_key: Option<&'static str>`, `save_vec: fn(&WorldData) -> JsonValue`, `load_vec: fn(&mut WorldData, JsonValue)` fn pointers; all 12 registry entries carry save/load closures; SaveData replaces per-kind building fields (farms, beds, waypoints, etc.) with `#[serde(flatten)] building_data: HashMap<String, serde_json::Value>` that captures all building vecs by key; save function loops `BUILDING_REGISTRY` to serialize, load function loops to deserialize; 5 save-only structs deleted (TownSave, PosTownSave, WaypointSave, MinerHomeSave, BuildingHpSave); `BuildingHpState` gains `Serialize + Deserialize + Clone` for direct serialization; all world building structs (Farm, Bed, Waypoint, etc.) gain serde derives with `vec2_as_array` module for Vec2↔[f32;2] backwards-compatible format; GoldMine load_vec has fallback deserializer for old `[[x,y],...]` format

## 2026-02-18i

- **delete BuildingSave** — `Building` enum gains `Serialize`/`Deserialize` derives directly; `BuildingSave` shadow enum and its 12-arm `from_building`/`to_building` conversions deleted from save.rs; save format uses `Building` directly (backwards-compatible via `#[serde(alias = "GuardPost")]` on Waypoint)

## 2026-02-18h

- **fighter damage 1.5x** — fighter `base_damage` 15→22.5 (1.5x multiplier vs standard melee)
- **melee range 50px** — `CombatConfig` default melee range 150→50px, projectile speed 500→200; all melee units (fighters, raiders) now engage at close range
- **combat log location button** — `CombatLogEntry` gains `location: Option<Vec2>`; `CombatLog::push_at()` accepts explicit position; wave-started entries include target position; combat log UI renders clickable ">>" button that pans camera to the target
- **wave logs as Raid** — squad wave start/end events use `CombatEventKind::Raid` (orange) instead of `Ai` (purple); wave-started messages use `building_def(bk).label` instead of `{:?}` debug format
- **NpcDef label_plural** — `NPC_REGISTRY` entries gain `label_plural` field ("Farmers", "Archers", etc.) for data-driven UI display
- **DisplayCategory** — `BuildingDef` gains `display: DisplayCategory` enum (Hidden/Economy/Military) for factions tab column assignment
- **BuildingDef town_idx fn** — registry gains `town_idx: fn(&Building) -> u32` fn pointer; building inspector uses `(def.town_idx)(&building)` instead of `building_town_idx()` helper
- **registry-driven intel panel** — factions tab Economy/Military columns use `BUILDING_REGISTRY` loop with `DisplayCategory` filter and `label_plural` from `NPC_REGISTRY` instead of hardcoded building lists
- **registry-driven inspector** — building inspector uses `building_def(kind).label` and `(def.town_idx)(&building)` instead of `building_name()`/`building_town_idx()` helpers

## 2026-02-18g

- **BUILDING_REGISTRY fn pointers** — `BuildingDef` gains 6 fn pointer fields (`build`, `len`, `pos_town`, `count_for_town`, `hps`, `hps_mut`); all 12 registry entries carry closures that dispatch to the correct WorldData/BuildingHpState vec; `WorldData::building_pos_town()`, `building_len()`, `building_counts()` delegate to registry (no per-kind match); `BuildingHpState::hps()`/`hps_mut()`/`push_for()` delegate to registry; `TownBuildingCounts` struct removed, replaced by `HashMap<BuildingKind, usize>` via registry loop
- **registry-driven build menu** — `build_place_click_system` uses `building_def(kind).label` and `(building_def(kind).build)(town_idx)` instead of per-kind matches
- **registry-driven factions tab** — `AiSnapshot` replaces 14 per-kind fields with `npcs`/`buildings` HashMaps; snapshot builder uses `BUILDING_REGISTRY` loop for NPC counts
- **wave-based squad attacks** — `ai_squad_commander_system` uses gather→threshold→dispatch→retreat cycle instead of continuous retargeting; `Squad` gains `wave_active`, `wave_start_count`, `wave_min_start`, `wave_retreat_below_pct` fields; personality-driven thresholds (Aggressive 3/25%, Balanced 5/40%, Economic 8/60%)
- **raider squads** — raider camps use squad system instead of `RaidQueue`; single squad per camp targets nearest enemy farm; `RaidQueue` resource removed entirely
- **SquadUnit component** — unified `SquadUnit` marker replaces per-job `Archer` queries in `squad_cleanup_system` and `ai_squad_commander_system`; applied to all military NPCs (archers, crossbows, fighters, raiders) at spawn and load
- **squad sync all military** — `decision_system` squad sync block applies to any NPC with `SquadId` (not just `is_patrol_unit()`); covers raiders and fighters in squads

## 2026-02-18f

- **registry-driven building healing** — `healing_system` replaces 10 per-kind blocks with single `BUILDING_REGISTRY` loop; `BuildingHpState::hps_mut(kind)`/`hps(kind)` dispatch to the right HP vec by BuildingKind; `DirtyFlags.buildings_need_healing` skips iteration when no buildings are damaged (set by `building_damage_system`, cleared when all healed)
- **building_pos_town dispatch** — `WorldData::building_pos_town(kind, index)` single method replacing 12-arm match in `building_damage_system` and per-kind blocks in healing; returns `Option<(Vec2, u32)>` with tombstone filtering
- **registry-driven iter_damaged** — `BuildingHpState::iter_damaged()` replaces 11-chain `chain_buildings!` macro with `BUILDING_REGISTRY` loop; returns `Vec` instead of chained iterator

## 2026-02-18e

- **RenderFrameConfig** — consolidate 4→1 ExtractResourcePlugin: `NpcGpuData`, `ProjGpuData`, `NpcSpriteTexture`, `ReadbackHandles` absorbed into single `RenderFrameConfig` resource; all render-world systems read via `config.npc`, `config.proj`, `config.textures`, `config.readback`
- **TileSpec::External carries asset path** — `External(usize)` → `External(&'static str)` so `BUILDING_REGISTRY` is single source of truth for building sprite paths; `SpriteAssets` replaces 5 named texture fields with `external_textures: Vec<Handle<Image>>` loaded by iterating registry; `spawn_world_tilemap` and `build_menu.rs` derive external images from the vec; adding a new External building = one registry entry + drop PNG

## 2026-02-18d

- **fighter home building** — new `FighterHome` building type (cost 5 food, 150 HP) spawning `Job::Fighter` units that patrol waypoints via `FindNearestWaypoint` behavior; `is_patrol_unit: true` so fighters join the waypoint patrol loop like archers and crossbows
- **NPC registry** — single source of truth `NPC_REGISTRY` in constants.rs: 6 `NpcDef` entries define job, label, sprite, color, base stats, attack_override, classification flags, and spawn component flags; `npc_def(job)` lookup replaces scattered `SPRITE_*` constants and hardcoded stat fields; `Job` methods (`color()`, `label()`, `sprite()`, `is_patrol_unit()`, `is_military()`) delegate to registry
- **data-driven build menu** — `build_menu.rs` iterates `BUILDING_REGISTRY` with `player_buildable`/`camp_buildable` filters instead of hardcoded `PLAYER_BUILD_OPTIONS`/`CAMP_BUILD_OPTIONS` arrays; `init_sprite_cache` extracts textures from `TileSpec` automatically
- **fix inspector spawner kind** — building inspector used hardcoded ints (0-4) for spawner `building_kind` lookups; replaced with `tileset_index(BuildingKind::X)` for registry-order-independence
- **full save/load** — FighterHome buildings, HP state, and spawner entries persisted with `#[serde(default)]` backward compatibility

## 2026-02-18c

- **consolidate GPU extractions** — 8→4 ExtractResourcePlugins: absorbed NpcComputeParams into NpcGpuData (derives ShaderType, serves as both extraction and compute uniform), absorbed ProjComputeParams into ProjGpuData (same pattern), replaced GrowthStates + BuildingHpRender per-feature extractions with generic OverlayInstances resource (zero-clone Extract<Res<T>> → BuildingOverlayBuffers with RawBufferVec reuse)
- **fix building sprite UV** — BLDG_LAYERS shader constant 10→11 to match actual 11-tile building atlas; fixes building sprites rendering between two tiles

## 2026-02-18b

- **building registry** — single source of truth `BUILDING_REGISTRY` in constants.rs: 11 `BuildingDef` entries define kind, tile spec, HP, cost, label, spawner, placement mode, tower stats; `building_def(kind)`, `tileset_index(kind)`, `building_cost(kind)` replace scattered constants
- **Town → Fountain + Camp split** — `BuildingKind::Town` removed; `Fountain` (player town center, auto-shoots) and `Camp` (raider center) are separate registry entries with distinct properties
- **kill BuildKind** — redundant UI-only `BuildKind` enum eliminated; `BuildMenuContext` uses `Option<BuildingKind>` + `destroy_mode: bool` instead; `.to_building_kind()` bridge removed
- **registry-driven methods** — `Building::tileset_index()`, `spawner_kind()`, `is_tower()`, `BuildingHpState::max_hp()`, `is_population_spawner()`, `resolve_spawner_npc()` all delegate to `BUILDING_REGISTRY`; removed `TILESET_*`, `SPAWNER_*`, `*_HP` constants
- **SpawnBehavior enum** — `FindNearestFarm`, `FindNearestWaypoint`, `CampRaider`, `Miner` replace hardcoded spawner index checks in `resolve_spawner_npc()`

## 2026-02-18a

- **crossbow homes** — new building type (`CrossbowHome`, cost 8 food) spawning `Job::Crossbow` units; crossbowmen are premium ranged military with higher damage (25 vs archer 15), longer range (150 vs 100), faster projectiles (150 vs 100), slower cooldown (2.0s vs 1.5s)
- **crossbow upgrade branch** — 5 new `UpgradeType` variants (CrossbowHp, CrossbowAttack, CrossbowRange, CrossbowAttackSpeed, CrossbowMoveSpeed) with dedicated "Crossbow" branch in upgrade tree UI; `UPGRADE_COUNT` 20→25
- **crossbow AI** — `AiAction::BuildCrossbowHome` scored after 2+ archer homes established; crossbow homes included in territory, neighbor counts, squad targets, and fallback attack lists; 25-element upgrade weight arrays for all personalities
- **crossbow save/load** — full persistence: `CrossbowHome` buildings, `Job::Crossbow` NPCs (components, sprite, combat flags), `BuildingHpState.crossbow_homes`, 25-element upgrade/auto-upgrade arrays (backward compatible via `#[serde(default)]`)
- **DRY patrol helpers** — `Job::is_patrol_unit()` (Archer|Crossbow) and `Job::is_military()` (Archer|Crossbow|Raider|Fighter) replace scattered match arms across behavior.rs, spawn.rs, save.rs
- **CombatConfig.crossbow_attack** — crossbow-specific attack base stats stored separately from `BaseAttackType` enum; `resolve_combat_stats()` overrides `atk_base` for `Job::Crossbow` while keeping `BaseAttackType::Ranged`

## 2026-02-17m

- **fountain tower** — fountains auto-attack nearby enemies (range=400, damage=15, cooldown=1.5s, proj_speed=350); strong spawn defense that prevents early wipes
- **GPU tower targeting** — shader `npc_compute.wgsl` now checks `npc_flags` bit 1 (tower) to let buildings bypass speed==0 early-return and reach combat targeting; `allocate_building_slot` sets flags=3 for fountains via `Building::is_tower()`
- **turret → tower rename** — all turret naming standardized to tower: `TowerStats`, `TowerState`, `TowerKindState`, `FOUNTAIN_TOWER`, `fire_towers()`, `building_tower_system`
- **waypoint turret removal** — waypoints are no longer part of the tower system; removed `WAYPOINT_TURRET`, waypoint state sync, and save/load of waypoint attack state (backward compat preserved via `#[serde(default)]`)
- **click select SystemParam** — `render.rs` `click_to_select_system` refactored: 8 params → `ClickSelectParams` SystemParam bundle; dead NPC guard via `NpcEntityMap` check prevents inspecting recycled slots
- **inspector dead NPC guard** — `game_hud.rs` inspector falls back to building inspector or placeholder when selected NPC no longer exists in ECS

## 2026-02-17l

- **building turret system** — generalized `waypoint_attack_system` into `building_turret_system` with shared `fire_turrets()` helper; any building kind can now be a turret via `TurretStats` config
- **fountain turrets** — fountains (sprite_type == 0) auto-attack nearby enemies (range=300, damage=5, cooldown=2.5s); always-on, refreshed from sprite_type each tick so camps never fire
- **`TurretState` resource** — replaces `WaypointState`; holds per-kind `TurretKindState` (timers + attack_enabled vecs); waypoint behavior unchanged (default disabled, persisted in save)
- **`TurretStats` struct** — `constants.rs` consolidates 5 loose WAYPOINT_* consts into `WAYPOINT_TURRET` and `FOUNTAIN_TURRET` typed consts
- **AI squad commander improvements** — `SquadRole::Idle` for excess squads, `defense_share_pct` + `attack_split_weight` per personality for explicit defense/attack allocation; non-attack squads clear targets

## 2026-02-17k

- **crash handler** — custom panic hook in `main.rs` catches crashes, copies full report (backtrace + location + version) to clipboard via `arboard`, writes `crash.log` next to executable, and shows native Windows `MessageBoxW` error dialog
- **arrow upgrades** — two new upgrades: Arrow Speed (#16) and Arrow Range (#17), +8% per level, require Range Lv1; applied to Archer/Raider/Fighter projectile stats in `resolve_combat_stats()`; AI weight arrays expanded to 18
- **ranged rebalance** — base ranged stats reduced (range 300→100, speed 200→100, lifetime 3.0→1.5) to make arrow upgrades meaningful progression
- **inspector faction links** — NPC and building inspectors show clickable faction links that open Factions tab with that faction selected
- **factions squad commander view** — Factions tab now displays per-faction squad details: member count, target position (with jump button), patrol/rest state, and AI commander targeting info

## 2026-02-17j

- **AI squad commander** — aggressive AI towns now group archers into squads and dispatch them to attack enemy buildings via `ai_squad_commander_system`; uses same `Squad` struct and behavior code path as player squads
- **`SquadOwner` enum** — `Player` or `Town(usize)` on each squad; first 10 indices permanently player-reserved, AI squads appended after; `npc_matches_owner()` helper for owner-safe recruitment
- **per-squad command state** — `AiSquadCmdState` with independent cooldown + `BuildingRef` target identity (kind + index, validated alive each cycle); desynchronized init cooldowns prevent AI wave synchronization
- **personality-driven squad allocation** — Aggressive: 1 attack squad (100% archers, 15s retarget). Balanced: 2 squads (60% attack military-first, 40% reserve patrol, 25s retarget). Economic: 1 raiding party (25% archers targeting farms only, 40s retarget)
- **`find_nearest_enemy_building_filtered()`** — new variant in `world.rs` accepting `&[BuildingKind]` allowed set for personality-based target filtering with broad fallback
- **owner-safe squad cleanup** — `squad_cleanup_system` generalized to recruit per-owner via `TownId` matching instead of hardcoded player-only
- **UI isolation** — left panel and squad overlay filtered to `is_player()` squads; hotkeys 1-0 unchanged (player-reserved indices)
- **save/load** — `Squad.owner` persisted with `#[serde(default)]` for backward compatibility; AI squad indices rebuilt from ownership scan on load

## 2026-02-17i

- **DRY: `TownContext` per-tick bundle** — unified 6 loose locals (center, food, empty_count, has_slots, slot_fullness, cached_mines) into `TownContext` struct with `build()` constructor; `execute_action` signature reduced from 10 params to 6
- **type-safe mine access** — `TownContext.mines: Option<MineAnalysis>` is `Some` for Builder, `None` for Raider; builder-only arms (BuildWaypoint, BuildMinerHome) guard with `let Some(mines) = &ctx.mines else { return None }` — invalid state is unrepresentable
- **mine data single source of truth** — removed `all_gold_mines` from `AiTownSnapshot`; `MineAnalysis.all_positions` is now the only mine position source; `miner_toward_mine_score` takes `&[Vec2]` instead of `&AiTownSnapshot`
- **DRY: `NeighborCounts` + `count_neighbors()`** — extracted shared 3x3 adjacency traversal from `farm_slot_score`, `farmer_home_border_score`, `archer_fill_score` into single helper
- **DRY: `territory_building_sets!` macro** — single definition of the 4 building types that constitute owned territory; both `all_building_slots()` and `all_building_slots_from_world()` consume only macro output
- **DRY: mining radius constants** — replaced 5 occurrences of hardcoded `300.0`/`5000.0` with `DEFAULT_MINING_RADIUS`, `MINING_RADIUS_STEP`, `MAX_MINING_RADIUS`
- **`is_population_spawner()` helper** — `SpawnerEntry` method replaces raw `matches!(building_kind, 0|1|2|3)` in ai_player and left_panel
- **`try_build_miner_home()`** — separate build path for miner homes using `ctx.mines.all_positions` instead of snapshot fn pointer, avoiding `unwrap()` inside closures

## 2026-02-17h

- **bug fix: waypoint pruning full teardown** — `sync_town_perimeter_waypoints` now calls `destroy_building()` instead of `remove_building()`, fixing stale GPU slots and spawner leaks when waypoints are pruned
- **DRY: `is_alive()` sentinel helper** — replaced 42 occurrences of `pos.x > -9000.0` across 8 files with `world::is_alive(pos)`; only the definition references the magic number
- **DRY: `empty_slots()` unified scan** — single `world::empty_slots()` replaces 4 inline grid-walk copies; deleted `count_empty_slots`, `has_empty_slot`; `find_inner_slot` rewritten as 2-line `min_by_key`
- **DRY: `try_build_scored()` unified build arms** — collapsed 4 near-identical BuildFarm/BuildFarmerHome/BuildArcherHome/BuildMinerHome match arms (~60 lines → ~12)
- **DRY: `MineAnalysis` single-pass** — replaced `uncovered_mines()` + `find_mine_waypoint_pos()` with `analyze_mines()` computing all mine metrics in one traversal; precomputed result passed from scoring to execution phase
- **DRY: `build_and_pay()` now includes dirty flag** — folded `dirty.mark_building_changed()` into `build_and_pay()`, removed separate dirty calls from AI and player build paths
- **DRY: spawner constants** — replaced raw `building_kind == 0|1|2|3` with `SPAWNER_FARMER/ARCHER/TENT/MINER` constants
- **DRY: territory from snapshot** — `controlled_territory_slots` derives from `AiTownSnapshot` (union of 4 building sets) instead of re-scanning WorldData
- **DRY: waypoint spacing** — extracted `min_waypoint_spacing()` as single source of truth; `waypoint_spacing_ok()` is a one-liner wrapper
- **rename: `economic_*` → `balanced_*`** — `balanced_farm_ray_score` and `balanced_house_side_score` match the personality that uses them

## 2026-02-17g

- **waypoint building inspector** — waypoint inspector now shows patrol order, turret on/off status from `WaypointState`, and nearby archer name + level from spawner lookup; `BuildingInspectorData` extended with `WaypointState`
- **F9 load allocates building GPU slots** — `load_game_system` now clears and calls `allocate_all_building_slots` after applying save data, matching the menu load path
- **town positions snapped to grid** — world gen now snaps player, AI town, and camp center positions to grid cell centers so fountain sprites align with their grid cells

## 2026-02-17f

- **save version checking** — `SAVE_VERSION` bumped to 2; `farm_growth` now saves only farm entries (mines in `mine_growth`); `apply_save` version-gates v1 farm_growth interpretation (clips to farm_count); `read_save_from` logs migration from older versions; version changelog comment above constant
- **click-to-select skips building slots** — `click_to_select_system` filters out building GPU slots via `BuildingSlotMap::is_building()`, preventing accidental selection of invisible building proxies
- **pop count from PopulationStats** — top bar total population now uses `PopulationStats` sum instead of `SlotAllocator::alive()`, which includes building slots

## 2026-02-17e

- **buildings rendered via GPU instanced pipeline** — buildings moved from TilemapChunk layer to the NPC storage buffer render path; building atlas generated as 32x320 vertical strip texture (`build_building_atlas`); `allocate_building_slot` now sets real tileset indices (atlas_id=7) instead of hiding with col=-1; building visual data filled by fallback loop in `build_visual_upload`
- **explicit render pass ordering** — 5 deterministic sort keys replace single sort_key=0.5; `StorageDrawMode` enum with 3 shader-def variants (`MODE_BUILDING_BODY`, `MODE_NPC_BODY`, `MODE_NPC_OVERLAY`) via Bevy's `#ifdef` preprocessor; generic `DrawStoragePass<const BODY_ONLY: bool>` replaces `DrawNpcsStorage`; `CompareFunction::Always` eliminates depth-test ordering ambiguity
- **render order contract** — ORDER_BUILDING_BODY (0.2) < ORDER_BUILDING_OVERLAY (0.3) < ORDER_NPC_BODY (0.5) < ORDER_NPC_OVERLAY (0.6) < ORDER_PROJECTILES (1.0); `queue_phase_item` helper reduces queue boilerplate
- **terrain opaque** — terrain TilemapChunk changed from `AlphaMode2d::Blend` to `Opaque`; building TilemapChunk removed entirely (`BuildingChunk`, `sync_building_tilemap` deleted)
- **ATLAS_* constants** — `constants.rs` now has canonical atlas ID constants (ATLAS_CHAR through ATLAS_BUILDING); TILESET_* constants in `world.rs` map building variants to strip indices with compile-time assertions

## 2026-02-17d

- **fix archers attacking own waypoints** — GPU combat targeting scan now skips building slots (speed=0) for both combat targeting and threat assessment; CPU `attack_system` validates GPU targets via `NpcEntityMap` (rejects building proxy slots, stale dead slots), faction check (rejects same-faction/neutral from stale readback), and health check (rejects dead targets); defense-in-depth against transient GPU readback state

## 2026-02-17c

- **neutral faction (-1)** — `FACTION_NEUTRAL` constant; GPU compute and projectile shaders treat faction -1 as same-faction (never targeted, no friendly fire); gold mines assigned neutral faction instead of player faction 0
- **no combat while resting** — `attack_system` skips NPCs with `Activity::Resting` in addition to `Returning` and `GoingToRest`; prevents sleeping archers from firing

## 2026-02-17b

- **hybrid GPU buffer writes** — `extract_npc_data` now uses per-buffer dirty flags instead of single `dirty: bool`; GPU-authoritative buffers (positions/arrivals) use per-index sparse writes (~10-50 calls/frame), CPU-authoritative buffers (targets/speeds/factions/healths/flags) use single bulk `write_buffer` per dirty buffer; reduces wgpu staging allocation overhead
- **CPU/GPU default alignment** — `NpcGpuState` defaults now match GPU buffer initialization: positions=-9999 (tombstone sentinel), factions=-1 (no faction), healths=0; GPU compute buffers for positions and factions use `create_buffer_with_data` with matching sentinels; fixes archers attacking phantom faction-0 slots on bulk upload

## 2026-02-17a

- **buildings as GPU NPC slots** — buildings (farms, waypoints, homes, tents, mines, beds, towns) now occupy invisible NPC GPU slots for projectile collision; eliminates the CPU `BuildingSpatialGrid` collision loop in `process_proj_hits` and fixes the double-hit bug where projectiles damaged both NPCs and nearby buildings in the same frame
- **three-tier GPU compute optimization** — Mode 2 now has three tiers via `npc_flags` buffer (binding 17): buildings (speed=0) early exit, non-combatants (farmers/miners) scan only `threat_radius` (7×7 cells), combatants (archers/raiders/fighters) do full `combat_range` scan (9×9 cells); ~33% reduction in Mode 2 GPU work
- **MAX_NPC_COUNT 50K → 100K** — accommodates building slots alongside NPC slots; `NpcLogCache` changed to lazy init (`VecDeque::new()`) to avoid 464MB pre-allocation at 100K
- **BuildingSlotMap resource** — bidirectional HashMap mapping `(BuildingKind, index) ↔ NPC slot`; allocated at startup/load, freed on destroy; `WorldState` SystemParam extended with `slot_alloc` and `building_slots`
- **building GPU HP sync** — `building_damage_system` writes `SetHealth` to GPU after damage so projectile compute sees updated building HP

## 2026-02-16u

- **double-click fountain → factions tab** — double-clicking a fountain building opens the Factions tab pre-selected to that fountain's faction; `DoubleClickState` tracks last click time/position in `click_to_select_system`; `UiState.pending_faction_select` bridges render→UI
- **tutorial 10-minute auto-end** — tutorial auto-completes after 600s wall-clock time (`Time<Real>`); `TutorialState.start_time` set on init, checked each frame
- **combat log faction filter** — `CombatLogEntry` now carries `faction: i32` (-1=global, 0=player, 1+=AI); "All"/"Mine" dropdown in combat log filters entries by faction; persisted in `UserSettings.log_faction_filter`; all 14 `push()` call sites across 9 files updated with faction param
- **roadmap: projectile double-hit bug** — documented phantom building damage from NPC projectile hits (same slot checked twice per frame)

## 2026-02-16t

- **DRY: position-based building lookups** — `WorldData::miner_home_at()` and `gold_mine_at()` replace 7 inline `iter().position(|m| (m.position - pos).length() < 1.0)` calls across economy.rs, left_panel.rs, game_hud.rs
- **DRY: alive building counts** — `WorldData::building_counts(town_idx)` returns `TownBuildingCounts` struct; replaces identical 6-line counting blocks in ai_player.rs and left_panel.rs
- **DRY: dirty-flag cascades** — `DirtyFlags::mark_building_changed(kind)` replaces 6 scattered flag-setting blocks across ui/mod.rs (3×), combat.rs, ai_player.rs (2×)
- **DRY: uncovered mines** — `uncovered_mines()` shared helper in ai_player.rs replaces duplicated waypoint-filtering logic between `find_mine_waypoint_pos` and `count_uncovered_mines`
- **cleanup: unused param** — removed `_center` from `count_uncovered_mines`; fixed test indentation in friendly_fire_buildings.rs

## 2026-02-16s

- **stage 14d: auto-mining policy** — `MiningPolicy` resource with `mining_policy_system` (discovery within configurable radius, round-robin miner distribution across enabled mines, stale assignment clearing); Policies tab mining section (radius slider 0–5000px, per-mine enable/disable checkboxes, assigned miner counts, jump-to-mine); gold mine inspector auto-mining ON/OFF toggle; manual override preserved via `MinerHome.manual_mine`; dirty-flag gated (`DirtyFlags.mining`)
- **AI town snapshot cache** — `AiTownSnapshot` caches per-town building positions and empty slots; smart slot scoring heuristics (farm clustering via 2×2 block detection, farmer-home adjacency to farms, archer gap-filling, miner-toward-mine); `farmer_home_target()` personality method (Aggressive 1:1, Balanced farms+1, Economic 2× for shift coverage); `pick_best_empty_slot()` generic scorer with `find_inner_slot` fallback
- **dirty-flagged AI perimeter waypoint sync** — `sync_patrol_perimeter_system` prunes in-town waypoints that no longer sit on the territory perimeter after building changes; gated by `DirtyFlags.patrol_perimeter`; preserves wilderness/mine outpost waypoints
- **mine occupancy limits** — `MAX_MINE_OCCUPANCY` constant; behavior system skips full mines; HUD shows occupancy count on gold mine inspector
- **gold mine naming + policy mine list UX** — consistent "Gold Mine #N" naming via `gold_mine_name()` helper; policy mine list shows per-mine assigned miner count and distance
- **friendly-fire building regression test** — 4-phase test: ranged shooter fires through vertical wall of 10 friendly farms at enemy target; verifies target lock, projectile activity, NPC damage dealt, and zero friendly farm damage

## 2026-02-16r

- **fix: mine occupancy leak on miner death** — `death_cleanup_system` now releases `WorkPosition` occupancy when a miner dies mid-mining; previously the mine stayed permanently "occupied" causing tended growth without a living miner
- **fix: pop_dec_working for miners** — `MiningAtMine` now counted as working activity in death cleanup (was only `Working` for farmers)
- **miner NPC inspector: set mine** — clicking a miner shows the same "Set Mine"/"Clear" UI as the MinerHome building inspector; extracted shared `mine_assignment_ui()` helper (DRY)

## 2026-02-16q

- **miner home mine assignment** — click-to-assign UI on miner home inspector: "Set Mine" enters placement mode (like squad targets), click a gold mine to assign it; "Clear" reverts to auto (nearest mine); `MinerHome.assigned_mine` persisted in save/load via `MinerHomeSave`; behavior decision + spawn both respect assignment
- **fix: squad archers leaving post when rest_when_tired=false** — Priority 6 (OnDuty+tired) now checks squad `rest_when_tired` flag; archers in squads with the flag off stay on duty instead of cycling home
- **fix: gold mines now indestructible** — `building_damage_system` skips `GoldMine` kind
- **returning miners show gold sprite** — `build_visual_upload` item layer shows gold sprite for `Returning { gold > 0 }` (food sprite for `has_food`)

## 2026-02-16p

- **stage 14b: AI expansion + waypoint rename + wilderness placement** — AI expansion brain: dynamic miner targets per personality, slot fullness scaling for expansion urgency, boosted expansion weights; disabled turrets on waypoints (code preserved for future Tower building); full rename GuardPost→Waypoint across 35 files with serde back-compat aliases; `place_waypoint_at_world_pos()` for wilderness placement (player + AI); AI territorial strategy places waypoints near uncovered gold mines; `WAYPOINT_COVER_RADIUS` (200px) determines mine coverage

## 2026-02-16n

- **mining progress cycle** — miners now work a 4-game-hour cycle at the mine with a gold progress bar overhead (`MiningProgress` component, `MINE_WORK_HOURS=4.0`); bar fills left-to-right in gold color (atlas_id=6.0 shader path); when full, miner extracts `MINE_EXTRACT_PER_CYCLE` (5) gold scaled by GoldYield upgrade and returns home; tired miners extract partial gold proportional to progress; combat flee/leash properly cleans up mining state + occupancy

## 2026-02-16m

- **fix: black NPC sprite on guard posts** — `build_visual_upload` now resets `visual_data` to -1.0 sentinel each frame (matching `equip_data`); guard post NPC slots have no ECS entity so the query never overwrites them — previously they rendered as sprite (0,0) with black tint

## 2026-02-16l

- **fix: pause freezes GPU compute** — spacebar pause now sets delta=0 in `update_gpu_data` and `update_proj_gpu_data`, stopping NPC movement and projectile physics on the GPU; previously only ECS systems checked `game_time.paused` while the compute shader kept running
- cleanup: remove unused imports in behavior.rs, combat.rs, stats.rs

## 2026-02-16k

- **GpuReadState extraction deleted** — removed `ExtractResourcePlugin::<GpuReadState>` (nothing in render world read it); saves ~1.2MB/frame clone
- **ProjBufferWrites zero-clone** — removed `Clone`/`ExtractResource` from `ProjBufferWrites` and `ProjPositionState`; new `extract_proj_data` (ExtractSchedule) replaces both `write_proj_buffers` and `prepare_proj_buffers` using `Extract<Res<T>>` + `queue.write_buffer()`; shared `write_dirty_f32`/`write_dirty_i32` helpers DRY dirty-index writes across NPC and projectile extract functions; saves ~3.4MB/frame in clones

## 2026-02-16j

- **profiler debug actions** — "Spawn Migration Group" button in Profiler tab (Debug Actions collapsible); bypasses cooldown/population checks, disabled while migration active; MigrationState.debug_spawn flag consumed by migration_spawn_system
- **main menu reorder** — Farmer Homes and Archer Homes nested under AI Towns; Tents nested under Raider Camps; Farms and Gold Mines at top level
- **slider limits raised** — Farms 50→100, Farmer Homes/Archer Homes/Tents 50→1000

## 2026-02-16i

- **zero-clone GPU upload** — eliminated 6.4MB/frame `ExtractResource` clone of `NpcBufferWrites` by splitting into `NpcGpuState` (compute + sprite + flash, persistent) and `NpcVisualUpload` (packed visual + equip, rebuilt each frame); both read during Extract via `Extract<Res<T>>` (zero-clone immutable access) with `queue.write_buffer()` direct GPU upload; replaced `sync_visual_sprites` + `write_npc_buffers` + `prepare_npc_buffers` visual repack with `build_visual_upload` (single O(N) ECS→GPU-ready pack) + `extract_npc_data` (single Extract function for compute per-dirty-index + visual bulk writes); sentinel -1.0 initialization on first-frame NpcVisualBuffers creation; net ~0.75ms/frame savings at 20K NPCs

## 2026-02-16h

- **dynamic raider camp migration** — new raider camps spawn organically as the player grows; every 12 game hours, if player alive NPCs exceed VILLAGERS_PER_CAMP × (camp_count + 1), a group of 3 + player_alive/scaling raiders spawns at a random map edge and wanders toward the nearest player town using existing Home + Wander behavior; group size scales with difficulty (Easy=6, Normal=4, Hard=2 divisor); when within 3000px (~30s walk) of any town, the group settles: places camp center + tents via existing place_camp_buildings(), stamps dirt, registers tent spawners, activates AiPlayer; new AiPlayer.active field defers AI decisions until settlement; MigrationState resource persisted in save/load with Migrating component re-attached on load; max 20 dynamic camps; combat log announces approach direction and settlement

## 2026-02-16g

- **guided tutorial** — 20-step condition-driven tutorial system teaching camera, building, NPC interaction (click/follow), food, upgrades, mining, policies, patrols, squads, and hotkeys; action-triggered steps auto-advance when player completes the action, info-only steps require clicking Next; skippable per-step or entirely; completion persisted in UserSettings; restart button in main menu
- **difficulty presets** — Easy/Normal/Hard presets auto-set farms, farmers, archers, raiders, AI towns, raider camps, and gold mines; grouped under collapsible Difficulty header in main menu; sliders still manually adjustable after preset selection
- **building cost rebalance** — flat costs (no difficulty scaling): Farm 2, FarmerHome 4, MinerHome 4, ArcherHome 4, GuardPost 1, Tent 3
- **build area expanded** — base town grid increased from 7x7 to 8x8 (-4 to +3)
- **default worldgen** — changed to Continents (gen_style=1); removed "Your Towns" slider (hardcoded to 1)

## 2026-02-16f

- **jukebox track selection fix** — dropdown now uses `play_next: Option<usize>` field consumed by `jukebox_system` instead of setting `last_track` directly (which caused random track instead of selected)
- **jukebox speed controls** — ComboBox dropdown with 10-100% (10% steps) and 150-500% (50% steps); applies via `AudioSink::set_speed()` each frame; speed persisted in UserSettings (`music_speed` field, serde default 1.0)
- **GPU-native NPC rendering** — new instanced render pipeline replacing Bevy sprite entities with custom RenderCommand + Transparent2d phase; dual vertex buffers (quad + per-instance); multi-layer atlas support (character, equipment, status icons)

## 2026-02-16e

- **fix crop sprite surviving farm destruction** — destroyed farms no longer show floating food icon or regrow; `FarmStates::tombstone()` method resets all 3 parallel vecs (positions, states, progress); `remove_building()` calls it instead of inline resets; `farm_growth_system` skips tombstoned farms (`position.x < -9000`)
- **SelectedNpc default fix** — default changed from 0 to -1 (no selection) to prevent phantom selection on startup
- **HUD text contrast** — HP bar, energy bar, and squad overlay text now use black instead of white for readability
- **roadmap updates** — added GPU-native NPC rendering spec and new every-frame review items
- **README status update** — "Early development" → "Active development" with accurate feature summary

## 2026-02-16d

- **squad cleanup dirty-flag gated** — `squad_cleanup_system` now skips when `DirtyFlags.squads` is false; flag set by death_cleanup (any death), spawn_npc (archer spawn), left_panel UI (assign/dismiss), and save load (DirtyFlags::default); eliminates per-frame squad iteration on idle frames
- **inspector overhaul** — shows combat stats (dmg/range/cooldown/speed), equipment (weapon/helmet/armor), attack type, squad assignment, starving status, carried gold, faction + home inline; window height 160→280px
- **load game window** — save picker moved from collapsing header to a centered egui::Window with close button
- **DirtyFlags lifecycle fix** — all load/startup/cleanup paths now reset via `DirtyFlags::default()` instead of setting individual flags; `game_cleanup_system` also clears `HealingZoneCache`

## 2026-02-16c

- **jukebox UI** — top-right overlay with track picker dropdown (ComboBox), pause/play, skip, loop toggle; dark semi-transparent background frame; FPS counter moved from standalone overlay into top bar right section
- **faction kills fix** — `xp_grant_system` now calls `FactionStats.inc_kills()` for the killer's faction (was never called, kills stuck at 0)
- **Intel → Factions rename** — `LeftPanelTab::Intel` → `Factions`, `IntelParams` → `FactionsParams`, `IntelCache` → `FactionsCache`, `intel_content()` → `factions_content()`, keyboard shortcut I unchanged
- **save file picker** — main menu "Load Game" collapsible section lists all `*.json` saves from save directory sorted newest first; shows filename + relative age ("3m ago"); click to load; `list_saves()`, `read_save_from()`, `SaveLoadRequest.load_path`

## 2026-02-16b

- **music jukebox** — 22-track soundtrack (Not Jam Music Pack, CC0) with random no-repeat playback; `GameAudio` resource + `MusicTrack` marker; `load_music` at Startup, `start_music` on OnEnter(Playing), `stop_music` on OnExit(Playing), `jukebox_system` auto-advances when track despawns; music/SFX volume sliders in pause menu Settings (persisted in UserSettings v3); `PlaySfxMsg` + `SfxKind` scaffold for future SFX; `play_sfx_system` drains messages (no .ogg files wired yet)
- **Help tab (H)** — new left panel tab with collapsible sections: Getting Started, Core Gameplay Loop, Economy, Military & Defense, Building, Controls, Tips; `LeftPanelTab::Help` variant; H key toggle; top bar Help button; `tab_help` help catalog entry
- **README update** — download links point to v0.1.1 direct .zip URLs per platform; controls table adds F5/F9/L; credits add Not Jam Music Pack

## 2026-02-16

- **guard post GPU targeting (Option D)** — guard posts now get real `SlotAllocator` NPC indices so the GPU spatial grid auto-populates `combat_targets[gp_slot]` with the nearest enemy. `sync_guard_post_slots` (dirty-flag gated via `DirtyFlags.guard_post_slots`) allocates/frees slots on build/destroy/load. `guard_post_attack_system` reads one array index per post — O(1) instead of scanning all NPCs. At 20K NPCs + 10K turrets: ~2.75ms CPU eliminated, ~0.3ms more GPU (parallel, hidden). Guard post slots use sprite_col=-1 (invisible to NPC renderer) and health=999 (immortal in GPU). Tombstoned posts auto-free their slots.
## 2026-02-15g

- **autosave system** — `autosave_system` triggers every N game-hours (default 12, configurable 0-48 on main menu); writes to 3 rotating files (`autosave_1.json`, `autosave_2.json`, `autosave_3.json`); `SaveLoadRequest` tracks interval/slot/last-hour; `UserSettings.autosave_hours` persisted; main menu "Autosave" slider between Difficulty and Play button; 0 = disabled

## 2026-02-15f

- **dirty flag consolidation** — replaced 4 separate dirty-flag types (`BuildingGridDirty`, `PatrolsDirty`, `SpatialDirtyFlags` SystemParam, `HealingZoneCache.dirty`) with single `DirtyFlags` resource (`building_grid`, `patrols`, `healing_zones`, `patrol_swap`); all default `true` so first frame always rebuilds; `rebuild_building_grid_system` now gated on `DirtyFlags.building_grid` (skips 99%+ of frames); `pending_swap` payload moved from deleted `PatrolsDirty` into `DirtyFlags.patrol_swap`; touches 11 files, pure refactor — no behavioral changes

## 2026-02-15e

- **GPU threat assessment** — move NPC threat counting (enemy/ally within 200px) from CPU O(N) linear scan to GPU spatial grid query; piggybacks on existing Mode 2 combat targeting neighbor loop in `npc_compute.wgsl`; packs `(enemies << 16 | allies)` into a single u32 per NPC, readback via `GpuReadState.threat_counts`; `decision_system` unpacks for flee threshold calculation; eliminates `count_nearby_factions()` CPU function; adds `threat_radius` param to `NpcComputeParams`; binding 16 on compute shader
- **save/load: load from main menu** — "Load Game" button on main menu (grayed if no save); `game_load_system` runs before `game_startup_system` via `.chain()`; skips world gen if save was loaded; centers camera on first town after load
- **vertical-slice test hardened** — adds WorldGrid init, BuildingHpState entries, spawner buildings (FarmerHome/ArcherHome/Tent), SpawnerEntry registration for all NPCs; extends timeout 60→90s for respawn phase; adds `building_hp` to test cleanup
- **patrol route fix** — `build_patrol_route` now filters out destroyed guard posts (position.x > -9000)

## 2026-02-15d

- **save/load system (Stage 18 MVP)** — F5 quicksave / F9 quickload with JSON serialization to `Documents/Endless/saves/quicksave.json`; saves full game state: WorldGrid terrain+buildings, WorldData (towns/farms/beds/guard posts/homes), GameTime, FoodStorage, GoldStorage, FarmStates, MineStates, SpawnerState, BuildingHpState, TownUpgrades, TownPolicies, AutoUpgrade, SquadState, GuardPostState, CampState, FactionStats, KillStats, AiPlayerState, and all live NPC data (position, health, energy, activity, combat state, personality, equipment, squad); load despawns all entities, rebuilds resources from save, spawns NPCs with saved state, triggers tilemap + spatial grid + patrol route rebuild; toast notification with fade-out on save/load; save version field + `#[serde(default)]` for forward compatibility; SystemParam bundles keep systems under Bevy's 16-parameter limit

## 2026-02-15c

- **difficulty system + building cost rebalance** — `Difficulty` enum (Easy/Normal/Hard) selectable on main menu, persisted in settings; `building_cost(kind, difficulty)` replaces 6 hardcoded `*_BUILD_COST = 1` constants with differentiated base costs (Normal: Farm=3, FarmerHome=5, MinerHome=5, ArcherHome=8, GuardPost=10, Tent=3); Easy≈half, Hard=double; player build menu, click-to-place, and AI player all use `building_cost()`
- **roadmap update** — marked food consumption, starvation effects, and building cost rebalance as complete in Stage 14

## 2026-02-15b

- **building damage now projectile-based** — `attack_system` no longer sends direct `BuildingDamageMsg` on fire; instead `process_proj_hits` checks active projectile positions against `BuildingSpatialGrid` (20px hit radius) and sends `BuildingDamageMsg` on collision; buildings now take damage from actual projectile hits, not instantly when fired
- **building HP bars render properly** — fragment shader now renders 3-color health bars (green/yellow/red) in bottom 15% of building quads for atlas_id≥4.5; previously discarded all pixels for bar-only mode
- **main menu reorganization** — moved AI Think, NPC Think, and Raider Passive Forage sliders from main area to Advanced collapsible section; cleaner default menu
- **FPS counter style** — changed from gray semi-transparent to black bold for readability
- **default towns 2→1** — new games start with 1 player town instead of 2
- **roadmap cleanup** — 1443→277 lines; moved 267 completed items to `docs/completed.md`; extracted 4 specs to `docs/specs/`; deleted done specs (AI Players, Continent WorldGen); collapsed done stages; renamed Godot parity→Backlog: UI & UX; moved game design tables to `docs/concepts.md`; fixed Guard→Archer terminology throughout docs

## 2026-02-15

- **building HP bars** — damaged buildings now display GPU-instanced health bars using atlas_id=5.0 bar-only mode (shader discards sprite, keeps bar); `BuildingHpRender` resource extracted to render world; all building types now have HP (Town=500, GoldMine=200, Bed=50); `Building::kind()` returns `BuildingKind` (no longer `Option`) — Fountain/Camp map to `Town`, Bed added as new variant
- **trait display from Personality** — inspector reads traits from `Personality` component via `trait_summary()` instead of cached `trait_id` in `NpcMetaCache`; `TraitKind::name()` method added
- **NPC rename in inspector** — text field + Rename button (or Enter) edits `NpcMetaCache.name` directly; `InspectorRenameState` local tracks active rename slot
- **BuildingSpatialGrid includes Beds** — beds now in spatial grid; `find_nearest_enemy_building()` skips Bed + Town + GoldMine (non-targetable)
- **SquadParams loses meta_cache** — `squads_content` takes `&NpcMetaCache` from roster params instead of bundling its own copy
- **RespawnTimers removed** — stale legacy resource deleted; `SpawnerState` is sole authority for respawn timing
- **fix: rebuild_patrol_routes performance** — replaced `WorldData.is_changed()` trigger (fired every frame when Patrols tab open due to `ResMut` DerefMut leak) with explicit `PatrolsDirty` resource set only on guard post build/destroy/reorder; added per-town route cache (O(towns) instead of O(archers)); merged `RosterRenameState` into `RosterState` to stay under Bevy's 16-param limit after adding `PatrolsDirty` to `left_panel_system`; `left_panel_system` now takes `Res<WorldData>` instead of `ResMut`
- **building HP & NPC building attacks** — all buildings now have HP (GuardPost=200, ArcherHome=150, FarmerHome/MinerHome/Tent=100, Farm=80); archers and raiders opportunistically fire at enemy buildings when no NPC target is in range; raiders only target military buildings (ArcherHome, GuardPost); buildings destroyed at HP≤0 with linked NPC killed; `BuildingHpState` resource with parallel Vecs; `BuildingDamageMsg` direct-on-fire message; `BuildingSpatialGrid` extended with all building types + faction field; `building_damage_system` processes damage and calls shared `destroy_building()` helper
- **DRY: destroy_building() consolidation** — extracted shared `destroy_building()` in world.rs that handles grid clear + WorldData tombstone + spawner tombstone + HP zero + combat log; replaces duplicated destroy paths in click-destroy and inspector-destroy; also used by building_damage_system for HP→0 destruction
- **AiBuildRes SystemParam bundle** — `ai_decision_system` hit Bevy's 16-param limit when adding BuildingHpState; bundled 8 mutable world resources into `AiBuildRes<'w>` SystemParam struct (same pattern as CleanupWorld); reduces param count from 17→11
- **fix: squad rest-when-tired** — squad archers now properly go home to rest when energy is low; three interacting bugs fixed: (1) arrival handler catches tired archers before entering OnDuty (prevents Patrolling↔OnDuty oscillation), (2) hard gate with hysteresis before combat priorities forces GoingToRest (enter at energy < 30, stay until ≥ 90), (3) squad sync block only writes GPU targets when needed instead of every frame (OnDuty archers only redirected when squad target moves >100px); `attack_system` now skips GoingToRest NPCs to prevent GPU target override
- **upgrade tree restructure (14→16)** — renamed Archer-specific upgrades to Military (applies to Archer + Raider + Fighter); added per-job upgrades: FarmerMoveSpeed, MinerHp, MinerMoveSpeed, GoldYield; removed ArcherSize and FoodEfficiency; categories: Military (7), Farmer (3), Miner (3), Town (3); `UPGRADE_RENDER_ORDER` defines tree UI layout with indentation depth; `upgrade_effect_summary()` shows current/next effect in UI; `branch_total()` per-category totals; `expansion_cost()` custom slot-based pricing for TownArea; Dodge now requires MoveSpeed Lv5 (was AlertRadius Lv1)
- **gold yield upgrade** — miners extract more gold per cycle with GoldYield upgrade (+15% per level); `decision_system` mining extraction reads `TownUpgrades`
- **upgrade UI tree rendering** — left panel Upgrades tab now renders upgrades in tree order with branch headers, indentation, branch totals, and effect summaries (now → next); auto-upgrade checkbox persists immediately to settings
- **settings version migration** — settings v1→v2 no longer resets all settings; outdated versions get new fields filled with defaults; added `auto_upgrades` persistence for per-upgrade auto-buy flags
- **AI upgrade weights updated** — `ai_player.rs` upgrade weights expanded to 16 entries matching new upgrade tree; raiders now score Military HP/Attack/AttackSpeed; builders score Miner and Farmer upgrades by personality
- **town build grid symmetry fix** — base build bounds changed from `-2..+3` to `-3..+3` (7x7) so towns/camps now have exactly 3 build slots outward in each direction from the center fountain/camp; updated related `TownGrid` docs/comments
- **selection indicator unification (world-space)** — added corner-bracket selection overlay for clicked NPCs and clicked buildings (no build-menu highlight); NPC uses smaller bracket size, building uses larger `WorldGrid`-scaled bracket; removed old NPC circle selection stroke for consistent visual language
- **build toggle polish** — removed up/down arrow text from build button and adjusted bottom offset so the closed button sits flush with the UI edge
- **tech tree prereqs + multi-resource costs (stage 19 chunk 1)** — `UPGRADE_REGISTRY` in `stats.rs` extended with `prereqs: &[UpgradePrereq]` and `cost: &[(ResourceKind, i32)]` per node; `ResourceKind { Food, Gold }` enum (extensible for Stage 23 wood/stone/iron); tree structure: Archer branch (ArcherHealth→ArcherAttack→AttackSpeed→ArcherRange→AlertRadius, ArcherHealth→MoveSpeed), Economy branch (FarmYield→FarmerHp, FarmYield→FoodEfficiency), Town branch (HealingRate→FountainRadius, FountainRadius+FoodEfficiency→TownArea); shared helpers `upgrade_unlocked()`, `upgrade_available()`, `deduct_upgrade_cost()`, `missing_prereqs()`, `format_upgrade_cost()` used by all 4 systems (process_upgrades, auto_upgrade, AI, UI); `TownUpgrades::town_levels()` DRY accessor; gold-cost nodes show "10g", mixed-cost show "10+10g"; locked nodes dimmed with prereq tooltip; auto-upgrade + AI skip locked nodes
- **upgrade metadata registry (DRY)** — `UpgradeNode` struct centralizes label/short/tooltip/category (previously duplicated in `left_panel.rs` UPGRADES array + UPGRADE_SHORT array + `ai_player.rs` match block); all consumers read from single `UPGRADE_REGISTRY` const array in `stats.rs`
- **rename: Guard→Archer, House→FarmerHome, Barracks→ArcherHome, MineShaft→MinerHome** — full codebase rename of NPC types and building names to reflect 1:1 building→NPC relationships; `Job::Guard` → `Job::Archer`, `Guard` marker → `Archer`; `Building::House/Barracks/MineShaft` → `Building::FarmerHome/ArcherHome/MinerHome`; `WorldData.houses/barracks/mine_shafts` → `farmer_homes/archer_homes/miner_homes`; `BuildKind` variants renamed; `PolicySet` fields renamed with `#[serde(alias)]` for backwards compat; `UpgradeType::Guard*` → `UpgradeType::Archer*`; `UserSettings.guards` → `archers` with serde alias; all UI labels, combat log messages, test names updated; `guard_patrol.rs` → `archer_patrol.rs`; `GuardPost` intentionally kept (it's the building, not the NPC)
- **mine shaft spawner building** — new `Building::MineShaft` spawns miners directly (1:1 building→NPC like House→Farmer, Barracks→Guard); replaces the confusing `job_reassign_system` that converted farmers↔miners via `MinerTarget` DragValue; `resolve_spawner_npc()` building_kind=3 → Miner (finds nearest gold mine); buildable from player build menu (cost=1 food); AI builds mine shafts by personality (Aggressive=1/3, Balanced=1/2, Economic=2/3 of houses); `try_build_inner()` DRY helper consolidates 5 identical AI build arms; deleted `MinerTarget` resource + `job_reassign_system` + miner DragValue UI
- **DRY: building + spawner + harvest consolidation** — `build_and_pay()` shared by player build menu + AI (eliminates duplicated place+pay+spawner logic); `register_spawner()` single construction site for all SpawnerEntry structs; `Building::spawner_kind()` derives spawner type from enum (no more magic 0/1/2 numbers); `resolve_spawner_npc()` shared by startup + respawn for building_kind→SpawnNpcMsg mapping; `FarmStates::harvest()` single authority for Ready→Growing transition used by farmer harvest (3 sites) and raider theft
- **build placement overhaul** — replaced right-click context menu with bottom-center horizontal build bar showing building sprites (cached atlas extraction) + concise help text; click-to-place with grid-snapped ghost preview (green=valid, red=invalid); destroy mode in build bar and inspector; `TownGrid.area_level` replaces `HashSet<(i32,i32)>` for expandable build area; `BuildKind::Destroy` + `DestroyRequest` resource for inspector→system destroy flow; slot indicators only visible during active placement
- **UI scale** — `ui_scale` setting (default 1.2, range 0.8-2.5) persisted in `UserSettings`; applied via `EguiContextSettings.scale_factor`; slider in pause menu Settings
- **in-game help tooltips** — `HelpCatalog` resource with ~35 help entries; `help_tip()` renders "?" buttons with rich hover tooltips; top bar has getting-started tip + tips on every stat (Food, Gold, Pop, Farmers, Guards, Raiders); every left panel tab (Roster, Upgrades, Policies, Patrols, Squads, Intel, Profiler) shows contextual help at top; build menu buttons have detailed hover text; NPC inspector shows tips on Level/XP, Trait, Energy, State; all help text answers "what is this?" AND "how do I use it?"
- **embedded assets** — release builds embed all assets (sprites + shaders) in the binary via `bevy_embedded_assets` v0.15; standalone 81MB exe runs without any external files; `ReplaceAndFallback` mode allows asset modding by placing files next to exe; assets restructured into standard Bevy layout (`rust/assets/sprites/`, `rust/assets/shaders/`); debug builds still load from disk for hot-reload

## 2026-02-14

- **squad auto-replenish** — squads now have a `target_size` field; set via DragValue in Squads tab instead of +1/+2/+4 recruit buttons; `squad_cleanup_system` auto-recruits unsquadded player guards when members drop below target (e.g. death) and dismisses excess when target is lowered; target_size=0 disables auto-recruit (manual mode); Dismiss All resets target_size to 0
- **profiler columns** — profiler panel now uses 3-column grid (system | ms | count); count entries (`decision/n_*`) paired to their timing row instead of mixed in as fake millisecond values; renamed `d.*` keys to `decision/*` for readability

## 2026-02-13

- **building spatial grid** — CPU-side spatial grid (`BuildingSpatialGrid`) for O(1) building lookups; 256px cells, rebuilt once per frame by `rebuild_building_grid_system` (before `decision_system`); replaces linear scans + Vec allocation in `find_location_within_radius`, `find_within_radius`, `find_nearest_free`, `find_nearest_location`; all use `distance_squared` instead of `sqrt`; `for_each_nearby` closure pattern avoids intermediate allocations; indexes farms, guard posts, towns, and gold mines; `d.arrival` should drop from ~2ms to ~0.1-0.3ms at 4700 NPCs
- **profiler copy button** — "Copy Top 10" button in system profiler panel copies frame time + top 10 system timings to clipboard
- **combat log keyboard toggle** — press L to show/hide combat log window; `combat_log_visible` field on `UiState`
- **decision throttling** — three-tier NPC decision system: arrivals every frame, combat flee/leash every 8 frames (~133ms), all other decisions (rest/work/idle scoring) bucketed by configurable interval (default 2s); `NpcDecisionConfig` resource + "NPC Think" slider in main menu (0.5-10s); with 5100 NPCs at 2s interval, only ~42 NPCs evaluate slow decisions per frame instead of 5100; `SystemTimings.record()` helper for sub-profiling; `d.arrival`/`d.combat`/`d.idle` sub-scope timings in profiler
- **miner job type** — proper `Job::Miner` (job_id=4, brown tint) replaces the `mining_pct` slider; houses still spawn farmers, `job_reassign_system` converts idle farmers↔miners to match per-town `MinerTarget`; `Miner` marker component, own Work branch in `decision_system` (find nearest mine with gold), shares farmer schedule/flee/off-duty policies; DragValue control in roster panel sets miner count; AI sets miner targets by personality (Aggressive=1/3, Balanced=1/2, Economic=2/3 of houses); miner base stats match farmer; miners show brown color in roster/left panel; conversion is bidirectional — reducing miner count converts them back to farmers with nearest free farm assignment
- **gold mines** — wilderness resource nodes placed between towns during world gen; miners walk out to mine gold; miners claim occupancy, extract `MINE_EXTRACT_PER_CYCLE` gold when tired, carry it home via `Activity::Returning { gold }` for proximity delivery to `GoldStorage`; mines regenerate `MINE_REGEN_RATE` gold/hour when unoccupied (capped at `MINE_MAX_GOLD`); mines are unowned — any faction's NPCs can use any mine; `Building::GoldMine` variant with `TileSpec::Single(43, 11)` sprite; HUD top bar shows gold count; mine inspector shows gold amount + progress bar + miner count; main menu slider for gold mines per town (0-10); `MineStates` + `GoldStorage` resources; `mine_regen_system` in Step::Behavior; `WorldGenConfig.gold_mines_per_town` persisted in settings
- **continents world generation** — new "Continents" mode selectable from main menu combo box; 3-octave fBm elevation noise with square-bump edge falloff (Red Blob Games approach) + independent moisture noise for biome selection (dry→Rock, moderate→Grass, wet→Forest); towns/camps constrained to land cells; `WorldGenStyle` enum in `WorldGenConfig`, persisted in settings; Classic mode unchanged as default
- **floating inspector** — NPC/building inspector changed from full-width `TopBottomPanel::bottom` to floating `egui::Window` anchored bottom-left; only visible when something is selected; matches combat log pattern
- **combat log wider** — 350px → 450px
- **system profiler** — `SystemTimings` resource with RAII `timings.scope("name")` guard pattern; internal Mutex so parallel systems don't serialize; toggle with F5; EMA-smoothed per-system millisecond timings
- **SystemParam bundles** — `CleanupResources` (health.rs, 9 resources) and `DecisionExtras` (behavior.rs, 6 resources) keep `death_cleanup_system` and `decision_system` under Bevy's 16-param limit
- **projectile dodge** — NPCs in combat stand their ground and shoot, but actively dodge incoming enemy projectiles; GPU spatial grid for projectiles (3-mode dispatch: clear, build, query) mirrors NPC grid pattern; NPC compute scans 3×3 neighborhood for approaching arrows within 60px, strafes perpendicular to projectile velocity with urgency scaling; 1-frame latency (proj grid built by projectile compute, read by NPC compute next frame); fixes combat circling bug where `SetTarget` to enemy position every frame reset arrival flag, causing separation/dodge physics to orbit NPCs counter-clockwise
- **ai weighted random decisions** — AI building/upgrade decisions now use scored weighted random selection (same pattern as NPC behavior system) instead of strict priority ordering; personality weights bias probabilities (Aggressive favors barracks, Economic favors farms) but don't hard-lock; need factors scale scores based on building ratio deficits; fixes bug where Balanced/Economic AI never built barracks (farm+house conditions were mutually exhaustive)
- **external building sprites** — House, Barracks, and GuardPost buildings use dedicated 32x32 PNGs (`house.png`, `barracks.png`, `guard_post.png`) instead of world atlas tiles; new `TileSpec::External(usize)` variant; `build_tileset` accepts extra images slice for non-atlas tiles
- **faction-based NPC coloring** — player faction (0) uses job colors (green/blue/red/yellow), all other factions use faction palette; previously color was job-based (only raiders got faction colors)
- **fix: turret npc_count** — `guard_post_attack_system` uses `gpu_state.npc_count` instead of `positions.len() / 2` for enemy scanning bounds
- **2x2 composite tiles** — `TileSpec` enum (`Single`/`Quad`) enables tiles built from four 16x16 sprites; `build_tileset` produces 32x32 array texture layers with nearest-neighbor 2x upscale for single sprites and quadrant blitting for composites; Rock terrain, Farm, Camp, and Tent buildings now use 2x2 composites; new grass sprites (A=3,16 B=3,13)
- **carried food untinted** — food sprite on returning raiders renders with original texture colors instead of faction color tint; equipment layers (atlas 0) still use job color, carried items (atlas >= 0.5) use white
- **roster faction filter** — roster panel only shows player faction (faction 0) NPCs by default; existing "All NPCs in Roster" debug setting in pause menu toggles all-faction view; replaces old raiders-only hide filter
- **background fps setting** — pause menu checkbox "Full FPS in Background" keeps game running at full framerate when window is unfocused; persisted in settings; applied on startup via WinitSettings
- **terrain-visual test** — new debug test showcasing all terrain biomes and building types in a labeled grid; test cleanup now despawns tilemap chunks and resets TilemapSpawned

## 2026-02-12

- **ai personalities** — AI players get random personality (Aggressive/Balanced/Economic) at game start; personality drives build order, upgrade priority, food reserve threshold, and town policies; combat log shows personality tag (`Town [Balanced] built farm`); smart slot selection: economy buildings prefer inner slots, guard posts prefer outer slots with min spacing of 5; slot unlock now sets terrain to Dirt (visible on tilemap via new `sync_terrain_tilemap` system + `TerrainChunk` marker)
- **ai players** — autonomous AI opponents that build, unlock slots, and buy upgrades; Builder AI (farms/houses/barracks/guard posts), Raider AI (tents); unique faction per settlement; `ai_decision_system` in Step::Behavior; configurable interval (1-30s); purple "AI" combat log entries with filter
- **world gen refactor** — independent placement of player towns, AI towns, and raider camps (no longer paired 1:1); configurable counts (AI Towns 0-10, Raider Camps 0-10); unique faction per settlement; removed `find_camp_position()` helper
- **main menu overhaul** — "Towns" renamed to "Your Towns" for clarity; AI Towns / Raider Camps / AI Speed sliders; per-town sliders in collapsible section; Reset Defaults button; removed "Colony simulation" subtitle
- **fix: NPC count estimate** — estimate now correctly counts AI town NPCs and uses raider camp count (not player town count) for raiders
- **fix: turret friendly fire** — `guard_post_attack_system` looks up post's owning faction from town data instead of hardcoding faction 0; prevents turrets from shooting their own town's NPCs
- **fix: spawner faction** — `spawner_respawn_system` + `game_startup_system` use `world_data.towns[idx].faction` instead of hardcoded 0; enemy town farmers/guards now spawn with correct faction
- **delete combat_log.rs** — dead code removed (undeclared module, never registered, referenced nonexistent `UiState.combat_log_open`)
- **fix: healing fountain drift deadlock** — NPCs in `HealingAtFountain` state could be pushed out of healing range by separation physics and get stuck forever (HP never recovers, decision system `continue`s); added drift check that re-targets fountain when NPC drifts >100px from town center; added early arrival so `GoingToHeal` NPCs transition to `HealingAtFountain` as soon as they enter healing range (100px) instead of walking to exact center
- **fix: duplicate "Healing, Healing" state display** — NPC inspector was showing both `Activity::HealingAtFountain` name and `Healing` marker component; removed marker components (AtDestination, Starving, Healing) from state display — only shows Activity + CombatState enums
- **target overlay visibility** — thicker line (1.5→2.5px), brighter alpha (140→200), larger diamond (5→7px) and NPC circle (8→10px radius)
- **squads system** — player-directed guard groups; 10 squads with map target markers; reassign existing patrol guards via +1/+2/+4/+8/+16/+32 recruit buttons; squad guards walk to target instead of patrolling; all survival behavior preserved (flee, rest, heal, sleep); `SquadState` resource + `SquadId` component + `squad_cleanup_system`; new Squads tab in left panel (Q key), top bar button, colored numbered target overlay, click-to-place targeting with ESC/right-click cancel

## 2026-02-11

- **separate rest from heal** — NPCs go home (spawner) to rest (energy recovery) and to the fountain to heal (HP recovery); new `GoingToHeal` + `HealingAtFountain{recover_until}` Activity variants; `Resting` simplified to unit variant (energy-only); raiders now heal at camp center like villagers (removed raider exclusion); raider Home changed from camp center to tent position; sleep icon only shows for Resting, not HealingAtFountain; energy recovers during both states to prevent ping-pong
- **sleep sprite texture** — sleep indicator now uses dedicated `sleep.png` texture (4th atlas, bindings 6-7, atlas_id=3.0) instead of character sheet lookup; white tint preserves sprite's natural blue Zz; fragment shader dispatches sleep (≥2.5) → heal (≥1.5) → normal; sleep_visual test stride fix (`idx*2` → `idx*3`) and assertion updated for atlas-based check
- **color saturation** — job colors changed from tinted to pure (farmer=green, guard=blue, raider=red, fighter=yellow); raider faction palette saturated (10 distinct pure colors instead of muted tints)
- **healing halo** — healing NPCs show a yellow halo ring sprite (`heal.png`) instead of a small icon overlay; third texture atlas bound at group 0 bindings 4-5 (`atlas_id=2.0`); healing layer renders at scale 20 with yellow tint; heal_visual test updated for new signal format (stride fix: `idx*2` → `idx*3`)
- **color tuning** — guard blue tint slightly darker (0.4→0.3 green), raider base red more saturated (0.5→0.3)
- **npc-visuals test scene** — new test in Debug Tests that spawns all 4 NPC types (Guard/Farmer/Raider/Fighter) in a labeled 4×7 grid showing each render layer individually (body, weapon, helmet, item, sleep, heal, full); egui labels at world positions with sprite coordinates; stays on screen for visual review
- **sprite coordinate updates** — Guard (0,0), Fighter (1,9), sword (45,6), helmet (28,0), sleep icon (24,7), food (24,9 on world atlas)
- **per-sprite atlas_id** — equipment/overlay buffers expanded from stride 2 (col, row) to stride 3 (col, row, atlas); body layer reads atlas from sprite_indices[2]; `SetSpriteFrame` gains `atlas` field; food carried item renders from world atlas; body layer skips rendering when sprite col < 0
- **per-job work schedules** — `work_schedule` split into `farmer_schedule` + `guard_schedule` in PolicySet; policies panel reorganized by job (Guards/Farmers sections)
- **auto-upgrade system** — per-upgrade auto-buy checkbox in Upgrades tab; `AutoUpgrade` resource + `auto_upgrade_system` queues affordable upgrades each game hour
- **remove FarmerCap/GuardCap upgrades** — UPGRADE_COUNT 14→12; population is building-driven (Stage 11), not upgrade-driven
- **merge policies+upgrades into left_panel** — deleted `policies_panel.rs` and `upgrade_menu.rs`; all UI lives in `left_panel.rs`
- **fix: raider wander drift** — `Action::Wander` now offsets from home position instead of current position, preventing unbounded random walk off the map; farm-seeking on raid arrival excludes current farm position and filters tombstoned farms, falls back to returning home if no other farm found; HP work gate lowered from 50% to 30% so starving raiders (HP capped at 50%) can still join raid queues
- **rename: Hut → House** — `Building::Hut` → `Building::House`, `WorldData.huts` → `WorldData.houses`, `HUT_BUILD_COST` → `HOUSE_BUILD_COST`, UI labels updated throughout
- **farms per town slider** — separate "Farms" slider in main menu (persisted in settings); farms placed first in spiral, then houses, then barracks
- **guard posts on corners** — guard posts placed at outer corners of all buildings (TL/TR/BR/BL) instead of spiral, ensuring perimeter coverage regardless of building count
- **fix: camera zoom over UI** — scroll wheel zoom disabled when pointer is over egui panels (combat log, etc.)
- **HUD: total population** — top bar shows `Pop: alive/total_spawners`
- **slider range increase** — barracks and tents sliders now go up to 5000
- **raider tent spawners** — raiders now spawn from individual Tent buildings instead of bulk camp spawns; `Building::Tent` variant + `WorldData.tents` + `BUILDING_TILES[7]`; `raider_respawn_system` removed, unified into `spawner_respawn_system` (building_kind 2=Tent → Raider with camp center as home)
- **camp TownGrids** — raider camps get expandable building grids like villager towns; `TownGrid` gains `town_data_idx` field replacing fragile `grid_idx * 2` mapping; `find_town_slot()` iterates all grids using stored index; `place_camp_buildings()` places Camp center + N Tents via spiral
- **build menu: camp support** — right-clicking camp grid slots shows Tent build option; villager-only buildings (Farm/GuardPost/Hut/Barracks) gated to faction==0 grids
- **guard posts on perimeter** — guard posts now placed after all spawner buildings via `spiral_slots()` so they're always on the outer ring regardless of slider values
- **HUD: raider/tent counts** — top bar shows `Raiders: alive/tents` for first raider camp; building inspector supports Tent (shows linked NPC + respawn timer)
- **main menu: rename raiders → tents** — slider now labeled "Tents" (1 raider per tent)

## 2026-02-10

- **fix: spiral building placement** — replace hardcoded 12-slot spawner array with `spiral_slots()` generator; `generate_world()` now populates `TownGrids` directly, auto-unlocking slots beyond base 6x6 grid; supports slider values up to 1000 huts/barracks per town
- **fix: settings path cross-platform** — fall back to `HOME` env var when `USERPROFILE` missing (macOS/Linux)
- **readme: per-platform getting started** — separate Windows/macOS/Linux install instructions with prerequisites
- **refactor: FarmOccupancy → BuildingOccupancy** — generic `Worksite` trait + `find_nearest_free()`/`find_within_radius()`/`find_by_pos()` replace farm-specific helpers; private field with claim/release/is_occupied/count API prevents double-increment bugs
- **fix: town index convention** — remove `÷2` pair-index conversion; NPCs and buildings both use direct WorldData indices (villagers at even, raiders at odd); fixes build menu spawner town_idx, spawner_respawn, and `build_patrol_route` (now `pub(crate)`)
- **UI: building inspector** — click buildings to inspect; shows per-type details (farm growth/occupancy, spawner NPC status/respawn timer, guard post patrol order/turret, fountain heal radius/food, camp food); `SelectedBuilding` resource with grid col/row
- **UI: Patrols tab (T)** — left panel tab to view and reorder guard post patrol routes; swap buttons mutate `WorldData` which triggers `rebuild_patrol_routes_system`
- **rename: right_panel → left_panel** — `RightPanelTab` → `LeftPanelTab`, `right_panel_open` → `left_panel_open`, module renamed
- **GPU: merged dodge/separation scan** — single 3x3 grid loop computes both separation and dodge forces; same-faction 1.5x push boost; avoidance clamped to `speed * 1.5`; lateral steering replaces backoff slowdown (routes around obstacles at 60% speed instead of jamming); backoff cap reduced from 200 to 30
- **HUD: per-town spawner counts** — top bar filters spawners by player's town_idx instead of showing global totals; format changed to `Farmers: alive/huts`, `Guards: alive/barracks`
- **rebuild_patrol_routes_system** — new system in `Step::Behavior` rebuilds all guards' patrol routes when `WorldData` changes (guard post added/removed/reordered)

## 2026-02-10

- **fix: enforce 1 farmer per farm** — `find_nearest_free_farm()` helper skips occupied farms; farm claiming gated on `FarmOccupancy` at arrival, spawn, and respawn; farmers redirect to free farm or idle when all occupied
- **remove role reassignment** — `reassign_npc_system`, `ReassignQueue` resource, and roster panel reassign buttons removed (building spawners replaced this workflow)
- **roadmap: stage 14 tower defense** — Wintermaul Wars-inspired TD mechanics: maze building with path validation, elemental rock-paper-scissors (6 elements), income/interest economy, competitive creep sending via guards, tiered tower upgrades, branching tower evolution
- **fix: guard post patrol order** — reorder post_slots so guards patrol clockwise (TL → TR → BR → BL) instead of arbitrary order
- **fix: newly-built spawner timing** — change spawner timer check from `> 0.0` to `>= 0.0` so newly-built Huts/Barracks (timer=0.0) spawn their NPC on the next hourly tick
- **fix: settings loaded at app startup** — replace `init_resource::<UserSettings>()` with `insert_resource(load_settings())` so saved settings persist across app restarts

## 2026-02-10

- **remove beds** — NPCs rest at their spawner building (Hut/Barracks) instead of separate beds
  - remove beds from world gen (`place_town_buildings`), build menu, `BedOccupancy` resource, `LocationKind::Bed`
  - spawner_respawn_system sets home to building position instead of nearest bed
  - world-gen test updated: no longer counts beds in building phase
  - reduce hut/barracks build cost from 3/5 to 1/1
- **increase raider camp distance** from 1100px to 3500px (~3x farther, ~35s travel)
- **esc closes build menu** before toggling pause menu (standard innermost-UI-first pattern)
- **roadmap: continent world generation spec** added to stage 12

## 2026-02-11

- **stage 11: building spawners — population driven by Hut/Barracks buildings**
  - add `Hut`/`Barracks` building types with `Building` enum variants, tile sprites, place/remove/tombstone support
  - `SpawnerState` resource tracks building→NPC links; each entry has building_kind, position, npc_slot, respawn_timer
  - `spawner_respawn_system` (hourly): detects dead NPCs via `NpcEntityMap`, counts down 12h timer, spawns replacement via `SlotAllocator` + `SpawnNpcMsg`
  - `game_startup_system` builds `SpawnerState` from world gen Huts/Barracks, spawns 1 NPC per building (replaces bulk farmer/guard loops)
  - `place_town_buildings` places N Huts + N Barracks from config sliders (sliders renamed to Huts/Barracks)
  - build menu: Hut (3 food) and Barracks (5 food) buttons with spawner entry on build, spawner tombstone on destroy
  - HUD top bar shows spawner counts (Huts: N, Barr: N, with respawning count)
  - cleanup resets `SpawnerState` on game exit

- **fix: ghost NPC rendering — replace NpcCount with SlotAllocator**
  - remove `NpcCount` resource (running total, never decremented on death — caused GPU to dispatch uninitialized slots)
  - remove `GpuDispatchCount` resource and `GPU_DISPATCH_COUNT` static (redundant with SlotAllocator)
  - GPU compute dispatch, flash decay, and projectile collision all use `SlotAllocator.count()` (high-water mark)
  - add `SlotAllocator.alive()` (`next - free.len()`) for UI display and test assertions
  - single source of truth for NPC counting eliminates the class of bug where multiple count resources diverge

- **fix: GPU readback for factions and health**
  - add factions + health readback buffers (COPY_SRC on GPU buffers, ReadbackComplete observers, render node copy)
  - enables `count_nearby_factions()` for flee threat assessment and guard post turret targeting
  - initialize combat_target readback buffer to -1 (prevents zeroed memory misread as "target NPC 0" causing instant kills on frame 1)
  - initialize factions readback buffer to -1 (prevents unspawned slots being treated as faction 0)

- **fix: guard patrol cross-town bug**
  - `build_patrol_route()` now converts NPC WorldData index (0, 2, 4...) to building pair index (0, 1, 2...) via `/ 2`
  - guards from town 1+ were patrolling wrong town's guard posts due to index convention mismatch

- **UI: left panel changed from SidePanel to floating Window**
  - roster/upgrades/policies panel now an anchored egui::Window with close button
  - changed Res<UiState> to ResMut<UiState> to support window close

## 2026-02-10

- **UI overhaul: top bar, bottom panel, policy persistence, FPS overlay**
  - replace left HUD panel with full-width top bar (panel toggles left, town name + time center, stats right) and bottom panel (inspector left, combat log with filter checkboxes right)
  - merge combat_log.rs into game_hud.rs bottom panel — remove standalone combat_log module
  - right panel renamed to left panel (SidePanel::left), simplified to heading + content (no inline tab bar)
  - move FPS counter from tests/mod.rs to ui/game_hud.rs, anchor bottom-right, register globally (visible on all screens)
  - persist town policies to settings.json (PolicySet + WorkSchedule + OffDutyBehavior now serde Serialize/Deserialize)
  - load saved policies on game startup, save on leaving Policies tab
  - settings file moved from executable directory to `Documents\Endless\settings.json`
  - add debug settings to pause menu (Enemy Info, NPC Coordinates, All NPCs in Roster)
  - add NPC Activity filter to combat log
  - roster hides raiders unless "All NPCs" debug setting enabled
  - remove `combat_log_open` from UiState (combat log now always visible in bottom panel)

- **behavior: tighten eat/rest thresholds, fix raider queue**
  - eat only scored when energy < ENERGY_EAT_THRESHOLD (10) — emergency only, NPCs prefer rest
  - rest only scored when energy < ENERGY_HUNGRY (50) — prevents unnecessary rest at high energy
  - fix raiders re-wandering every frame while already queued for raid — only wander on initial queue join
  - initialize GPU combat target buffer to -1 (prevents zeroed memory misread as "target NPC 0")

- **escape menu, tabbed right panel, maximized window**
  - ESC opens pause menu overlay (Resume, Settings, Exit to Main Menu) instead of instantly quitting — game stays in Playing state, auto-pauses when menu opens
  - pause menu settings: scroll speed slider + combat log filter checkboxes, saved to UserSettings on close
  - consolidated Roster, Upgrades, Policies into a single tabbed right SidePanel (`right_panel.rs`) — always-visible tab bar (200px collapsed), expands to 340px on tab click, re-click active tab to collapse
  - removed Roster/Upgrades/Policies toggle buttons from left HUD (now accessed via right panel tabs or R/U/P keys)
  - fixed combat log layout shift caused by non-deterministic egui system ordering — all egui systems now `.chain()` in one `add_systems` call: HUD (left) → right panel → bottom panel → pause overlay
  - window starts maximized via `set_maximized(true)` startup system
  - old panel files (roster_panel.rs, upgrade_menu.rs, policies_panel.rs) no longer compiled, replaced by right_panel.rs

- **fix starving wounded oscillation + UI polish**
  - fix decision system oscillation: starving wounded NPCs looped between "Resting" and "Wounded → Fountain" every frame because fountain healing can't exceed the 50% starvation HP cap — skip wounded→fountain redirect when energy=0 so NPCs rest for energy first
  - fix arrival wounded check: if NPC is already Resting when wounded check fires, stamp `recover_until` threshold on existing state instead of redirecting to GoingToRest (prevents redirect loop at destination)
  - deselect NPC inspector when selected NPC dies (`death_cleanup_system` clears `SelectedNpc`)
  - persist combat log filter toggles to `UserSettings` JSON — filters load on init, save on change
  - main menu settings save now merges into existing file instead of overwriting (preserves log filters)
  - build menu opens at mouse cursor position (`fixed_pos` + `movable(false)`)

- **fix NPC inspector energy display + rebalance drain**
  - inspector energy bar was stuck at 100 — `NpcEnergyCache` resource was never synced
  - remove `NpcEnergyCache` entirely; HUD now queries `Energy` component directly (same pattern as HP)
  - change energy drain from 24h to 12h to empty — tighter rest/work cycle

- **energy-driven starvation**
  - remove `LastAteHour` component — energy is now the single survival resource
  - starvation triggers at energy=0 instead of 24h without eating
  - eating restores energy to 100 instantly (was +30)
  - starving speed penalty increased: 50% (was 75%)
  - starving HP cap unchanged at 50%
  - rest still works when starving (slow recovery, must walk home)
  - remove `keep_fed()` test helper and `STARVATION_HOURS`/`ENERGY_FROM_EATING` constants

- **stage 10: town policies**
  - add `TownPolicies` resource with `PolicySet` per town: flee thresholds, work schedule, off-duty behavior, healing priority
  - add `WorkSchedule` enum (Both/DayOnly/NightOnly) — gates work scoring in `decision_system` based on `GameTime.is_daytime()`
  - add `OffDutyBehavior` enum (GoToBed/StayAtFountain/WanderTown) — drives idle behavior when work is gated out
  - wire `policies_panel.rs` to `ResMut<TownPolicies>` — sliders/checkboxes directly mutate resource, removed `ui.disable()` and `Local<PolicyState>`
  - `decision_system` reads `Res<TownPolicies>` for policy-driven flee: guards use `guard_flee_hp`, farmers use `farmer_flee_hp`, raiders hardcoded 0.50
  - `guard_aggressive` disables guard flee, `farmer_fight_back` disables farmer flee
  - `guard_leash` policy controls whether guards return to post after combat (off = chase freely)
  - `prioritize_healing` sends wounded NPCs (HP < `recovery_hp`) to town fountain before resuming work
  - remove hardcoded `FleeThreshold(0.50)` and `WoundedThreshold(0.25)` from raider spawn — thresholds now policy-driven
  - fix `pseudo_random()` PRNG: old implementation discarded frame contribution via `>> 16` shift, causing identical rolls per NPC across frames (rest/wake loops). New xorshift mixing with Knuth's multiplicative hash (2654435761)

- **stage 9: upgrades & xp**
  - add `UpgradeQueue` resource + `process_upgrades_system`: UI pushes upgrade requests, system validates food cost, increments `TownUpgrades`, re-resolves `CachedStats` for affected NPCs
  - add `upgrade_cost(level) = 10 * 2^level` (doubles each level, capped at 20)
  - add last-hit XP tracking: `DamageMsg.attacker` → `LastHitBy` component → `xp_grant_system` grants 100 XP to killer on death
  - add `level_from_xp(xp) = floor(sqrt(xp/100))`, level multiplier `1.0 + level * 0.01` wired into `resolve_combat_stats()`
  - level-up rescales current HP proportionally to new max, emits `CombatEventKind::LevelUp` to combat log (cyan)
  - wire `upgrade_menu.rs`: functional buttons show level/cost, push to `UpgradeQueue`, disabled when unaffordable
  - wire `farm_growth_system`: applies FarmYield upgrade multiplier per-town
  - wire `healing_system`: applies HealingRate + FountainRadius upgrades per-town
  - fix `starvation_system` speed: uses `CachedStats.speed * 0.75` instead of hardcoded 60.0
  - fix `reassign_npc_system`: passes actual NPC level instead of hardcoded 0
  - fix NPC meta init: `level: 0` (was 1), aligned with `level_from_xp(0) == 0`
  - remove Stage 8 `#[cfg(debug_assertions)]` parity checks
  - `game_hud.rs` NPC inspector shows XP and XP-to-next-level
  - `combat_log.rs` adds LevelUp filter checkbox + cyan color
  - `main_menu.rs` adds DragValue widgets alongside sliders for typeable config inputs

- **stage 8: data-driven stats**
  - add `systems/stats.rs`: `CombatConfig` resource with per-job `JobStats` + per-attack-type `AttackTypeStats`, `TownUpgrades` resource stub, `resolve_combat_stats()` resolver
  - add `CachedStats` component on all NPCs — resolved from config on spawn, replaces `AttackStats` + `MaxHealth`
  - add `BaseAttackType` enum (Melee/Ranged) as ECS component, keys into `CombatConfig.attacks` HashMap
  - remove `AttackStats` struct and `MaxHealth` struct — `CachedStats` is single source of truth
  - add `Hash` derive to `Job` (needed as HashMap key in CombatConfig)
  - wire `Personality::get_stat_multipliers()` into resolver (was defined but never called)
  - `attack_system` reads `&CachedStats` instead of `&AttackStats` (same query shape, data-driven)
  - `healing_system` reads `CombatConfig.heal_rate`/`heal_radius` instead of local constants, `&CachedStats` instead of `&MaxHealth`
  - `spawn_npc_system` and `reassign_npc_system` use resolver for all stats
  - UI queries (`game_hud.rs`, `roster_panel.rs`) and tests (`healing.rs`) migrated from `&MaxHealth` to `&CachedStats`
  - `#[cfg(debug_assertions)]` parity checks: assert resolved stats match old hardcoded values
  - formula: `final_stat = base[job] * upgrade_mult * trait_mult * level_mult` (upgrades/levels all 1.0 in Stage 8)
  - init values match exactly: guard/raider damage=15, all speeds=100, all max_health=100, heal_rate=5, heal_radius=150

- **camera follow + target indicator**
  - add `FollowSelected(bool)` resource, `camera_follow_system` in `render.rs` — tracks selected NPC position
  - F key toggles follow mode, WASD cancels follow (natural override)
  - add "Follow (F)" selectable button in game HUD NPC inspector
  - add `target_overlay_system` using egui painter on background layer — yellow line + diamond to NPC's movement target, blue circle on NPC
  - bundle 8 readonly HUD resources into `HudData` SystemParam to stay under 16-param limit

- **performance: render world entity leak fix + scale-up**
  - fix render world entity leak: `extract_npc_batch` and `extract_proj_batch` now despawn stale entities before spawning fresh ones — previously accumulated one entity per frame, causing `command_buffer_generation_tasks` to grow linearly over time
  - scale GPU spatial grid from 128×128×64px (8,192px coverage) to 256×256×128px (32,768px coverage) — fixes NPCs not colliding or targeting on worlds larger than ~250×250
  - raise max NPC count from 10K to 50K (both CPU `MAX_NPC_COUNT` and GPU `MAX_NPCS`)
  - remove dead CPU-side spatial grid constants from `constants.rs` (GRID_WIDTH/HEIGHT/CELLS, CELL_SIZE, MAX_PER_CELL — unused since GPU compute)
  - add chunked tilemap spec to roadmap (32×32 tile chunks for off-screen culling, not yet implemented)

- **stage 7: playable game features**
  - add `settings.rs`: `UserSettings` resource with serde JSON save/load to `endless_settings.json` next to executable
  - main menu loads saved settings on init (world size, towns, farmers, guards, raiders), saves on Play click
  - `camera_pan_system` reads `UserSettings.scroll_speed` instead of hardcoded constant
  - add `reassign_npc_system` in `systems/spawn.rs`: Farmer↔Guard role swap via component add/remove
  - roster panel gets `→G` (farmer→guard) and `→F` (guard→farmer) buttons per NPC row
  - `ReassignQueue` resource bridges UI (EguiPrimaryContextPass) to ECS (Update schedule) since MessageWriter unavailable in egui schedule
  - reassignment swaps: job marker, equipment (sword/helmet), patrol route, work position, activity, sprite frame, population stats
  - equipment visuals update automatically via `sync_visual_sprites` (no manual GPU equipment update needed)
  - add `guard_post_attack_system` in `systems/combat.rs`: turret auto-attack fires projectiles at nearest enemy within 250px
  - `GuardPostState` resource with per-post cooldown timers and attack_enabled flags, auto-syncs length with WorldData.guard_posts
  - add turret toggle in build menu: right-click guard post shows "Disable/Enable Turret" button
  - add guard post turret constants: range=250, damage=8, cooldown=3s, proj_speed=300, proj_lifetime=1.5s
  - add `ReassignMsg` to messages.rs (defined but unused — `ReassignQueue` resource used instead)

- **building system playtesting fixes**
  - fix coordinate misalignment: TOWN_GRID_SPACING 34→32px (matches WorldGrid cell size), remove -0.5 offset from `town_grid_to_world` so slot (0,0) = town center
  - rewrite `place_town_buildings` to use town grid (row,col) coordinates instead of float offsets
  - fix right-click unresponsive for ~30s: remove `is_pointer_over_area()` guard (too aggressive near any egui panel), keep only `wants_pointer_input()`
  - widen slot click radius from 0.45 to 0.7 of TOWN_GRID_SPACING
  - fix crash on second game start: add bounds check `if raider_idx >= world_data.towns.len()` in spawn loop
  - replace gizmo slot indicators with Sprite entities (`SlotIndicator` marker) at z=-0.3 — gizmos render in separate pass after all Transparent2d items, can't be z-sorted with buildings/NPCs
  - green "+" crosshairs for empty unlocked slots, dim bracket corners for adjacent locked slots
  - NPC sort_key 0.0→0.5 (above indicators, below projectiles)
  - lower all building costs to 1 for testing
  - change defaults: 2 guards/town, 0 raiders/camp for peaceful testing
  - add Stage 8 spec: stat resolution, upgrades, XP, policies (4-phase plan)

- **building system: right-click context menu with per-tile slot unlock**
  - add `TownGrid`/`TownGrids` per-town slot tracking with `HashSet<(i32,i32)>` unlock system
  - add `BuildMenuContext` resource for right-click context menu state
  - add `place_building()` / `remove_building()` in world.rs with tombstone deletion (-99999 position)
  - add `find_town_slot()`, `get_adjacent_locked_slots()`, coordinate helpers (town_grid_to_world, world_to_town_grid)
  - add `BuildingChunk` marker + `sync_building_tilemap` system for runtime tilemap updates when WorldGrid changes
  - add `slot_right_click_system`: right-click town slot → populate context → open build menu
  - rewrite `build_menu_system` as context-driven menu: Farm/Bed/GuardPost build, Destroy, Unlock buttons with food costs
  - add `draw_slot_indicators` gizmo overlay: green "+" empty slots, dim brackets locked adjacent, gold ring town center
  - add building cost constants: Farm=50, Bed=10, GuardPost=25, SlotUnlock=25 food
  - add grid constants: TOWN_GRID_SPACING=34px, base grid 6x6 (-2..3), expandable to 100x100
  - guard posts get auto-incrementing patrol_order based on existing post count
  - ported from Godot's per-town grid system (scenes/main.gd, ui/build_menu.gd)

- **5 UI panels: roster, combat log, build menu, upgrades, policies**
  - `roster_panel.rs`: right side panel (R key) — NPC list with job filter (All/Farmers/Guards/Raiders), sortable column headers (Name/Job/Lv/HP/State/Trait with ▼/▲ arrows), click-to-select, follow button moves camera to NPC, cached rows rebuild every 30 frames
  - `combat_log.rs`: bottom panel (L key) — event feed with color-coded timestamps (red=Kill, green=Spawn, orange=Raid, yellow=Harvest), filter checkboxes, auto-scroll, 200-entry ring buffer
  - `build_menu.rs`: floating window (B key) — Farm/Bed/GuardPost buttons with costs and tooltips, all disabled until Stage 7 backend
  - `upgrade_menu.rs`: floating window (U key) — 14 upgrade rows (Guard: Health/Attack/Range/Size/Speed/MoveSpeed/AlertRadius, Farm: Yield/FarmerHP/FarmerCap/GuardCap, Town: HealingRate/FoodEfficiency/FountainRadius), all disabled until Stage 8 backend
  - `policies_panel.rs`: floating window (P key) — checkboxes (Eat Food, Aggressive, Leash, Fight Back, Prioritize Healing), sliders (Farmer/Guard flee HP%, Recovery HP%), dropdowns (Work Schedule, Off-Duty behavior), all disabled until Stage 8 backend
  - add `UiState` resource (tracks which panels are open, combat_log defaults true)
  - add `CombatLog` resource (ring buffer VecDeque, max 200 entries, 4 event kinds)
  - add `ui_toggle_system` for keyboard shortcuts (R/L/B/U/P)
  - add panel toggle buttons to game HUD left panel (Roster/Log/Build/Upgrades/Policies)
  - add combat log emitters: `death_cleanup_system` → Kill, `spawn_npc_system` → Spawn (skip initial bulk), `decision_system` → Raid dispatch, `arrival_system` → Harvest
  - all panels ported from Godot originals (roster_panel.gd, combat_log.gd, build_menu.gd, upgrade_menu.gd, policies_panel.gd)


- **async GPU readback via Bevy `Readback` + `ReadbackComplete`**
  - replaces hand-rolled ping-pong staging buffers + blocking `device.poll(Wait)` (~9ms/frame)
  - 4 `ShaderStorageBuffer` assets as async readback targets (npc positions, combat targets, proj hits, proj positions)
  - `ReadbackComplete` observers write directly to `Res<GpuReadState>`, `Res<ProjHitState>`, `Res<ProjPositionState>`
  - deleted: `readback_all` (~140 lines), `StagingIndex`, 8 staging buffers, 3 static Mutexes
  - deleted: `sync_gpu_state_to_bevy` system + `systems/sync.rs` module
  - `GpuReadState` + `ProjPositionState` extracted to render world for instanced rendering
  - fix: proj hit readback buffer initialized with `[-1, 0]` per slot (zeroes misread as "hit NPC 0")
  - fix: `npc_count` no longer overwritten from readback (buffer is MAX-sized, actual count from `NpcCount` resource)

- **fix process_proj_hits iteration bounds and inactive skip**
  - iterate only up to `proj_alloc.next` (high-water mark) instead of full readback buffer
  - skip inactive projectiles (deactivated but stale in readback) via `proj_writes.active[slot] == 0` check
  - prevents wasted iteration over 50K unallocated slots

- **optimize per-frame visual sync and flash decay**
  - `sync_visual_sprites`: merged two-pass (clear all + set all) into single pass that writes defaults inline where components are absent, eliminating ~8K redundant array writes per frame at 500 NPCs
  - `populate_buffer_writes`: flash decay loop now iterates only active NPCs (npc_count) instead of all 16,384 MAX_NPCS slots
  - ~10% FPS improvement at 700+ NPCs (90 → 100 FPS)

- **fix projectile slot leak (9 FPS → 33+ FPS death spiral fix)**
  - expired projectiles now write `proj_hits[i] = (-2, 0)` sentinel in WGSL shader
  - `process_proj_hits` handles expired sentinel: frees slot + sends Deactivate
  - fixes death spiral where `ProjSlotAllocator.next` grew forever (never freed expired projectiles)
  - `command_buffer_generation_tasks` 24ms → 9.8ms, `RenderExtractApp` 40ms → 18ms

- **double-buffered ping-pong staging + unified readback**
  - `NpcGpuBuffers` and `ProjGpuBuffers` staging buffers changed from `Buffer` to `[Buffer; 2]`
  - `StagingIndex` resource tracks current/previous frame, flips each frame
  - compute nodes copy to `staging[current]`, readback reads `staging[1-current]`
  - unified `readback_all` replaces separate `readback_npc_positions` + `readback_proj_data`
  - single `device.poll()` maps all staging buffers (up to 4) in one call

- **per-index GPU uploads replace full-array uploads**
  - `NpcBufferWrites`: per-field boolean dirty flags → `Vec<usize>` dirty indices
  - `write_npc_buffers`: per-index `write_buffer` calls (8-byte writes instead of 128KB bulk)
  - `ProjBufferWrites`: per-slot `spawn_dirty_indices` and `deactivate_dirty_indices`
  - `write_proj_buffers`: spawn writes all fields per slot, deactivate writes only active+hits

- **equipment tinted with job color**
  - equipment layers (armor, helmet, weapon, item) now use job RGBA tint instead of white
  - guards' equipment renders blue, raiders' red — visually distinct at a glance

- **brighter job colors for tint-based rendering**
  - all job colors brightened (e.g. farmer 0.2,0.8,0.2 → 0.4,1.0,0.4)
  - raider faction palette raised (10 colors, min component ~0.4 instead of ~0.1)
  - tint-multiplication on white sprites needs brighter base colors to look vivid

- **add tracy profiling feature**
  - `tracy = ["bevy/trace_tracy"]` feature flag in Cargo.toml
  - build with `--features tracy` to enable Tracy instrumented spans

- **two-enum state machine: Activity + CombatState replace 13 marker components**
  - add `Activity` enum (Idle, Working, OnDuty, Patrolling, GoingToWork, GoingToRest, Resting, Wandering, Raiding, Returning) — models what NPC is *doing*
  - add `CombatState` enum (None, Fighting, Fleeing) — models whether NPC is *fighting*
  - concurrent state machines pattern: Activity preserved through combat (Raiding NPC stays Raiding while Fighting)
  - `Activity::is_transit()` replaces `HasTarget` marker — arrival detection derived from enum state
  - `Returning { has_food }` replaces `CarryingFood` marker — food state folded into activity
  - `Resting { recover_until: Some(t) }` replaces `Recovering` component — recovery folded into rest
  - remove 13 components: HasTarget, Working, OnDuty, Patrolling, GoingToWork, GoingToRest, Resting, Wandering, Raiding, Returning, InCombat, CombatOrigin, CarryingFood
  - remove `NpcStateParams` and `CombatParams` SystemParam bundles (enum queries replace marker queries)
  - update all 18 files: components, lib, 6 systems, gpu, ui, 8 tests
  - fix Bevy B0001 query conflict: `Without<AssignedFarm>` on arrival_system returning query for disjointness
  - cargo check: 0 errors, 0 warnings; cargo run --release: launches clean

- **fix terrain z-ordering: AlphaMode2d::Opaque → Blend**
  - terrain was rendering over NPCs because Opaque2d phase executes after Transparent2d regardless of z-value
  - both tilemap layers now use AlphaMode2d::Blend in Transparent2d phase (terrain z=-1.0, buildings z=-0.5, NPCs sort_key=0.0)

- **NPC debug inspector with clipboard copy**
  - add full debug section to game HUD: position, target, home, faction, all state components, recent log entries
  - add "Copy Debug Info" button using `arboard::Clipboard` directly (bevy_egui `EguiClipboard`/`ctx.copy_text()` didn't work)
  - add `NpcStateQuery` SystemParam bundle for querying 15 state marker components
  - guard `click_to_select_system` with `ctx.wants_pointer_input() || ctx.is_pointer_over_area()` — prevents game click handler from stealing egui button clicks (was deselecting NPC on same frame as Copy button press)
  - add `arboard = "3"` dependency

## 2026-02-09
- **main game mode: menu → world gen → play → HUD → cleanup cycle**
  - add `AppState` to `lib.rs` with 4 states: MainMenu (default), Playing, TestMenu, Running
  - game systems gated on `Playing | Running` via `.or()` run condition (shared between real game and debug tests)
  - add `ui/main_menu.rs`: egui main menu with world config sliders (size, towns, farmers, guards, raiders) + Play / Debug Tests buttons
  - add `ui/game_hud.rs`: egui side panel HUD with game time, population stats, food, kill stats, NPC inspector (HP/energy bars, job, trait, town)
  - add `ui/mod.rs`: game startup (OnEnter Playing) generates world + spawns NPCs per town, game cleanup (OnExit Playing) despawns all entities + resets resources, ESC/Space/+/- controls
  - add `TilemapSpawned` Resource (replaces `Local<bool>`) so cleanup can reset it for re-entry
  - change tilemap z-ordering: terrain -1.0→-100.0, buildings -0.5→-99.0 (NPCs were rendering under terrain)
  - add "Back to Menu" button in test menu alongside "Run All"
  - `CleanupWorld` + `CleanupDebug` SystemParam bundles keep cleanup system under 16-param limit
  - known issue: NPCs still render under terrain despite z-ordering change (needs further investigation)

- **sync_visual_sprites: derive visual state from ECS, remove redundant GPU messages**
  - add `sync_visual_sprites` system (gpu.rs): derives colors, equipment, indicators from ECS components each frame
  - remove 4 GpuUpdate variants: SetColor, SetHealing, SetSleeping, SetEquipSprite — visual state no longer deferred via messages
  - remove SetColor/SetEquipSprite sends from spawn_npc_system, decision_system, arrival_system, death_cleanup_system
  - remove SetHealing sends from healing_system — visual derived from Healing marker component
  - remove SetSleeping sends from decision_system — visual derived from Resting marker component
  - consolidate RAIDER_COLORS palette + `raider_faction_color()` to constants.rs (was duplicated in spawn.rs + behavior.rs)
  - healing_system: remove Healing marker when NPC fully healed (was only removed when leaving zone)
  - all 15 tests pass at time_scale=1 (no accelerated time), build clean with 0 warnings

- **fix 6 timed-out tests at time_scale=1**
  - energy: spawn with home=(-1,-1) so rest_score=0; start energy=55; completes in 6s
  - guard-patrol: start energy=40; P5 timeout 40s for recovery walk time; completes in 33s
  - farmer-cycle: start energy=35; P5 checks resting==0 not going_work; completes in 25s
  - sleep-visual: start energy=35; schedule tick after sync_visual_sprites; completes in 25s
  - economy: farm progress 0.5→0.95; completes in 30s
  - farm-visual: farm progress 0.8→0.95; completes in 1.5s
  - key pattern: set initial energy via mutable query + flag on first tick (avoids racing spawn frame)

- **suppress debug tick spam across test runs**
  - remove explicit `flags.combat=true` / `flags.readback=true` from vertical_slice, combat, projectiles test setups
  - add DebugFlags reset to `cleanup_test_world` (CleanupExtra bundle) — prevents flag bleed between tests

- **stage 6 green phase: visual indicator render layers + test refactor**
  - add 2 new render layers: Status (layer 5, sleep icon) and Healing (layer 6, heal glow)
  - LAYER_COUNT 5→7, EquipLayer enum extended with Status=4 and Healing=5
  - wire SetHealing to write HEAL_SPRITE to `healing_sprites` buffer (was no-op)
  - add SetSleeping message, sent from behavior.rs at 3 Resting insert/remove sites
  - add `farm_visual_system`: spawns/despawns FarmReadyMarker on farm state transitions
  - update sleep/heal visual tests to check dedicated buffers (`status_sprites`/`healing_sprites`)
  - refactor test infrastructure: TestSetupParams SystemParam bundle, tick_elapsed/require_entity/keep_fed helpers (-278 lines)
  - heal-visual phases 1-2 pass; sleep-visual and farm-visual still have timing issues to debug

- **fix healing test: all 3 phases pass**
  - keep farmer fed each tick (`LastAteHour = total_hours`) — isolates healing from starvation
  - healing was 2/3 (hp regressed due to starvation HP cap), now ALL PASS
  - all 12 behavior tests pass; 3 visual indicator red tests remain (expected)

- **review fixes for visual indicator tests**
  - `sleep_visual.rs`: add missing FarmStates init, fix `starting_post: 0` → `-1`
  - `mod.rs`: add FarmReadyMarker cleanup to `cleanup_test_world`

- **fix guard-patrol test: all 5 phases pass**
  - tired guards now leave post (Priority 6 energy check) and fall through to rest scoring
  - test keeps guard fed via `LastAteHour` reset each tick — isolates duty cycle from starvation
  - guard-patrol was 3/5, now ALL PASS

- **gate tick spam behind debug flag**
  - "Tick: N NPCs active" log now requires `flags.readback` (F1) to be on
  - merge the NPC count log into the readback block — no output unless F1 toggled

- **visual indicator test infrastructure (red tests)**
  - add `sleep_visual.rs`: resting NPC gets SLEEP_SPRITE on item layer, cleared on wake (3 phases)
  - add `farm_visual.rs`: ready farm spawns FarmReadyMarker entity, removed on harvest (3 phases)
  - add `heal_visual.rs`: healing NPC gets HEAL_SPRITE on item layer, cleared when healed (3 phases)
  - add `FarmReadyMarker` component, `SLEEP_SPRITE` and `HEAL_SPRITE` constants
  - tests are expected to fail Phase 2 until visual systems write to `item_sprites`/spawn markers

- **require(HasTarget) on transit components: compile-time arrival detection guarantee**
  - add `#[require(HasTarget)]` to 6 transit components (Patrolling, GoingToRest, GoingToWork, Raiding, Returning, Wandering)
  - add `Default` derive to `HasTarget` (required by `#[require]`)
  - remove 13 manual `.insert(HasTarget)` calls from `decision_system` and 1 from `spawn_npc_system`
  - Bevy's required components auto-insert `HasTarget` on any `.insert(Patrolling)` etc — impossible to forget

- **camera: eliminate CameraState duplication, Bevy camera is single source of truth**
  - remove `CameraState` from main world (`init_resource`, `ExtractResourcePlugin`)
  - remove `camera_viewport_sync` and `camera_transform_sync` systems (6 systems → 4)
  - `camera_pan_system` and `camera_zoom_system` write directly to `Transform` + `Projection`
  - `click_to_select_system` reads `Transform` + `Projection` instead of `CameraState`
  - add `extract_camera_state` in render world ExtractSchedule: reads Camera2d entity → builds CameraState for shader
  - add `ortho_zoom()` helper: reads zoom from `Projection::Orthographic.scale`

- **building tilemap: two-layer TilemapChunk (terrain + buildings)**
  - buildings now rendered via second TilemapChunk layer (z=-0.5, AlphaMode2d::Blend) on top of terrain (z=-1, Opaque)
  - generic `build_tileset(atlas, tiles, images)` replaces `build_terrain_tileset()` — same pixel-copy logic, parameterized by tile list
  - add `BUILDING_TILES` const (5 atlas positions: fountain, bed, guard post, farm, camp)
  - add `Building::tileset_index()` mapping building variant to tileset array index
  - add `spawn_chunk()` DRY helper in render.rs, called twice for terrain and building layers
  - rename `spawn_terrain_tilemap` → `spawn_world_tilemap`
  - remove `WorldRenderInstances` resource, `compute_world_render_instances` system, `ExtractResourcePlugin` from instanced renderer
  - `LAYER_COUNT` 6→5: body(0), armor(1), helmet(2), weapon(3), item(4) — buildings no longer instanced
  - remove dead code from world.rs: `SpriteDef`, `LocationType`, `SpriteInstance`, `get_all_sprites()`, sprite constants

- **terrain tilemap: migrate 62K instanced terrain to TilemapChunk**
  - terrain rendered via Bevy's built-in `TilemapChunk` (single quad, fragment shader tile lookup, zero per-frame CPU cost)
  - `build_terrain_tileset()`: extracts 11 terrain tiles from world atlas into `texture_2d_array` at runtime
  - `Biome::tileset_index()`: maps biome + cell position to tileset array index (0-10)
  - `spawn_terrain_tilemap` system: spawns TilemapChunk entity when WorldGrid populated + atlas loaded
  - remove terrain from instanced pipeline: `LAYER_COUNT` 7→6, `WorldRenderInstances` buildings-only
  - remove dead `Biome::sprite()` method (replaced by tileset_index)
  - add FPS counter overlay (egui, bottom-left, EMA-smoothed)
  - suppress tick log when 0 NPCs active

- **unified instanced renderer: terrain + buildings + NPCs in one pipeline**
  - rename `NpcInstanceData` → `InstanceData`, add `scale` (per-instance quad size) and `atlas_id` (atlas selection) fields (40→48 bytes)
  - bind both character and world atlases simultaneously (group 0, bindings 0-3)
  - shader selects atlas per-instance: character (NPCs/equipment/projectiles) or world (terrain/buildings)
  - add `WorldRenderInstances` resource: pre-computed terrain + building instances, extracted to render world
  - add `compute_world_render_instances` system: builds 62,500 terrain + ~42 building instances from WorldGrid
  - add `Biome::sprite(cell_index)` method for deterministic terrain sprite variation
  - `LAYER_COUNT` 5→7: terrain (0), buildings (1), NPC body (2), equipment (3-6)
  - world-gen test now renders visible terrain tiles and buildings
  - tests stay running after pass/fail (single test mode) — user clicks Back to return

- **fix arrival detection: consolidate to single targeting path**
  - remove dead `SetTargetMsg` + `apply_targets_system` (redundant O(n) entity scan, nobody wrote SetTargetMsg)
  - remove dead `ArrivalMsg` + `ARRIVAL_QUEUE` + `drain_arrival_queue` (nothing ever wrote to the queue)
  - remove ArrivalMsg event-reading section from `arrival_system` (now proximity checks only)
  - add `HasTarget` insert at all 13 transit points in `decision_system` (was missing — arrival detection required it)
  - single targeting path: `decision_system` writes `GpuUpdate::SetTarget` + inserts `HasTarget` → `gpu_position_readback` detects arrival → `AtDestination`
  - fixes guard-patrol Phase 3, farmer-cycle Phase 5, raider-cycle Phase 2, combat Phase 5, projectiles Phase 3
  - 4 previously-failing tests now fully pass (farmer-cycle, raider-cycle, combat, projectiles)


- **world grid + procedural generation (TDD)**
  - add `WorldGrid` resource: 250x250 cell grid (32px/cell) covering 8000x8000 world
  - add `WorldCell` with `Biome` (Grass/Forest/Water/Rock/Dirt) + `Option<Building>` layers
  - add `Building` enum: Fountain, Farm, Bed, GuardPost, Camp (each with town_idx)
  - add `WorldGenConfig` resource with defaults (2 towns, 1200px spacing, 1100px camp distance)
  - add `generate_world()` pure function: random town placement, camp positioning, simplex terrain, building layout
  - add terrain generation via `noise` crate (simplex noise, Dirt override near settlements)
  - add `world_gen.rs` test (6 phases): grid dimensions, town count, distances, buildings, terrain, camps
  - add `rand` 0.9 and `noise` 0.9 dependencies
  - all 6 test phases pass

- **stage 5: write 10 test suites**
  - add `spawning.rs`: spawn 5 NPCs, kill one, verify slot freed and reused (4 phases)
  - add `energy.rs`: energy starts at 100, drains, reaches ENERGY_HUNGRY (3 phases, time_scale=50)
  - add `movement.rs`: HasTarget set, GPU positions update, AtDestination on arrival (3 phases)
  - add `guard_patrol.rs`: OnDuty → Patrolling → OnDuty → rest → resume (5 phases, time_scale=20)
  - add `farmer_cycle.rs`: GoingToWork → Working → tired → rest → return (5 phases, time_scale=20)
  - add `raider_cycle.rs`: dispatch group → steal food → return → deliver (5 phases, time_scale=20)
  - add `combat.rs`: GPU targeting → InCombat → damage → death → slot freed (6 phases)
  - add `projectiles.rs`: ranged targeting → projectile spawn → hit → slot freed (4 phases)
  - add `healing.rs`: damaged NPC near town → Healing marker → health recovers (3 phases, time_scale=20)
  - add `economy.rs`: farm growth → harvest → camp forage → raider respawn (5 phases, time_scale=50)
  - add `Debug` derive to `FarmGrowthState` (needed for test display)

- **test framework (stage 5)**
  - add UI-selectable test menu via bevy_egui (`EguiPrimaryContextPass` schedule)
  - add `AppState` (TestMenu | Running) — all game systems gated on `in_state(Running)`
  - add `TestState`, `TestRegistry`, `RunAllState` resources for test lifecycle
  - add `test_is("name")` run condition for per-test system gating
  - add test HUD: phase checklist overlay with `○` pending, `▶` active, `✓` passed, `✗` failed
  - add Run All: sequential test execution with summary on completion
  - add `CleanupCore`/`CleanupExtra` SystemParam bundles (avoids 16-param limit)
  - add `cleanup_test_world` (OnExit Running): despawn NPC entities, reset all resources
  - relocate Test12 to `tests/vertical_slice.rs` (8 phases, all passing in ~4s)
  - remove `Test12` resource from `resources.rs`

- **multi-layer equipment rendering**
  - add EquipLayer enum (Armor, Helmet, Weapon, Item) + EquippedWeapon/Helmet/Armor components (components.rs)
  - add EQUIP_SWORD, EQUIP_HELMET, FOOD_SPRITE sprite constants (constants.rs)
  - add SetEquipSprite GpuUpdate variant, remove SetCarriedItem (messages.rs)
  - add 4 equipment sprite Vec fields to NpcBufferWrites (stride 2, -1.0 sentinel), route by EquipLayer in apply() (gpu.rs)
  - refactor NpcRenderBuffers: single instance_buffer → Vec<LayerBuffer> with 5 layers (npc_render.rs)
  - DrawNpcs draws all non-empty layers sequentially (body → armor → helmet → weapon → item)
  - npc_render.wgsl: equipment layers (health >= 0.99) discard bottom pixels to preserve health bar
  - spawn clears all equipment, then sets job-specific gear: guards get sword+helmet, raiders get sword
  - death_cleanup clears all 4 equipment layers on death (prevents stale slot data)
  - behavior.rs: SetCarriedItem → SetEquipSprite(Item) for food carry/deliver
  - test 12 passes

- **damage flash (white overlay on hit, fade out)**
  - add `SetDamageFlash { idx, intensity }` to GpuUpdate enum (messages.rs)
  - add `flash_values: Vec<f32>` to NpcBufferWrites, handle in apply(), decay at 5.0/s in populate_buffer_writes (gpu.rs)
  - damage_system sends SetDamageFlash(1.0) after SetHealth (health.rs)
  - add `flash: f32` to NpcInstanceData (36 → 40 bytes), vertex attribute @location(6) (npc_render.rs)
  - npc_render.wgsl: add flash to vertex I/O, mix(color, white, flash) in fragment shader
  - test 12 passes

- **health bars + projectile sprite**
  - add `health: f32` to NpcInstanceData (32 → 36 bytes), vertex attribute @location(5)
  - prepare_npc_buffers reads NpcBufferWrites.healths, normalizes by /100.0
  - prepare_proj_buffers sets health=1.0 (no bar on projectiles)
  - npc_render.wgsl: pass quad_uv + health through vertex shader, 3-color health bar in bottom 15% of sprite (green >50%, yellow >25%, red ≤25%), show-when-damaged mode
  - change projectile sprite from (7, 0) to (20, 7)
  - test 12 passes (2.3s)

## 2026-02-08
- **projectile instanced rendering**
  - add projectile position readback: COPY_SRC on proj_positions buffer, position_staging buffer, copy in ProjectileComputeNode, PROJ_POSITION_STATE static
  - merge readback_proj_hits into readback_proj_data: single device.poll() reads both hits and positions
  - add ProjBatch marker, ProjRenderBuffers resource, DrawProjs RenderCommand, DrawProjCommands type
  - add spawn_proj_batch, extract_proj_batch, prepare_proj_buffers, queue_projs systems in NpcRenderPlugin
  - projectiles reuse NPC pipeline, shader, quad geometry, texture and camera bind groups
  - faction-colored: blue (0.4, 0.6, 1.0) for villagers, red (1.0, 0.3, 0.2) for raiders
  - sort_key=1.0 renders projectiles above NPCs (0.0)
  - test 12 passes (2.3s)

- **camera controls: WASD pan + scroll zoom + click-to-select**
  - npc_render.wgsl: replace hardcoded CAMERA_POS/VIEWPORT with `Camera` uniform struct at @group(1) @binding(0)
  - render.rs: add CameraState resource (ExtractResource), 5 camera systems (pan, zoom, viewport sync, transform sync, click select)
  - npc_render.rs: add CameraUniform (ShaderType), NpcCameraBindGroup, SetNpcCameraBindGroup RenderCommand, prepare_npc_camera_bind_group system
  - pipeline layout: 2 bind groups (texture at 0, camera at 1)
  - pan speed 400px/s scaled by 1/zoom, scroll zoom factor 0.1 toward mouse cursor (range 0.1–4.0)
  - click-to-select: screen-to-world via CameraState, nearest NPC within 20px from GPU_READ_STATE
  - delete old .glsl shaders and .zip archive (cleanup)
  - test 12 passes (2.3s)

- **port separation physics from glsl to wgsl compute shader**
  - boids-style separation force: 3x3 grid neighbor scan, asymmetric push (moving=0.2x, settled=2.0x), golden angle for exact overlap
  - TCP-style dodge: perpendicular avoidance for converging NPCs (head-on/crossing/overtaking), consistent side-picking via index comparison
  - backoff persistence: blocked NPCs slow down exponentially (persistence = 1/(1+backoff)), blocking detection via push/goal alignment
  - backoff buffer (binding 6) now read/written by shader — was allocated but unused
  - all params now active: separation_radius, separation_strength, grid_width/height, cell_size, max_per_cell
  - combat targeting unchanged (wider search radius than GLSL version)
  - test 12 still passes (5.3s, down from 6.8s)

## 2026-02-08
- **test 12: vertical slice integration test — phase 4 complete**
  - add Test12 resource with phased assertions (8 phases, time-gated, PASS/FAIL logging)
  - test12_setup startup: populate WorldData (2 towns, 5 farms, 5 beds, 4 guard posts), spawn 12 NPCs, init FoodStorage/FarmStates
  - test12_tick: validates spawn → GPU readback → farmers working → raiders raiding → combat → death → respawn
  - all 8 phases pass in 6.8s at time_scale=10
  - fix: add CPU-side arrival detection in gpu_position_readback (position vs goal within ARRIVAL_THRESHOLD → AtDestination)
  - fix: add HasTarget component to farmers at spawn (was missing, blocking arrival detection)
  - ARRIVAL_QUEUE static is now unused — replaced by CPU-side arrival detection in movement.rs

- **docs: authority model and roadmap restructure**
  - messages.md: Data Ownership → Data Ownership & Authority Model with 4 categories (GPU-authoritative, CPU-authoritative, CPU-only, render-only)
  - added staleness budget (1 frame, 1.6px drift) and anti-pattern rule (no read-then-write feedback loops)
  - fixed stale entries: GPU_READ_STATE now documented as populated, removed "not yet ported" references
  - roadmap: restructured phases 4-7 from infrastructure milestones to gameplay-driven milestones (core loop → visual feedback → playable game → content)
  - roadmap: added multi-layer equipment rendering spec with data model, implementation steps, and performance budget
  - roadmap: added maintenance guide (phases = priority, capabilities = backlog)
  - behavior.md: removed strikethrough fixed-bug (changelog material)
  - gpu-compute.md: "not ported from old GLSL shader" → "not yet implemented"
  - economy.md, rendering.md: created from behavior.md split + new render pipeline docs

- **wire combat projectiles end-to-end**
  - npc_compute.wgsl: 3-mode spatial grid (mode 0 clear, mode 1 build with atomicAdd, mode 2 movement + combat targeting)
  - multi-dispatch NpcComputeNode: 3 bind groups with different mode uniform, 3 dispatches per frame
  - combat targeting via grid neighbor search: finds nearest enemy within combat_range (300px), ~6 cell search radius
  - combat_target_staging buffer: dual readback (positions + combat_targets) in single device.poll()
  - attack_system: reads GPU combat_targets, fires projectiles via PROJ_GPU_UPDATE_QUEUE or applies point-blank damage
  - projectile hit readback: hit_staging buffer, readback_proj_hits system, PROJ_HIT_STATE static
  - process_proj_hits: converts hits to DamageMsg, recycles projectile slots
  - arrival flag reset: SetTarget resets arrivals[idx]=0 so GPU resumes movement toward chase targets
  - ProjBufferWrites default dirty=true for first-frame -1 hit initialization
  - Deactivate also resets hits buffer to -1 to prevent re-triggers
  - test_spawn_combat: 5v5 faction fighters for combat pipeline verification
  - debug_tick_system: F2 combat logging shows targets/attacks/chases/deaths per second
  - verified: NPCs find targets (9/10), chase out-of-range enemies, point-blank damage reduces NPC count

- **gpu→ecs position readback + debug flags**
  - add staging buffer (MAP_READ | COPY_DST) to NpcGpuBuffers for position readback
  - copy positions to staging after compute dispatch in NpcComputeNode
  - add readback_npc_positions system (Cleanup phase): map staging, write to GPU_READ_STATE
  - register existing sync_gpu_state_to_bevy (Step::Drain) and gpu_position_readback (after Drain)
  - add per-field dirty flags to NpcBufferWrites: prevents write_npc_buffers from overwriting GPU-computed positions
  - npc_render.rs reads GPU_READ_STATE for positions (falls back to NpcBufferWrites on first frame)
  - add DebugFlags resource with F1-F4 keyboard toggles (readback, combat, spawns, behavior)
  - add wgpu 27 dependency for MapMode and PollType
  - verified: NPCs move from spawn y=300 to work y=257 via GPU compute

- **port projectile_compute.glsl to wgsl + wire into bevy render graph**
  - create shaders/projectile_compute.wgsl: lifetime, movement, spatial grid collision detection
  - add ProjGpuUpdate enum and PROJ_GPU_UPDATE_QUEUE to messages.rs
  - add projectile compute pipeline to gpu.rs: ProjGpuBuffers (8 buffers), ProjComputeParams, ProjBufferWrites, ProjectileComputeNode
  - render graph: NpcCompute → ProjectileCompute → CameraDriver (grid built before projectile reads it)
  - projectile bind group shares NPC buffers (positions, factions, healths, grid) as read-only
  - pipeline compiles and dispatches; no-op with proj_count=0

- **restructure: flatten project layout**
  - flatten gpu/mod.rs → gpu.rs, render/mod.rs → render.rs (single-file modules don't need folders)
  - move shaders from assets/shaders/ to root shaders/ (all shaders in one place)
  - delete Godot .import files from assets/
  - change asset root to project root (asset_server loads from ".." instead of "../assets")
  - update texture load paths with assets/ prefix

- **cleanup: remove dead render pipeline code from gpu/mod.rs**
  - delete old render graph Node approach (~450 lines): NpcRenderNode, NpcRenderPipeline, NpcRenderBindGroups, NpcSpriteTextureBindGroup, init_npc_render_pipeline, prepare_npc_texture_bind_group, prepare_npc_render_bind_groups
  - delete sprite_indices/colors fields from NpcGpuBuffers (npc_render.rs uses its own vertex buffer)
  - delete quad mesh creation (NpcRenderMesh) from init_npc_compute_pipeline
  - delete unused spawn_test_sprites from render/mod.rs
  - clean up unused imports (TrackedRenderPass, ViewTarget, GpuImage, RenderAssets, old binding types)
  - zero warnings, zero errors

- **phase 2.5: gpu instanced npc rendering working**
  - fix npc_render.rs for Bevy 0.18 API: BindGroupLayoutDescriptor, Option<Cow> entry points
  - add depth-stencil state (Depth32Float) matching Transparent2d render pass
  - add dynamic MSAA sample count from camera Msaa component
  - fix bind group layout: remove unused mesh2d view bind group, texture at group 0
  - update npc_render.wgsl: texture bindings moved to @group(0)
  - disable old render graph Node pipeline in gpu/mod.rs (replaced by RenderCommand pattern)
  - 5 test NPCs visible as green squares via instanced draw call
  - enable sprite texture sampling in fragment shader (was solid color debug)
  - fix UV flip: remove incorrect 1.0-v inversion (wgpu uses top-left origin)
  - fix instance count: use NpcGpuData.npc_count instead of buffer length (was rendering all MAX_NPCS slots)

- **pure bevy migration phase 2-3**: wire ECS systems, add GPU compute and sprite rendering
  - wire build_app() into main.rs, verify systems tick with debug logging
  - add gpu/mod.rs: GPU compute via Bevy render graph (follows game_of_life pattern)
  - add assets/shaders/npc_compute.wgsl: simplified WGSL shader for movement
  - add render/mod.rs: 2D camera, texture atlas loading, sprite rendering
  - configure AssetPlugin to load from ../assets
  - test sprites rendering correctly (8 character sprites visible)
  - GPU compute pipeline compiles and dispatches (no data flow yet)
  - update /endless, /test, /debug skills for pure Bevy workflow

- **pure bevy migration phase 1**: convert from godot+bevy hybrid to standalone bevy app
  - remove godot dependencies: api.rs, channels.rs, rendering.rs, gpu.rs deleted (~2,000 lines)
  - keep pure bevy ECS: components.rs, resources.rs, systems/*.rs (~2,500 lines)
  - update Cargo.toml: bevy 0.18, bevy_egui 0.39, remove godot deps
  - add main.rs: bevy App entry point with window creation
  - update lib.rs: strip to build_app() + helpers only
  - migrate godot-bevy patterns to pure bevy:
    - `#[derive(Event)]` → `#[derive(Message)]` (bevy 0.17+ terminology)
    - `EventReader/Writer` → `MessageReader/Writer`
    - `Vector2` → `Vec2`
    - `PhysicsDelta` → bevy `Time` resource
  - remove BevyToGodot channel (projectile firing stubbed for phase 3)
  - window opens successfully with vulkan backend
  - systems compile but not yet wired (phase 2: GPU compute)
- update docs/README.md: reflect new pure bevy architecture

- refactor lib.rs: split into modules (2887 → 2121 lines)
  - api.rs: UI query methods with #[godot_api(secondary)] (518 lines)
  - rendering.rs: MultiMesh setup methods (283 lines)
- add town/camp info labels in main.gd showing population counts
- change raid balance: max 5 raiders per camp (was 10), 3 per raid group (was 5)
- change start menu defaults: 1 town, 10 guards per town

- add game loop phase 1: raider economy
  - camp_forage_system: camps gain 1 food/hour passive income
  - raider_respawn_system: camps spawn raiders for 5 food each (max 10 per camp)
  - hour_ticked flag in GameTime for clean hourly event triggering
- add game loop phase 2: starvation system
  - LastAteHour component tracks when NPC last ate
  - Starving marker after 24 hours without eating
  - starvation_system: adds/removes Starving, updates speed to 75%
  - healing_system: HP capped at 50% for starving NPCs
  - decision_system Eat action: updates LastAteHour, removes Starving
- add game loop phase 3: group raids
  - RaidQueue resource: simple waiting queue per faction
  - raiders join queue when picking Work, dispatch all when 5+ waiting
  - no separate coordinator system — queue checked inline in decision_system
  - transit skip now includes Raiding and Returning (no mid-journey decisions)
- add constants: CAMP_FORAGE_RATE, RAIDER_SPAWN_COST, CAMP_MAX_POP, RAID_GROUP_SIZE,
  STARVATION_HOURS, STARVING_HP_CAP, STARVING_SPEED_MULT

## 2026-02-07
- fix fountain offset: position at grid slot (0,0) instead of geometric center
  - gold ring also centered on fountain position
- simplify bed system: 1 bed per slot instead of 4 in 2x2 arrangement
  - remove bed offset calculations, place at slot center
  - build menu closes after placing bed (consistent with other buildings)
- fix FIXED_SLOTS: only fountain (0,0) is fixed, other slots checked by contents
- add building removal system: runtime add/remove of farms, beds, guard posts
  - add remove_location(type, x, y) API with NPC eviction
  - change AssignedFarm from usize index to Vector2 position (survives deletion)
  - change FarmOccupancy/BedOccupancy from Vec to HashMap<(i32,i32), i32>
  - add pos_to_key() helper for position → grid key conversion
  - guard patrol routes rebuild on add/remove (clockwise around town center)
  - evicted NPCs become idle (Working/Resting cleared, re-enter decision_system)
- fix build_menu: use npc_manager.get_town_food() instead of dead town_food array
- refactor behavior.rs: central brain architecture with SystemParam bundles
  - add FarmParams, EconomyParams, CombatParams, NpcStateParams bundles
  - consolidate 19 parameters into 4 logical groups (stays under Bevy's 16-param limit)
  - add Priority 0 arrival handling in decision_system (AtDestination marker)
  - remove redundant Priority 8 (Raiding re-target every frame) - now handled in Priority 0
  - arrival_system now minimal: marks AtDestination + proximity delivery only
- fix working farmer harvest: farmers now harvest when assigned farm becomes Ready
  - previously only harvested on arrival, not while already working
  - harvest check added to drift check loop (throttled every 30 frames)
- energy system now uses game time instead of frame-based rates
  - 6 game hours to recover from 0→100 while resting
  - 24 game hours to drain from 100→0 while active
  - respects time_scale and pause
- remove passive food production from economy_tick_system
  - food now only comes from harvesting Ready farms
- fix food storage: upgrade_menu and main.gd now use ECS API
  - npc_manager.get_town_food() and add_town_food() instead of main.town_food[]
- fix NPC inspector: HP and Energy now read from Bevy components
  - both use consistent pattern via entity lookup
  - removed redundant NpcEnergyCache sync
- add farm progress bar: visual indicator showing crop growth
  - item_icon.gdshader samples food sprite (24,9) from roguelikeSheet
  - progress bar at top (like NPC HP bar): green growing, gold ready
  - separate item_canvas_item with z=10 (above NPCs)
  - 16-float buffer: Transform2D + Color + CustomData for progress
- add Godot path and version to CLAUDE.md lessons learned
- unify decision system: consolidate 5 systems into one priority cascade
  - flee_system, leash_system, patrol_system, recovery_system → decision_system
  - priority order: flee > leash > combat > recovery > tired > patrol > wake > raid > idle
  - add on_duty_tick_system for guard tick counting
  - simplify energy_system to drain/recovery only (no state transitions)
  - eliminates scattered decision-making and command sync race conditions
- implement eat action: consume food from town storage, restore 30 energy instantly
  - no travel required, NPCs eat at current location
  - only scored when town actually has food (prevents stuck eat loops)
- fix rest loop: move wake-up from energy_system to decision_system
  - avoids Bevy command sync race where Resting removal wasn't visible to decision_system
  - farmers now properly wake at 90% energy and pick Work instead of Rest
- fix farmer arrival: use WorkPosition instead of current position for farm lookup
  - prevents "Working (no farm)" when separation forces push farmer away
- tighten farmer placement: ARRIVAL_THRESHOLD 40→20px, MAX_DRIFT 50→20px
  - farmers stay visually on the farm sprite
- fix build.rs: always rerun so DLL timestamp updates every build
- add ENERGY_FROM_EATING (30.0) and ENERGY_TIRED_THRESHOLD (30.0) constants
- slim down CLAUDE.md: move reference material to docs/README.md

## 2026-02-02
- add AssignedFarm component for farm occupancy tracking
  - farmers get AssignedFarm(farm_idx) when entering Working state
  - reserves farm (FarmOccupancy.occupant_count++)
  - target set to farm position so farmers return if pushed away
- add working farmer drift check (throttled every 30 frames)
  - re-targets farmers who drifted >50px from assigned farm
- energy_system removes Working when energy < 30%
  - releases farm, removes AssignedFarm
  - farmer then enters decision_system and can choose to rest
- death_cleanup_system releases farm if dead NPC had AssignedFarm
- fix food storage: read from Bevy resource instead of dead static
- fix item MultiMesh buffer size mismatch (was using farm_data.len() instead of MAX_FARMS)

## 2026-02-01
- add farm growth system: farms have growing → ready cycle
  - FarmStates resource tracks Growing/Ready state and progress per farm
  - farm_growth_system advances progress based on game time
  - hybrid growth: passive 0.08/hour (~12h), tended 0.25/hour (~4h with farmer)
  - farmers harvest ready farms on arrival (adds food, resets growth)
  - raiders can only steal from ready farms (otherwise seek another)
  - ready farms show yellow food icon via build_item_multimesh()
- add find_location_within_radius() to world.rs
  - returns (index, position) for locations within radius
  - find_nearest_location() now wraps it for backward compatibility
  - used by arrival_system for clean farm harvest/steal logic
- add MAX_FARMS constant (500), item MultiMesh allocates extra slots
- fix farm data stutter: cache BevyApp reference in ready()
  - add bevy_app_cache field to EcsNpcManager
  - add get_bevy_app_cached() for hot paths (process())
  - eliminates 60 scene tree traversals per second
- remove dead code: FREE_SLOTS and NPC_SLOT_COUNTER statics
  - SlotAllocator Bevy resource replaced these
- update messages.md: fix outdated UI query state section
  - doc claimed static Mutexes but code uses Bevy Resources
  - correct architecture table counts
- track deaths by job: show farmer/guard/raider deaths separately in UI
  - add dead field to PopStats (tracks by job + town)
  - add pop_inc_dead() helper, call in death_cleanup_system
  - expose farmers_dead, guards_dead, raiders_dead in get_population_stats()
  - left_panel.gd now shows guard deaths instead of "-"
- fix raider healing: unify settlements as towns with faction
  - add "town_center" location type replacing "fountain" and "camp"
  - Town struct now has sprite_type field (0=fountain, 1=tent)
  - remove Camp struct and add_town() - all settlements are towns
  - raiders now heal at their camps (same-faction town center)
  - add healing debug stats: healing_in_zone_count, healing_healed_count, healing_towns_count
- fix location MultiMesh rebuild spam: remove per-add rebuild, use build_locations() once
- optimize GPU buffer updates: batch uploads reduce ~670 → ~8 buffer_update() calls/frame
  - add DirtyRange tracking for each buffer type
  - drain loop updates CPU caches, then batch uploads dirty ranges
  - add upload_*_range() methods to GpuCompute for each buffer
  - add CPU caches for arrivals, backoffs, speeds
- remove dead code: sprite_frame_buffer never read by shader
  - remove binding 12 from shader and Rust uniform set
  - keep CPU cache for multimesh builder
- remove GPU multimesh writes (already done earlier, was dead code)
- fix color.a state hack: use arrival flag instead of overloaded alpha
  - shader now checks `settled == 0` instead of `color.a > 0.0`
  - color buffer is now purely visual (faction tinting)

## 2026-01-31
- move locations to ECS: eliminate all Location nodes (~260 nodes → 0)
  - add unified add_location(type, x, y, town_idx, opts) API
  - types: "farm", "bed", "guard_post", "camp", "fountain"
  - add build_locations() to build/rebuild location MultiMesh
  - sprite definitions moved to Rust (world.rs): SPRITE_FARM, SPRITE_TENT, etc.
  - main.gd stores positions instead of node references
  - delete location.gd, location.tscn, location_renderer.gd, location_renderer.tscn
- reduce location node overhead: 1117 → 657 nodes (~460 removed)
  - location.tscn: removed CollisionShape2D, changed Area2D → Node2D
  - location.gd: queue_free() labels for farms/beds/posts instead of hiding
  - add get_location_at_position() API for click selection without Area2D collision
- fix godot_ms timing: use previous frame's ECS time for accurate measurement
  - frame_ms spans process-to-process, was wrongly subtracting current frame's ECS
  - add prev_ecs_total_ms field to PerfStats for correct calculation
- add render time profiling: RenderingServer.viewport_get_measured_render_time_cpu/gpu
  - shows actual Godot rendering overhead in perf stats
  - cleaned up misleading Performance.TIME_PROCESS display
- optimize FFI calls: get_selected_npc() returns {idx, position, target} in single call
  - TargetOverlay uses cached values instead of re-fetching (eliminates 4 FFI calls/frame)
  - reduces 7 FFI calls to 1 when NPC selected
- change start menu defaults: 4 guards per town, 3 raiders per camp
- fix raider food delivery at wrong location after combat
  - event-based Returning arrival now re-targets home instead of delivering
  - only proximity check (within 150px of home) delivers food
  - prevents raiders delivering food at guard's last position after combat chase
- add debug stats caching: arrival/backoff stats computed during main sync
  - get_debug_stats() no longer does extra GPU reads
  - stats stored in PERF_STATS for cheap retrieval
- add UI throttling optimizations
  - left_panel, farm_menu, guard_post_menu: update every 30 frames (was 10 or every frame)
  - _set_text() helper skips label updates when text unchanged (avoids layout recalc)
  - Detail OFF mode skips all Rust calls (just shows FPS/zoom)
- add physics optimization: disable Area2D monitoring on locations
- add vsync=0 default in project.godot
- add GPU-side spatial grid building: eliminates 3MB CPU→GPU upload per frame
  - mode 0: clear grid counts (one thread per cell)
  - mode 1: insert NPCs via atomicAdd (one thread per NPC)
  - mode 2: main NPC logic (existing code)
  - 3 dispatches with barriers, single compute list
  - ~30% faster at 9K NPCs (58→75 FPS)
- add NPC decision logging: 100-entry circular buffer per NPC with timestamps
  - decisions logged as "Action (e:XX h:XX)" showing energy and health
  - state transitions logged ("→ OnDuty", "→ Resting", "Stole food → Returning")
- add scrollable log display in left panel inspector
- add DLL build time on start menu for version verification
- add performance profiling: get_perf_stats() API with timing breakdown
  - queue_ms, dispatch_ms, readpos_ms, combat_ms, build_ms, upload_ms
  - bevy_ms for Bevy ECS systems timing
  - displayed in left panel with other stats
- fix HP-based work score: NPCs below 50% HP won't work/raid
  - score scales from 0 at 50% to full at 100% HP
  - applies to all jobs (prevents wounded raiders from raiding)
- fix recovery_system: Resting no longer required to exit Recovering
  - NPCs that lost Resting marker were stuck in Recovering forever

## 2026-01-30
- add FactionStats resource: per-faction alive/dead/kills tracking
  - init_faction_stats(), get_faction_stats(), get_all_faction_stats() API
  - inc_alive() on spawn, dec_alive()/inc_dead() on death
  - left_panel.gd shows aggregated raider dead counts
- add multi-faction support: Faction changed from enum to struct(i32)
  - faction 0 = villagers (all towns share)
  - faction 1+ = each raider camp is unique faction
  - raiders fight each other (GPU targeting uses != comparison)
- add raider faction colors: 10-color palette cycles per faction
- add CarriedItem component for visual item display above NPC heads
  - separate MultiMesh layer for carried items (O(1) draw calls)
  - SetCarriedItem GPU update message
- fix proximity-based arrival for Returning and GoingToRest
  - uses DELIVERY_RADIUS (150px, same as healing aura)
  - fixes raiders/resting NPCs getting stuck waiting for exact arrival
- fix raiders deliver to their own camp (not hardcoded camp 0)
- change game defaults to stress test mode: 10 towns, 50 guards, 35 raiders
- change start menu: stress test mode enabled by default

## 2026-01-30
- add Wandering state marker (fixes NPCs showing "Idle" while walking to wander target)
  - add Wandering component to components.rs
  - decision_system now inserts Wandering marker when choosing Action::Wander
  - arrival_system clears Wandering on arrival (back to decision_system)
  - derive_npc_state() returns "Wandering" for wandering NPCs
- refactor: unified slot allocator in Bevy (fixes zombie NPCs)
  - spawn and death now both use SlotAllocator bevy resource
  - removed static FREE_SLOTS and NPC_SLOT_COUNTER
  - get_npc_count() now reads from SlotAllocator.count()
  - fixes slot recycling mismatch that caused zombie NPCs
- add debug info to get_population_stats() for diagnosing UI count issues
  - returns _debug_towns, _debug_cache_total, _debug_health_len
- fix: dead NPC slots now fully cleaned up (prevents zombie NPCs walking from -9999,-9999)
  - HideNpc now resets position, target, arrival, and health (was only position)
  - fixes slot recycling bug where new NPCs would walk from death position
- fix: NPC click selection uses slot counter instead of dispatch count (timing issue)
  - get_npc_at_position, get_npc_position, get_npc_health now use NPC_SLOT_COUNTER
  - fixes NPCs becoming unclickable after some time
- fix: skip drawing target line for dead selected NPCs (was drawing to -9999,-9999)
- change: ARRIVAL_THRESHOLD increased from 8px to 40px (easier food drop-off)
- fix: NPCs now wake from Resting when energy reaches 90% (was stuck forever)
- add healing aura: NPCs heal 5 HP/sec when near own faction's town center (150px radius)
  - add MaxHealth component for per-NPC health cap (supports future leveling)
  - add Healing marker component for visual state tracking
  - add healing_system in health.rs, GpuUpdate::SetHealing message
  - shader halo effect not working yet (healing logic works)
- add raid continuation: raiders keep Raiding marker after combat, decision_system re-targets nearest farm
- add find_nearest_location(pos, world, LocationKind) generic helper in world.rs
- delete all legacy gdscript npc systems (rust ECS is now single source of truth)
  - npc_manager.gd, npc_manager.tscn, npc_navigation.gd
  - npc_combat.gd, npc_needs.gd, npc_grid.gd, npc_renderer.gd
  - gpu_separation.gd, separation_compute.glsl
  - guard_post_combat.gd, projectile_manager.gd
- remove .uid files from git tracking (auto-generated by godot)
- update roadmap: add missing features from deleted files
  - trait combat/flee modifiers, target switching
  - guard post auto-attack, town policies, icon overlays
  - food consumption, hp regen, healing upgrades
- clean up readme: move all feature tracking to docs/roadmap.md
  - readme now intro only (gameplay, controls, credits)
  - reorder: The Struggle and Controls before Inspirations
  - add proper credits for godot, bevy, godot-bevy
- rename "Rust Migration Roadmap" to "Roadmap" (migration complete)
- update CLAUDE.md: remove obsolete gdscript sections
- update docs/README.md: add guard_post_menu.gd, terrain_sprite.gdshader to file map
- remove icon.svg, tmp files, add *.tmp to gitignore
- refactor: rust returns state/job/trait as strings instead of integers
- add derive_npc_state() returns "Idle", "Fighting", "On Duty", etc.
- add job_name() returns "Farmer", "Guard", "Raider", "Fighter"
- add trait_name() returns "Brave", "Coward", "Efficient", etc.
- update get_npc_info() and get_npcs_by_town(): state/job/trait now strings
- update left_panel.gd, roster_panel.gd: use strings directly from rust
- update combat_log.gd: remove NPCState dependency
- delete systems/npc_state.gd: all NPC data now sourced from rust ECS
- refactor: consolidate arrival handlers into single generic arrival_system
- delete handle_arrival_system, raider_arrival_system, wounded_rest_system
- arrival_system transitions based on state markers (component-driven, not job-driven)

## 2026-01-29
- docs: reorganize roadmap from phases to capabilities
- docs: move Data Ownership table to messages.md
- docs: move Key Optimizations and Performance Lessons to gpu-compute.md
- docs: add Testing & Debug capability section
- add Phase 11.7: replace 5 static queues with lock-free crossbeam channels
- add GodotToBevy channel (spawn, target, damage, reset, pause, timescale)
- add BevyToGodot channel (projectile fire, future sync messages)
- add godot_to_bevy_read system: drains channel, dispatches to Bevy messages/resources
- remove SPAWN_QUEUE, TARGET_QUEUE, DAMAGE_QUEUE, PROJECTILE_FIRE_QUEUE, RESET_BEVY statics
- remove drain_spawn_queue, drain_target_queue, drain_damage_queue systems
- update lib.rs spawn_npc, set_target, apply_damage: send via channel
- update lib.rs process(): drain BevyToGodot for FireProjectile messages
- add ResetFlag resource (replaces RESET_BEVY static)
- pattern: crossbeam channels for cross-thread, statics only at lib.rs boundary
- add Phase 10.2: GPU update messages replace direct Mutex locks
- add GpuUpdateMsg message type (wraps GpuUpdate enum)
- add collect_gpu_updates system: drains messages, single Mutex lock at end of frame
- update spawn_npc_system: uses MessageWriter<GpuUpdateMsg>
- update behavior systems (8 systems): use MessageWriter<GpuUpdateMsg>
- update attack_system: uses MessageWriter<GpuUpdateMsg>
- pattern: godot-bevy Messages for high-frequency batch operations (not Observers)

## 2026-01-28
- add Time API: get_game_time(), set_time_scale(), set_paused() via BevyApp world access
- add get_npc_log() API: per-NPC activity log with timestamps
- add get_bevy_app() helper: fetches BevyApp autoload for direct Bevy resource access
- delete world_clock.gd: time now handled by Bevy GameTime resource
- update main.gd, left_panel.gd, combat_log.gd to use ECS time API
- add CombatOrigin component: stores position where combat started
- fix leash_system: now measures distance from combat origin, not home (allows raiders to travel far for raids without premature leashing)
- fix flee_system and leash_system: clear Raiding marker when disengaging (prevents stuck state after returning home)
- fix raider arrival bug: verify raider is within 100px of farm before food pickup (prevents stale arrival events from spawn/home triggering false pickups)
- fix fake arrival bug: blocked NPCs no longer trigger arrival, cap backoff at 200 instead of giving up
- add get_npc_target() API: returns NPC movement target from cached GPU data
- add target visualization: selected NPC shows cyan line to target + magenta crosshair marker
- add targets cache in GpuState for CPU-side target tracking
- delete raider_idle_system: dead code now absorbed into npc_decision_system
- disable autospawning: comment out check_respawns call (code kept for later)
- remove console spam from economy_tick_system and produce_food
- add utility AI: weighted random decision system replaces priority cascades
- add Personality component: 0-2 traits (Brave, Tough, Swift, Focused) with magnitude 0.5-1.5
- add TraitKind, TraitInstance types for trait data
- add npc_decision_system: scores Eat/Rest/Work/Wander actions, weighted random selection
- remove tired_system, resume_patrol_system, resume_work_system, raider_idle_system (absorbed into npc_decision_system)
- add action score constants (SCORE_FIGHT_BASE, SCORE_WORK_BASE, SCORE_WANDER_BASE, etc.)
- change start menu sliders: now per-town values instead of totals (2 farmers, 4 guards, 6 raiders default)
- update config.gd defaults to match (BASE_FARMERS=2, BASE_GUARDS=4, BASE_RAIDERS=6)
- add Phase 9.4: UI data queries (10 new APIs for population stats, NPC info, roster, selection)
- add unified Town model: all settlements are "towns" with faction field (0=Villager, 1=Raider)
- add NPC_META static: per-NPC name/level/xp/trait cached for UI queries
- add NPC_STATES static: per-NPC state ID updated by behavior systems
- add NPC_ENERGY static: per-NPC energy synced from Bevy
- add KILL_STATS static: tracks guard/villager kills for UI display
- add SELECTED_NPC static: currently selected NPC index for inspector
- add NPCS_BY_TOWN static: per-town NPC lists for O(1) roster queries
- add name generation: "Adjective Noun" names based on job (Swift Tiller, Brave Shield, etc.)
- add get_population_stats(), get_town_population(), get_npc_info(), get_npcs_by_town() APIs
- add get/set_selected_npc(), get_npc_name(), get_npc_trait(), set_npc_name(), get_bed_stats() APIs
- add get_npc_at_position(x, y, radius) API for click selection
- add NPC click selection in main.gd: left-click selects nearest NPC within 20px
- fix sprite rendering: store ShaderMaterial reference to prevent garbage collection, include custom_data in build_multimesh_from_cache
- update left_panel.gd: uses ECS APIs for stats, bed info, and NPC inspector
- update roster_panel.gd: uses ECS APIs for NPC roster with sorting/filtering
- update upgrade_menu.gd: uses ECS APIs for farmer/guard counts
- rename Clan component to TownId for clarity
- fix deprecated VariantArray → VarArray in lib.rs
- refactor UI to ECS-only: remove _uses_methods dual-mode code from all UI panels
- add ECS API NEEDED comments documenting required ECS API for each UI feature
- preserve old GDScript code as comments for future porting reference
- fix UI compatibility with EcsNpcManager: all UI panels now detect ECS mode and gracefully degrade
- add get_npc_health() API to EcsNpcManager for UI health queries
- add npc_manager group registration so UI can find EcsNpcManager via get_first_node_in_group()
- guard main.gd building management code to skip GDScript-only operations in ECS mode
- add Phase 9.2: food production and respawning in Bevy ECS
- add economy_tick_system: unified hourly economy (food + respawn) using PhysicsDelta
- add Clan component: universal town/camp identifier on every NPC
- add GameTime resource: Bevy-owned game time tracking (no GDScript bridge needed)
- add GameConfig resource: farmers/guards per town, spawn interval, food per hour
- add PopulationStats resource: tracks alive/working counts per (job, clan)
- add RespawnTimers resource: per-clan respawn cooldowns
- remove FRAME_DELTA static: all timing now uses godot-bevy's PhysicsDelta (Godot-synced)
- remove GAME_TIME static: game time fully owned by Bevy
- refactor cooldown_system to use PhysicsDelta instead of FRAME_DELTA
- add Animal Crossing to inspirations: existence is the game, NPCs have their own lives
- add Factorio philosophy: satisfaction of watching your creation work

## 2026-01-27
- wire EcsNpcManager into main.gd (Phase 1): replace npc_manager + projectile_manager with Rust ECS
- comment out: food production, respawning, building, upgrades, active radius, NPC selection (future phases)
- spawn_npc() calls for farmers, guards, raiders with Dictionary opts
- fix multimesh culling: set custom visibility rect on canvas item (NPCs disappeared at close zoom)
- fix test 11: add combat_target_2/3 to get_combat_debug() (was returning -99 default)
- fix test 11: move ranged pair from 200px to 150px apart (200px spans 2 grid cells, outside 3x3 neighborhood)
- test 11 unified attacks: all 7 phases passing (melee + ranged projectile pipeline)
- remove fire_projectile() GDScript API: all projectiles now created via PROJECTILE_FIRE_QUEUE from Bevy attack_system
- remove dead constants PROJECTILE_SPEED and PROJECTILE_LIFETIME (speed/lifetime now per-projectile via AttackStats)
- unify melee and ranged attacks: attack_system fires projectiles via PROJECTILE_FIRE_QUEUE instead of direct DamageMsg
- add AttackStats::melee() (range=150, speed=500, lifetime=0.5s) and AttackStats::ranged() (range=300, speed=200, lifetime=3.0s)
- add FireProjectileMsg and PROJECTILE_FIRE_QUEUE (attack_system → process() → GPU projectile system)
- add upload_projectile() lifetime parameter (was hardcoded PROJECTILE_LIFETIME constant)
- add spawn_npc opts Dictionary: optional params (home_x/y, work_x/y, town_idx, starting_post, attack_type) with defaults
- refactor spawn_npc from 10 positional params to 4 required + Dictionary (no more -1 padding)
- add attack_type spawn param: 0=melee (default), 1=ranged (fighters only)
- add gpu_health_2/3 to get_combat_debug() for 4-NPC test scenarios
- fix test 10 phase 4: check GPU health decrease instead of per-frame damage_processed counter
- fix melee projectile speed: 9999→500 px/s (was overshooting 10px hit radius at 60fps)
- add Fighter job (job=3): combat-only NPC with AttackStats+AttackTimer, no behavior components (yellow)
- rewrite Test 10 as 6-phase TDD combat test using Fighter NPCs (GPU buffers → grid → targeting → damage → death → slot recycle)
- add phase_results tracking: each phase records timestamp + values at pass/fail, included in debug dump
- add get_combat_debug() GPU buffer direct reads and grid cell data for NPC 0 and 1
- fix test 10 phase 6 infinite spawn: missing terminal test_phase assignment caused spawn every frame
- fix raider yellow-on-spawn: remove Raiding from spawn bundle, let steal_decision_system assign first target
- fix NPCs drifting to (-1,-1): add Home.is_valid() guard to tired_system and steal_decision_system
- fix farmers stuck in bed: set GPU target to work position on spawn (not spawn position)
- fix spawn timing: decouple slot allocation from GPU dispatch count (NPC_SLOT_COUNTER + GPU_DISPATCH_COUNT)
- fix uninitialized GPU buffers on spawn: process() now reads GPU_DISPATCH_COUNT instead of GPU_READ_STATE.npc_count
- change GPU_UPDATE_QUEUE drain guards from idx < npc_count to idx < MAX_NPC_COUNT (allows spawn writes before dispatch count catches up)
- add Chunk 8.5: generic spawn + eliminate direct GPU writes
- replace 5 spawn methods (spawn_npc, spawn_guard, spawn_guard_at_post, spawn_farmer, spawn_raider) with single spawn_npc() (10 params, job-as-template)
- remove SpawnGuardMsg, SpawnFarmerMsg, SpawnRaiderMsg and their queues/drain functions/spawn systems
- remove GpuData Bevy Resource (dead-end intermediary)
- remove all direct buffer_update() calls from spawn path — all GPU writes via GPU_UPDATE_QUEUE
- fix slot mismatch bug: slot_idx carried in SpawnNpcMsg (spawn.md rating 6→8/10)
- update ecs_test.gd to use unified spawn API
- add Chunk 8: generic raider behavior systems (steal, flee, leash, recovery)
- add generic components: Stealer, CarryingFood, Raiding, Returning, Recovering
- add config components: FleeThreshold, LeashRange, WoundedThreshold
- add steal_decision_system (wounded → carrying → tired → raid nearest farm)
- add steal_arrival_system (farm pickup with yellow color, camp delivery to FoodStorage)
- add flee_system (exit combat below HP threshold, drop food)
- add leash_system (disengage combat if too far from home)
- add wounded_rest_system + recovery_system (rest until healed to 75%)
- add FoodStorage resource with GDScript API (init, add, get town/camp food, events)
- update raider spawn bundle: add Energy, Stealer, FleeThreshold(0.50), LeashRange(400), WoundedThreshold(0.25), Raiding initial state
- update tired_system to also remove Raiding/Returning states
- fix deprecated Dictionary → VarDictionary in lib.rs (6 occurrences)
- fix test 11 slider: per-test range config (10-10,000 for projectiles, 500-5,000 for NPCs)
- fix slider label overwritten every frame by NPC count display
- fix performance regression: dynamically size projectile multimesh to active count instead of max
- increase MAX_PROJECTILES from 5,000 to 50,000 (~3.2 MB VRAM, zero cost when idle)

## 2026-01-26
- add Chunk 7a: Health + Death system
- add Health component (100 HP default), Dead marker component
- add DamageMsg queue and apply_damage() GDScript API
- add damage_system (applies queued damage to Health)
- add death_system (marks entities with Dead when health <= 0)
- add death_cleanup_system (despawns Dead entities, hides on GPU)
- add get_health_debug() API for health system inspection
- add Test 9: Health/Death (validates damage, death, despawn)
- add Chunk 7b: GPU Targeting + Attack system
- add GPU combat buffers: faction (binding 9), health (binding 10), combat_target (binding 11)
- add GPU targeting algorithm in npc_compute.glsl (finds nearest enemy in 3x3 grid neighborhood)
- add Faction component (Villager=0, Raider=1) for hostility checks
- add AttackStats component (range=150px, damage=15, cooldown=1s)
- add AttackTimer component for attack cooldown tracking
- add attack_system (reads GPU targets, queues damage, sets chase target)
- add cooldown_system (decrements attack timers using FRAME_DELTA)
- add spawn_raider() GDScript API with SpawnRaiderMsg queue
- add spawn_raider_system in Bevy ECS
- add GPU_COMBAT_TARGETS and GPU_POSITIONS static Mutexes for Bevy access
- add faction/health upload to GPU on spawn (guards, farmers, raiders)
- add Test 10: Combat (5 guards vs 5 raiders, validates GPU targeting and damage)
- add InCombat marker prevents behavior systems from overriding combat chase
- fix SystemSet phases with explicit apply_deferred between Spawn and Combat
- fix combat: sync positions/health to GPU, hide dead NPCs, enhance debug
- refactor GPU-First Architecture: consolidate 10+ queues → 2 (GPU_UPDATE_QUEUE, GPU_READ_STATE)
- fix grid cells: 64px → 100px (properly covers 300px detection range with 3x3 neighborhood)
- add O(1) entity lookup via NpcEntityMap (replaces O(n) iteration in damage system)
- add slot reuse: FREE_SLOTS pool recycles dead NPC indices (infinite churn without 10K cap)
- add Chunk 7c: GPU Projectile system
- add projectile_compute.glsl shader (movement + spatial grid collision detection)
- add projectile buffers: position, velocity, damage, faction, shooter, lifetime, active, hit
- add fire_projectile() GDScript API with slot reuse via FREE_PROJ_SLOTS
- add projectile MultiMesh rendering with velocity-based rotation
- add get_projectile_count() and get_projectile_debug() APIs
- add Test 11: Projectiles (TDD test covering fire, move, collide, damage, expire, recycle)
- refactor test harness: replace 10 buttons with dropdown + run button
- fix projectile rendering: share NPC canvas item (second canvas_item_create doesn't render)
- fix projectile hit buffer: initialize to -1 (GPU zeros misread as "hit NPC 0")
- add get_projectile_trace() API for GPU buffer inspection (lifetime, active, position, hit)
- add faction-colored projectiles (blue=guard, red=raider) via proj_factions cache
- improve Test 11: spawn offset (20px forward), 50-projectile burst test

## 2026-01-25
- add Chunk 3: GPU physics with 8-buffer architecture (position, target, color, speed, grid, multimesh, arrivals)
- add EcsNpcManager owns GpuCompute (RenderingDevice not Send-safe for static Mutex)
- add spatial grid 128x128 cells with 48 NPCs/cell max
- add push constants 48-byte alignment with padding fields
- fix GPU buffer upload timing (separate GPU_NPC_COUNT static for immediate spawn)
- fix separation algorithm: accumulate proportionally instead of normalizing (boids-style)
- fix zero-distance separation: use golden angle fallback when NPCs overlap exactly
- add persistent arrival flag (NPCs don't chase target after being pushed away)
- add reset() function for EcsNpcManager (clears NPC count for scene reload)
- add arrival flag initialization on spawn (prevents stale buffer data)
- add ecs_test.tscn with 5 test scenarios and NPC count slider
- add test descriptions: arrive (target seeking), separation (push apart), both, circle, mass
- add TDD assertions: get_npc_position(), _assert_all_separated(), PASS/FAIL state
- add real-time min separation metric display
- add GPL 3.0 license
- add unified collision avoidance system (merged separation + blocking detection)
- add TCP-style exponential backoff for blocked NPCs (give up after 120 frames)
- add backoff buffer (binding 8) for per-NPC collision counter
- add get_debug_stats() for GPU state inspection (arrived count, max backoff)
- reduce separation strength from 200 to 100
- add asymmetric push: moving NPCs shove through settled NPCs (0.2x resistance)
- add TCP-style dodge: NPCs sidestep around other moving NPCs (head-on, overtaking, crossing)
- fix backoff detection: sideways pushing now counts as blocked
- fix test 1: check arrived count instead of position, wait 5s instead of 3s
- add copy debug info button to test harness
- add comprehensive documentation to lib.rs and npc_compute.glsl
- add Chunk 4: World Data API (towns, farms, beds, guard posts as Bevy Resources)
- add init_world(), add_town(), add_farm(), add_bed(), add_guard_post() GDScript API
- add get_town_center(), get_camp_position(), get_patrol_post() query functions
- add get_nearest_free_bed(), get_nearest_free_farm() for NPC assignment
- add reserve_bed(), release_bed(), reserve_farm(), release_farm() for occupancy tracking
- add get_world_stats() for debugging (counts and free slots)
- add Test 6: World Data (verifies all world data API functions)
- add Chunk 5: Guard Logic (guards patrol and rest autonomously in Bevy ECS)
- add guard state components: Patrolling, OnDuty, Resting, GoingToRest
- add Guard, Energy, HomePosition components
- add energy system (drain 0.02/tick active, recover 0.2/tick resting)
- add guard decision system (energy < 50 → go rest, energy > 80 → resume patrol)
- add guard patrol system (OnDuty 60 ticks → move to next post clockwise)
- add arrival detection from GPU buffer (ArrivalMsg queue, prev_arrivals tracking)
- add GPU_TARGET_QUEUE for Bevy→GPU target updates (systems can set NPC targets)
- add spawn_guard() and spawn_guard_at_post() GDScript API
- add Test 7: Guard Patrol (4 guards at corner posts, clockwise perimeter patrol)
- fix reset() to clear all queues (GUARD_QUEUE, ARRIVAL_QUEUE, GPU_TARGET_QUEUE)
- fix reset() to clear prev_arrivals (enables arrival detection on new tests)
- fix backoff: sideways jostling no longer increments backoff (only pushed away from target)
- reduce separation strength from 100 to 50 (prevents outer NPCs being pushed away on converge)
- add get_build_info() for DLL version verification (dynamic timestamp + commit hash)
- add build.rs for compile-time build info injection
- add /test command for clean build and test workflow
- fix test harness perf: throttle O(n²) min_sep to once per second (20 FPS → 130 FPS @ 500 NPCs)
- fix test harness perf: throttle get_debug_stats() GPU reads to once per second
- fix test harness perf: run O(n²) assertions once per test, not every frame after timer
- remove console log spam: debug info now UI-only (no godot_print/print calls)
- add "Collect Metrics" toggle (off by default) to disable all O(n²) checks and GPU reads
- skip test validation when metrics disabled (raw performance mode)
- add Chunk 6: Behavior-based NPC architecture (systems as behaviors)
- refactor guard components: HomePosition→Home, Guard.current_post→PatrolRoute
- add generic behavior systems: tired_system, resume_patrol_system, resume_work_system, patrol_system
- add WorkPosition, Working, GoingToWork components for work behavior
- add Farmer component and spawn_farmer() GDScript API
- add Test 8: Farmer Work Cycle (validates work/rest behavior)
- refactor lib.rs into modules: components, constants, resources, world, messages, gpu, systems/

## 2026-01-24
- add start menu with world size, town count, farmers/guards/raiders sliders (max 500)
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
- add Rust GDExtension with GPU compute: 10,000 NPCs @ 140fps
- add npc_compute.glsl shader (separation forces on GPU via Godot RenderingDevice)
- add spatial grid for O(n) neighbor lookup (128x128 cells, 48 NPCs/cell max)
- add godot-bevy integration with Bevy ECS state machine
- add NpcStateMachine bridge (GDScript pushes NPC data, pulls state changes)
- add guard patrol logic in Bevy ECS (low energy → walking, else → patrolling)
- add godot-bevy addon (BevyApp autoload, inspector panel, debugger plugin)
- add Chunk 1: Bevy ECS renders static NPCs via MultiMesh (EcsNpcManager)
- add Chunk 2: movement system with velocity, target, arrival detection
- add GDScript API: spawn_npc(), set_target(), get_npc_count()
- revise migration plan: GPU physics as Chunk 3 foundation layer
- add rust/, shaders/, scenes/, scripts/ to README architecture

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


