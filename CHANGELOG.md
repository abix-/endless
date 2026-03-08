# Changelog

## 2026-03-08b

- **Clippy setup** — added crate-level `#![allow]` for Bevy-inherent warnings (`too_many_arguments`, `type_complexity`, `collapsible_if`) in both `lib.rs` and `main.rs`. Ran `cargo clippy --fix` for ~85 auto-fixes: `.get(0)` → `.first()`, useless `format!()`, `map_or(false, ...)` → `is_some_and(...)`, redundant closures, unnecessary casts, `#[derive(Default)]` additions.
- **Production unwrap() audit** — replaced all 34 non-test `unwrap()` calls with safe alternatives. Critical GPU readback paths (`health.rs`, `economy/mod.rs`, `world.rs`, `game_hud.rs`) use `let Some(x) = ... else { continue/return }`. Guarded patterns (`ai_player.rs`, `stats.rs`, `entity_map.rs`) restructured to `map_or`/`if let Some`. Low-risk invariants (`blackjack.rs`, `pathfinding.rs`, `save.rs`, etc.) upgraded to `.expect("reason")`. NaN-safe f32 sort in `economy/mod.rs` via `unwrap_or(Ordering::Equal)`.
- **Roadmap Stage 19 (Code Health)** — inserted new stage with 4 items (fix failing tests, CI, split god-files, audit unwraps). All subsequent stages renumbered +1 (19→20 through 30→31). Cross-references updated (tech-tree spec, wall gates, tower defense).

## 2026-03-08

- **Fix 5 failing tests** — test towns used `faction: 0` (FACTION_NEUTRAL) instead of active factions, causing healing cache, mining discovery, and raider forage tests to fail. Fixed `update_healing_zone_cache` to skip negative factions (`<= FACTION_NEUTRAL` guard).
- **Split god-files** — extracted `entity_map.rs` from `resources.rs` (EntityMap + BuildingInstance + NpcEntry, ~1K lines), split `economy.rs` tests into `economy/tests.rs` (~960 lines), split `left_panel.rs` into `left_panel/` module with `roster_ui.rs`, `upgrades_ui.rs`, `inventory_ui.rs` submodules (~900 lines extracted). All re-exported for API compatibility.
- **Squad target behavior fix** — archers at squad target now scatter near the squad target instead of walking back to their patrol waypoint. Squad intent always submitted with priority resolution, removing brittle per-activity redirect logic. Patrol cycling suppressed when squad has an active target.
- **Inspector unequip buttons** — each equipped item in the NPC inspector now has an Unequip button. "Manage Equipment >" link opens the Inventory tab. Non-military NPCs no longer show "No equipment".
- **LLM prompt: skip wiped towns** — AI town data now includes alive/dead NPC counts. Prompt instructs AI to skip wiped towns (alive < 5 or no buildings) when choosing attack targets.

## 2026-03-07e

- **Rock/Water passable terrain** — rock (cost 500) and water (cost 800) are now expensive but passable in pathfinding, so NPCs pushed onto these tiles by GPU physics can slowly pathfind off. Previously cost 0 (impassable), trapping NPCs.
- **A* route spreading** — successive pathfinding calls within the same batch inflate costs along previously-found paths (`PATH_SPREAD_COST=100`, `PATH_SPREAD_RADIUS=1`), naturally spreading NPC routes apart instead of all funneling through one corridor. New `pathfind_with_costs` and `accumulate_path_cost` helpers in `pathfinding.rs`.
- **Intermediate waypoint threshold** — relaxed arrival detection for A* waypoints (96px vs 40px for final destination). Prevents pile-up when boid separation pushes NPCs away from shared waypoints.
- **Inventory system overhaul** — 3-way view mode (Unequipped/Equipped/All) shows equipped items across all town NPCs with owner names. Slot filter toggles (9 equipment slots with counts). Sorting by slot → rarity → bonus. Bulk sell common items for gold. Comparison tooltips on hover show current vs new bonus with upgrade/downgrade arrows. Multi-town support (derives town from selected NPC).
- **Auto-equip system** — once per game hour, scans `TownInventory` and distributes items to the best NPC candidate (empty slot first, then lowest current bonus). Writes `EquipItemMsg` to reuse existing equip pipeline.
- **Equipment drops on death** — victim's `NpcEquipment` and `CarriedLoot.equipment` transfer to killer at 50% per-item (deterministic hash). NPC killers carry items home; tower killers deposit directly to `TownInventory`. New `NpcEquipment::all_items()` helper.
- **LLM combat log improvements** — action messages use town names and building/upgrade labels instead of raw IDs. Filters confidence-rating chat spam. Surfaces LLM reasoning/commentary as single combat log entry.
- **Build menu max width** — clamped to available gap between left panel and combat log to prevent overflow on narrow screens.

## 2026-03-07d

- **Rock terrain impassable** — rock biome is now impassable (pathfinding cost 0, like water). Building placement blocked on rock tiles. `add_town_buildable` skips rock cells. Ghost preview shows red on rock.
- **Decision bucket speed scaling** — `think_buckets` and `COMBAT_BUCKET` now scale down with `time_scale` so NPC decisions keep pace with movement at high game speeds.
- **Build menu positioning** — anchor shifts horizontally to center between left panel and combat log when either is open, preventing overlap.
- **Build menu autotile sprites** — all autotile buildings (roads, walls) now extract the first sprite from their strip for the toolbar icon. Previously only walls did this; roads showed the full squashed strip.
- **Combat log word wrap** — log entries use `horizontal_wrapped` so long messages wrap within the window instead of expanding it horizontally.
- **Casino sprite** — changed from roguelike sheet tile to external `casino_64x64.png`.

## 2026-03-07c

- **Faction system refactor** — replaced bare `i32` faction IDs with structured `FactionKind` enum (`Neutral=0`, `Player=1`, `AiBuilder`, `AiRaider`), `FactionData` struct, and `FactionList` resource. Each town gets its own faction. Constants `FACTION_NEUTRAL` (0), `FACTION_PLAYER` (1), `TOWN_NONE` (`u32::MAX`) replace all hardcoded numeric checks across 30+ files.
- **Gold mine worksite fix** — gold mines now spawn with `town_idx: TOWN_NONE` (neutral) instead of `town_idx: 0` (Miami). `try_claim_worksite()` accepts `TOWN_NONE` buildings for any town's workers, so all factions' miners can claim gold mines.
- **Neutral faction targeting fix** — combat system (`combat.rs`) now excludes `FACTION_NEUTRAL` buildings from targeting. GPU shaders (`npc_compute.wgsl`, `projectile_compute.wgsl`) updated to treat faction 0 as non-hostile alongside the -1 sentinel.
- **Faction UI duplicate fix** — factions panel no longer shows the player faction twice. `rebuild_factions_cache` skips player-faction towns when iterating AI players.
- **Runtime faction tracking** — `create_ai_town` in economy.rs pushes `FactionData` to `FactionList` when spawning dynamic towns during migration.
- **Player stats fix** — `faction_stats.stats.get(player_faction)` replaces `.first()` which was returning neutral faction stats after renumbering.
- **Save/load backward compat** — old saves without `FactionList` get it reconstructed from town data (sprite_type=1 → AiRaider, else AiBuilder).

## 2026-03-07b

- **Stop-in-place short-circuit** — `resolve_movement_system` now bypasses `path_cooldown` when an intent's target is within 2 units of the NPC's current position. Fixes idle NPCs retaining stale GPU targets from a previous activity (e.g., farm claim losers walking into the farm they failed to claim), causing bumping/oscillation.
- **Chat inbox no longer drained by summary** — `summary_handler` reads `ChatInbox` immutably instead of using `std::mem::take`. Chat messages persist in the HUD after BRP summary requests.
- **Debug endpoint expansion** — `endless/debug` auto-detects NPC vs building from `uid` (no `kind` param needed). New `kind`+`index` mode for resource inspection: `squad`, `town`, `policy`.

## 2026-03-07a

- **World grid coordinates everywhere** — eliminated town-relative `(row, col)` coordinate system entirely. All ~50 call sites across 6 source files now use world grid `(col, row)` as `(usize, usize)`. Deleted `town_grid_to_world`, `world_to_town_grid`, `find_town_slot`, `TownSlotInfo`. `build_bounds` returns `(min_col, max_col, min_row, max_row)` in world grid. `empty_slots` returns `Vec<(usize, usize)>` world grid coords. AI scoring functions (`farm_slot_score`, `balanced_farm_ray_score`, `balanced_house_side_score`, etc.) all operate on `(usize, usize)`. BRP `endless/build` and `endless/destroy` params changed from `row: i32, col: i32` to `col: usize, row: usize` (world grid). LLM prompt updated.
- **`--autostart` CLI flag** — skip main menu for BRP testing. `AutoStart` resource parsed from CLI args in `main.rs`, `autostart_system` (OnEnter MainMenu) loads saved settings, configures world gen + AI slots + LLM towns, and transitions directly to Playing.
- **`endless/perf` BRP endpoint** — returns FPS, frame_ms, UPS, NPC count, entity count. Includes per-system timing BTreeMap when profiling is enabled.
- **`endless/debug` uses EntityUid** — changed from GPU slot index to stable `uid: u64` parameter. Resolves UID to slot via `EntityMap::slot_for_uid`.
- **Inspector cleanup** — removed ~835 lines of verbose GPU raw state dump from Copy Debug Info (NPC and building inspectors). Kept concise ECS-only debug output. Removed unused SystemParam fields (`GpuSlotPool`, `NpcTargetThrashDebug`, `EntityGpuState`, `NpcVisualUpload`).
- **`debug_coordinates` → `debug_ids`** — renamed settings field to reflect actual usage (shows EntityUid overlay, not coordinates).

## 2026-03-07

- **Unified chat system** — `ChatInbox` is now the single source of truth for player ↔ LLM messaging. Replaced dual-write (ChatInbox + combat log) with ChatInbox-only writes. Combat log UI reads directly from ChatInbox and renders `[chat to Town]` / `[chat from Town]` under the Chat filter. `ChatMessage` gains `sent_to_llm` and `has_reply` flags for delivery tracking. `ChatInbox` converted to `VecDeque` ring buffer (200 cap) to prevent unbounded growth.
- **LLM payload reduction** — flat per-town fields (compact building strings `"Farm:24,ArcherHome:4"`, squad counts, short field names) for tabular TOON format. Own-town extras (your_squads, open_slots, inbox) moved to top level. Upgrades flattened to single-line cost strings. Scales O(building_types) not O(buildings).
- **CSV action format** — LLM responses switched from TOON `actions[N]:` arrays to plain CSV lines: `method, key:value, key:value, ...`. Parser uses `split`/`split_once` instead of `serde_toon2::from_str`. Special `message:` handling preserves commas in chat text.
- **LLM combat log locations** — all LLM action log entries now use `push_at` with world positions, enabling camera-pan buttons in the combat log for build/destroy/upgrade/squad_target/policy actions.
- **Prompt updated** — `prompt_builtin.md` rewritten with flat field names (i, cx, cy, dist, rep), top-level your_squads/open_slots/inbox docs, CSV examples.

## 2026-03-06a

- **TOON response format for LLM player** — LLM responses now use proper TOON `actions[N]:` arrays instead of custom `method key:value` one-liners. Parser replaced with `serde_toon2::from_str` (eliminated custom `parse_toon_value` and line-by-line parser). Prompt updated with one comprehensive TOON example and compact per-action field lists. End-to-end TOON: outbound state AND inbound responses.
- **LLM player settings tab** — new pause menu tab with cycle interval slider (5-120s, step 5, synced live to timer), collapsible inspectors for last command, last payload, and last response, each with copy-to-clipboard buttons via `OutputCommand::CopyText`.
- **LLM HUD status indicator** — colored circle in top bar: gray (idle), blue (sending), yellow (thinking), green (done). `LlmStatus` enum on `LlmPlayerState`.
- **LLM chat action** — built-in player can now send chat messages to other towns via `chat` action (to, message params). Messages pushed to `ChatInbox` (moved from `LlmReadState` to `LlmWriteState` for write access).
- **BRP debug endpoint** — new `endless/debug` endpoint for deep NPC/building inspection by GPU slot. Returns full ECS component data (job, activity, health, equipment, personality, combat state, flags for NPCs; kind, grid, occupants, worksite for buildings).
- **Keyboard focus suppression** — camera pan and hotkeys suppressed when egui text fields have keyboard focus (chat input, save name).
- **Rust 1.94** — bumped MSRV, `array_windows()` replaces `windows(2)` in tests.

## 2026-03-05h

- **TOON format for all LLM communication** — replaced JSON with TOON (Token-Oriented Object Notation) for 30-60% token savings. Added `serde_toon2` crate. BRP summary endpoint uses typed `SummaryResponse` struct with tuples for tabular data (factions, buildings, squads, upgrades, combat_log, inbox), serialized via `serde_toon2::to_string()`. All write handler responses use `toon_ok()` helper. New data in summary: `RemoteCombatLogRing` (VecDeque cap 20) for recent combat events, `TownUpgrades` for per-town upgrade levels/costs, `compact_npc_counts()` collapses verbose per-activity keys into `Archer: 8 (Patrolling:5 OnDuty:3)`.
- **Built-in LLM player TOON I/O** — `llm_player.rs` sends TOON state (via `serde_toon2::to_string` on `serde_json::Value`) and parses TOON action lines (`method key:value ...`) with auto-typed values. Removed JSON fallback, `fix_unquoted_keys` regex, `Deserialize` derive. Topics use comma syntax: `subscribe topics:npcs,upgrades`.
- **External LLM player TOON params** — `actions.py` accepts TOON key:value pairs as separate shell args instead of quoted JSON blob. `parse_toon_params()` with auto-typing (bool/int/float/string), JSON fallback if arg starts with `{`. `loop.py` simplified to dump raw TOON string.
- **Prompts updated** — both `prompt.md` (external) and `prompt_builtin.md` (built-in) rewritten with TOON examples throughout. All JSON references removed.

## 2026-03-05g

- **Built-in LLM player** — new `systems/llm_player.rs` spawns `claude --print` every 20s, reads ECS resources directly (no BRP round-trip). Builds JSON state payload with own-town full building list vs enemy-town counts for token efficiency. Supports all actions: build, destroy, upgrade, policy, squad_target. Three-tier data model: base state (always sent), subscriptions (persistent topics), one-shot queries (next cycle only). Stdin piping bypasses Windows 32K CLI limit. `CREATE_NO_WINDOW` flag prevents console focus stealing. `fix_unquoted_keys` regex fallback handles Haiku's occasional unquoted JSON.
- **Built-in LLM prompt** — new `llm-player/prompt_builtin.md` with complete action reference, data topic docs (npcs, combat_log, upgrades, policies), per-town distance + reputation fields, road expansion strategy guidance. Optimized for token efficiency with no duplication.
- **Faction reputation matrix** — `Reputation` resource expanded from 1D `Vec<f32>` to 2D `Vec<Vec<f32>>` matrix. `values[a][b]` = faction a's opinion of faction b. Decremented by 1 per kill via `on_kill()` at both NPC and tower kill sites in `health.rs`. Range -9999..9999. Exposed per-town as `reputation` field in LLM state. Blackjack updated to 2D indexing. Save format migrated with backward-compatible custom deserializer (1D→2D).
- **Regex dependency** — added `regex v1` for LLM output fixing.

## 2026-03-05f

- **In-game chat (player ↔ LLM)** — two-way messaging between human player and LLM-controlled towns. Chat input at bottom of combat log sends messages to all LLM towns via `ChatInbox` resource. Messages appear in `endless/summary` response as `inbox` array per-town, drained on read. New `endless/chat` BRP endpoint lets LLM reply (logged as LLM combat event). New `CombatEventKind::Chat` (gold color) with filterable checkbox.
- **Faction consistency in inspectors** — NPC and building inspectors now show `Faction: Orlando (F1)` instead of raw numbers (`1 (town 1)` / `1`). Squad overlay arrows show `Orlando Squad 1` instead of `F1 Squad 1`. Matches the factions tab format.
- **LLM player CLI improvements** — `actions.py` now works as CLI tool (`python actions.py method '{"params"}'`), eliminating ugly heredoc/import patterns. `launch.py` grants `Read` permission for loop.log access. Prompt strategy section updated with battle-tested lessons: food > 50 gate, 15+ military homes before attacking, commit to one target, re-issue squad orders frequently.

## 2026-03-05e

- **LLM player guide** — new `docs/llm-player.md` (setup, game loop, token budget, strategy tips) and `docs/llm-player-prompt.md` (complete system prompt for the model: role, actions reference, strategy phases, rules). Covers Claude Code, Anthropic API, and generic HTTP approaches.

## 2026-03-05d

- **WC3-style AI player lobby** — main menu replaces aggregate "AI Builder Towns" / "AI Raider Towns" sliders with per-slot player rows. Each slot has a Builder/Raider dropdown and an LLM checkbox. Add/remove buttons, max 20 slots. Raider settings (tents, forage) shown conditionally when raider slots exist. Difficulty presets rebuild slots preserving LLM flags. Persisted via `ai_slots: Vec<AiSlotSave>` in UserSettings with backward compat from legacy fields.
- **BRP write access control** — new `RemoteAllowedTowns` resource populated from LLM-checked slots on Play. `check_town_allowed()` helper gates all write endpoints (`build`, `upgrade`, `policy`, `ai_manager`, `squad_target`) — rejects requests for non-LLM towns. Read endpoints (`summary`, `world.query`) remain unrestricted. Squad target resolves `SquadOwner` to town index for access check.

## 2026-03-05c

- **brp.md AI model integration section** — documented the design philosophy for AI model players: token-efficient polling, delegation to in-game AI Manager for grunt work, read-heavy/write-sparse interaction pattern, model-agnostic HTTP interface.

## 2026-03-05b

- **BRP live game control** — Bevy Remote Protocol (bevy_remote) HTTP JSON-RPC server on localhost:15702. Added `Reflect` + `#[reflect(Component)]`/`#[reflect(Resource)]` to all 33 components + 12 resources + 5 nested types. 60+ `register_type` calls in `build_app()`. 7 custom action endpoints in `systems/remote.rs`: `endless/summary` (game overview), `endless/build` (queue building placement), `endless/upgrade` (queue town upgrade), `endless/policy` (set town policies), `endless/time` (pause/speed), `endless/squad_target` (move squads), `endless/ai_manager` (configure AI Manager). Queue pattern for write endpoints needing SystemParams, direct resource_mut for simple mutations.
- **AI Manager settings persistence** — player town AI Manager state (active, build/upgrade enabled, personality, road style) saved to UserSettings on panel close and restored on game startup. New fields in `settings.rs`: `ai_manager_active`, `ai_manager_build`, `ai_manager_upgrade`, `ai_manager_personality`, `ai_manager_road_style`.
- **idle stop movement** — NPCs transitioning to Idle now submit a self-position movement intent to clear stale GPU targets, preventing oscillation with nearby NPCs.
- **pathfind-maze configurable count** — pathfind-maze test now supports 1-5000 NPCs via slider UI (PathfindMazeConfig resource).

## 2026-03-05a

- **stuck-transit redirect** — bucket-gated re-scatter for wandering and patrolling NPCs that haven't arrived. Wandering NPCs get a new random offset from current position (128px, clamped within 200px of home). Patrolling NPCs re-scatter to their current post. Unsticks NPCs blocked by walls or congestion.
- **wander from current position** — wander action now offsets from NPC's current position (was home), clamped within 200px of home to prevent unbounded drift. Scatter radius reduced from 200px to 128px.
- **settled-settled push reduction** — GPU separation shader adds a both-settled case (push_strength=0.15) so NPCs at different destinations don't jitter against each other. Patrol and heal fountain scatter radii unified to 128px.
- **orphaned NPC home reset** — when a building dies, its linked NPC (npc_uid) has Home set to (-1,-1). Inspector shows "Homeless" instead of coordinates. Prevents NPCs from walking to destroyed buildings to rest.
- **farmer_cycle test scale-up** — expanded from 3 homes/2 farms to 20 homes/16 farms (4x4 farm grid + 20 border homes). Validates occupancy at scale with 4 expected idle farmers.
- **tracked_section UI helper** — collapsing headers across Roster/Profiler/Help tabs now use `tracked_section()` with stable egui IDs for save/restore of collapsed state. Added profiler and help sections to TRACKED_SECTIONS.

## 2026-03-03b

- **centralized work targeting** — all worksite occupancy mutations (claim/release/retarget) moved from 17 inline release sites and 6 claim sites in `decision_system`/`death_system` into a single `resolve_work_targets` system via `WorkIntentMsg` messages. `NpcWorkState` merged from two fields (`occupied_building` + `work_target_building`) into single `worksite: Option<EntityUid>`, eliminating the desync class of bugs. Resolver is the sole caller of `entity_map.release()` and `try_claim_worksite()` for NPC work slots. Release messages carry UID from sender to avoid write-back race. `worksite_deferred` flag gates NpcWorkState write-back and stale invariant. Arrival handler simplified (~220 → ~60 lines). `find_farm_target` and `find_mine_target` consolidated into `work_targeting.rs`.

## 2026-03-03a

- **unified cleanup** — merged `cleanup_test_world` (OnExit Running) into `game_cleanup_system` (OnExit Playing). Single shared cleanup for both game and test states — no more drift. Added missing resets: `EndlessMode`, `TownInventory`, `MerchantInventory`, `NextLootItemId`, `DebugFlags`, `MiningPolicy`, `TerrainChunk` despawn. New `CleanupUi` SystemParam bundle consolidates loose params.
- **merged resolve_movement + pathfind_budget** — collapsed `pathfind_budget_system` into `resolve_movement_system`. Single system now handles intent filtering, PathRequest enqueueing, and A*/LOS routing in one pass. Eliminates duplicated manhattan + LOS decision logic. PathRequestQueue gains `submit()` for world-space intents and `drain_intents()` for phase 1 processing. Guard against empty `pathfind_costs` prevents crash on first frame.

## 2026-03-02q

- **A* pathfinding system** — new `pathfinding.rs` module with budgeted A* on `WorldGrid`. CPU computes waypoints via `pathfinding` crate; GPU boids steer toward current waypoint via existing `goals[]` buffer. Walls and water block pathfinding. LOS bypass for short-distance moves (<5 tiles). Priority queue (`PathRequestQueue`) with per-frame budget (50 requests). `NpcPath` component on all NPCs. `advance_waypoints_system` progresses through path on arrival. `invalidate_paths_on_building_change` re-queues paths when buildings change.
- **pathfind-maze test scene** — visual integration test: farmer navigates serpentine wall maze (5 horizontal wall rows with alternating gaps). 5 phases: spawn → A* waypoints → cross wall rows → reach farm. 4 unit tests for wall-based maze pathfinding (single wall, serpentine, walled-off, LOS blocked by wall).
- **economy system tests** — unit tests for `mining_policy_system` (discover/ignore mines by radius, skip without dirty) and `squad_cleanup_system` (remove dead members, retain alive, skip without dirty).

## 2026-03-02p

- **7-axis spectrum personality** — replaced 4-trait system (Brave/Tough/Swift/Focused) with 7 spectrum axes (Courage/Diligence/Vitality/Power/Agility/Precision/Ferocity), each with signed magnitude (±0.5 to ±1.5). Positive pole = beneficial (Brave, Efficient, Hardy, Strong, Swift, Sharpshot, Berserker), negative = detrimental (Coward, Lazy, Frail, Weak, Slow, Myopic, Timid). All 7 axes affect both stats (`resolve_combat_stats` via `TraitStatMods`) and behavior weights (`decision_system` via `TraitBehaviorMods`). Personality generation: 20% per axis, cap at 2 traits, deterministic LCG. Save compat: `PersonalitySave.version` (0=legacy 4-trait, 1=spectrum) with `from_legacy_id()` migration.
- **berserk damage system** — `CachedStats.berserk_bonus` from Ferocity axis. `attack_system` applies damage multiplier `(1 + berserk_bonus)` when HP <50%. Berserker trait: +50%×m damage bonus; Timid: -50%×|m| penalty.
- **personality-modified flee** — Brave trait: `never_flees = true` (ignores flee threshold). Coward trait: `flee_threshold_add = +0.20 × |m|` (flees earlier). Applied in `decision_system` after policy flee_pct calculation.
- **query-first death detection** — `death_system` Phase 1a now uses ECS query `(Entity, &Health, &GpuSlot), Without<Dead>` instead of `iter_npcs()`. Inserts `Dead` marker component. Eliminates last hot-path `iter_npcs()` violation in health.rs.
- **blackjack: single deck + rules** — shoe changed from 3 decks to 1, cut card 39→13. Added collapsible "Rules" section in betting UI.
- **NpcMeta.trait_display** — replaced `trait_id: i32` with `trait_display: String` (pre-formatted at spawn). Roster panel reads cached string instead of calling `trait_name()`. Removed `lib::trait_name()`.

## 2026-03-02o

- **dead code cleanup: constants.rs** — removed 16 unused constants: `SEPARATION_RADIUS/STRENGTH` (gpu.rs hardcodes values), `ENERGY_RESTED`, `SCORE_FIGHT_BASE`, `SCORE_FLEE_MULT`, `ROAD_SPEED_MULT`, `WALL_EXTRA_LAYERS`, `BUILDING_HIT_RADIUS`, `WAYPOINT_COVER_RADIUS`, and 7 `ATLAS_*` constants (CHAR/WORLD/HEAL/SLEEP/ARROW/BUILDING_HP/MINING_BAR — all bypassed with magic literals in npc_render.rs). Kept `ATLAS_BUILDING` and `ATLAS_BOAT` (referenced in registries).

## 2026-03-02n

- **casino building + blackjack popup** — new `BuildingKind::Casino` (1 per town, 80 gold, Economy category). Blackjack minigame moved from left panel tab to standalone popup window (`UiState.casino_open`). Open via double-click on Casino building, inspector "Open Casino" button, or keybind. Full-window card rendering with visual card layout.
- **perf: dc_slots() helper** — `game_hud.rs` replaced 3× `iter_npcs()` O(n) scans for direct-control NPCs with `dc_slots()` helper that iterates selected squad members O(squad_size). Squad size typically <100 vs 50K NPCs.
- **perf: death_system single scan** — `health.rs` Phase 1a marks newly dead NPCs and collects their slots; Phase 2b reuses that vec instead of re-scanning all NPCs. Eliminates redundant O(n) `iter_npcs()` call.
- **sandbox test** — human player sandbox scene: 1 player + 1 AI builder town, 100K food+gold, no raiders. Auto-completes for free play.
- **performance.md updates** — added SLO targets table, Current Tunings reference table, Known Exceptions table (4 tracked violations with exit criteria), slot invariant documentation, scoped rule claims. behavior.md and economy.md now reference performance.md for bucketing/cadence formulas instead of duplicating them.

## 2026-03-02l

- **performance doc consolidation** — `performance-review.md` → `performance.md`, now single authority for all perf patterns. Added: core principles table, GPU perf patterns (readback minimization, dirty-index uploads, coalescing, instanced rendering), CPU cadencing patterns (bucket-gated decisions, candidate-driven healing, fixed-cadence systems, event-driven systems), debug overhead rules with O(n²) example. Slimmed `concepts.md` — removed GPU Readback Avoidance, Debug Mode Overhead, Staggered Processing, LOD Intervals sections (all moved to performance.md), trimmed summary table perf rows.

## 2026-03-02k

- **authority doc consolidation** — merged full data ownership table from messages.md into authority.md (now single source of truth for all data ownership: GPU-authoritative, CPU-authoritative, CPU-only, render-only categories). Slimmed messages.md to reference link. Fixed stale AttackStats values (was range=150/300, corrected to 100/200). Added anti-pattern rule #8 (no readback→writeback same frame).
- **click_to_select ECS faction** — enemy NPC hit-test in `click_to_select_system` now reads faction from `EntityMap` (ECS authoritative) instead of throttled GPU factions readback.
- **debug_tick EntityMap count** — `debug_tick_system` uses `EntityMap.npc_count()` instead of `GpuSlotPool.alive()` per authority rule #7.

## 2026-03-02j

- **fix ghost character sprites on buildings** — `build_visual_upload` sized buffers from `RenderFrameConfig.npc.count` (stale FixedUpdate copy) instead of live `GpuSlotPool.count()`. On frames where FixedUpdate hadn't ticked (especially startup/OnEnter), count was 0, truncating visual buffers and silently dropping dirty writes. Building slots never got visual_data populated, causing uninitialized data to render as character sprites on top of buildings. Fixed by reading live allocator count directly.
- **fix 64px grid alignment in 20 test files** — all test building/NPC positions snapped to 64px-aligned coordinates, `slots.alive()` replaced with `entity_map.iter_npcs()` count, equip stride 24→28, obsolete Bed buildings removed, default town center 400→384.
- **building inspector equip debug** — "Copy Debug Info" now dumps all 7 equip layers (col/row/atlas) for building slots, enabling diagnosis of stale equip buffer issues.
- **authority doc: entity count rules** — added entity slot count, NPC count, and building count authority to `docs/authority.md` with hard rules #6 (main-world systems must read GpuSlotPool directly) and #7 (use EntityMap for type-specific counts).

## 2026-03-02i

- **fix building overlap (64px grid)** — `EntityMap` grid cell lookups in `resources.rs` used hardcoded `/ 32.0` instead of `TOWN_GRID_SPACING` (now 64.0), causing `has_building_at()` to check wrong grid cells. AI would stack buildings at the same position. All 12 occurrences replaced with `TOWN_GRID_SPACING`.
- **fix ghost NPC sprites on buildings** — equip buffer coalesced upload in `npc_render.rs` used stride 24 (6 layers) but actual data is 28 floats per slot (7 layers × 4 floats). Incremental equip updates wrote to wrong GPU offsets, leaving stale NPC equipment sprites visible on building slots. Fixed stride to 28 and gap constant to 27.
- **FPS cap setting** — new `fps_cap` field in `UserSettings` (0=uncapped, 30/60/120/144/240 presets). ComboBox in Video settings (both pause menu and main menu). Drives Bevy `focused_mode` via `focused_mode_for_fps_cap()` helper. Applied on startup, setting change, and reset-to-defaults.

## 2026-03-02h

- **fixed 60 UPS game loop (Factorio model)** — all game systems (Drain → Spawn → Combat → Behavior → movement resolution → GPU data update) moved from `Update` (variable dt) to `FixedUpdate` at 60 Hz (16.67ms/tick). Deterministic simulation: `time.delta_secs()` in FixedUpdate always returns 1/60s, `GameTime::delta()` returns `(1/60) * time_scale`. Test tick systems (28 registrations) also moved to FixedUpdate; UI/save/audio/camera stay on Update. `UpsCounter` resource tracks actual ticks/second — FixedUpdate increments counter, HUD samples per wall-clock second. UPS displayed in top bar alongside FPS.
- **blackjack GoldStorage crash fix** — `UpgradeParams` and `FactionsParams` both accessed `GoldStorage` in `left_panel_system`, causing Bevy SystemParam conflict panic on launch. Fixed by consolidating `GoldStorage` into `FactionsParams` only, passing read-only ref to `upgrade_content()`.
- **boat_pos economy fix** — merchant boat position update changed from raw `time.delta_secs()` (frame-rate dependent) to `game_time.delta(&time)` (deterministic under FixedUpdate).

## 2026-03-02g

- **atlas 64px upscaling** — sprite atlas cell size doubled from 32px to 64px with 2x blit upscaling (`blit_2x`); world scale 2x→4x; building/overlay scales doubled to match; shader updated for 64px cell alignment
- **NPC speed doubling** — all NPC base speeds doubled (Farmer 100→200, Archer 100→200, Raider 115→230, Fighter 85→170, Miner 100→200, Crossbow 85→170) to match 64px atlas scale; separation radius 20→40, separation strength 50→100, arrival threshold 20→40; raider leash ranges doubled (400→800, 600→1200)

## 2026-03-02f

- **loot-cycle integration test** — 6-phase test: spawn archer+raiders → raider dies → archer carries equipment → returns home → deposits to TownInventory → equip item → verify stats change. Handles RNG edge case (no drops) gracefully. Also added TownInventory/MerchantInventory/NextLootItemId to test cleanup.

## 2026-03-02e

- **merchant building** — `BuildingKind::Merchant` (Economy, 50g, 200 HP, TownGrid), 1-per-town enforcement in build menu + `place_building()`. `MerchantInventory` resource with per-town `MerchantStock` (4-6 random items, 12h refresh timer). `merchant_tick_system` auto-refreshes stock. Inspector UI: rarity-colored stock with Buy buttons, Sell from TownInventory at half price, Reroll (50g). Save/load persisted via `#[serde(default)]`.

## 2026-03-02d

- **inventory UI tab** — `LeftPanelTab::Inventory` with `I` keybind (Factions moved `I`→`G`), `ControlAction::ToggleInventory`, top bar button, `InventoryParams` SystemParam. Tab shows selected NPC's D2 equipment slots with rarity-colored names + stat bonuses + Unequip buttons, and scrollable town inventory list with Equip buttons. Help catalog entry added.
- **NPC inspector rarity equipment** — bottom-panel inspector shows per-slot rarity-colored item names with stat %, hover tooltip shows `Slot: Name (Rarity +X%)`, carried equipment count in loot display

## 2026-03-02c

- **equip/unequip system** — `EquipItemMsg`/`UnequipItemMsg` messages + `process_equip_system`: moves items between TownInventory and NpcEquipment, handles ring slot preference (empty ring1 first), swaps occupied slots back to inventory, re-resolves stats via `re_resolve_npc_stats()` helper with proportional HP rescaling + GPU visual dirty

## 2026-03-02b

- **NpcEquipment (D2 slots) + DRY consolidation** — replaced 3 separate components (`EquippedWeapon`/`EquippedArmor`/`EquippedHelmet`) with unified `NpcEquipment` component (10 slots: helm/armor/weapon/shield/gloves/boots/belt/amulet/ring×2). `EquipmentSlot` expanded to 9 D2 variants; sprite-visible slots (helm/armor/weapon/shield) with GPU layers, stat-only slots (gloves/boots/belt/amulet/ring) with bonus aggregation. GPU equip stride 24→28, LAYER_COUNT 7→8, shader slot*6u→slot*7u for shield layer. `resolve_combat_stats()` now takes weapon_bonus/armor_bonus and applies damage/max_health multipliers. 3 equipment queries → 1 across gpu.rs, save.rs, game_hud.rs. Save/load backward compat with legacy weapon/helmet/armor fields.

## 2026-03-02a

- **authority safety hardening** — `attack_system` liveness check changed from `gpu_state.health` (GPU readback, can be 1+ frames stale) to `entity_map.get_npc().dead` (ECS authoritative); `ManualTarget::Npc` dead check also migrated to ECS; eliminated redundant double `get_npc` lookup. `building_tower_system` (fountain + player towers) now re-validates GPU `combat_targets` candidates via ECS: target must exist in EntityMap, not dead, and enemy faction. All docs aligned to authority.md contract — corrected stale claims in combat.md, concepts.md, gpu-compute.md, messages.md, resources.md
- **roadmap cleanup** — completed stages 17 (Generic Growth) and 23 (Tech Trees) moved to completed.md; remaining stages renumbered 17-29 with cross-references updated
- **arrow shoot SFX toggle** — `sfx_shoot_enabled` setting (default off) gates ArrowShoot SFX; checkbox in pause menu Audio tab; persisted in UserSettings

## 2026-03-01w

- **per-stat tower auto-buy** — `auto_upgrade: bool` replaced with `auto_upgrade_flags: Vec<bool>` for per-stat auto-buy control; tower upgrade popup window (`tower_upgrade_window`) with per-stat upgrade buttons and individual auto-buy checkboxes; `auto_tower_upgrade_system` only buys flagged stats
- **tower/fountain inspector** — kills/xp/level always shown (was hidden when kills==0); XP-to-next displayed like NPCs (`XP: 0/100`)
- **gold mine name consistency** — gold mine inspector and hover now use `gold_mine_name()` display names matching policy panel (was showing raw slot numbers)
- **main menu spacing** — uniform 8px vertical spacing between all menu buttons

## 2026-03-01v

- **per-tower upgrades** — each tower now has its own `upgrade_levels: Vec<u8>` and `auto_upgrade: bool` on `BuildingInstance`; tower inspector shows resolved per-instance stats (HP, Attack, Range, AtkSpd, ProjSpd, ProjLife, HpRegen) with upgrade buttons and auto-buy checkbox; `resolve_tower_instance_stats()` applies XP level bonus (+1%/level) and per-stat upgrade multipliers from `TOWER_UPGRADES`; `auto_tower_upgrade_system` runs each game-hour for auto-buy towers; `PlacedBuilding` saves/loads upgrade_levels and auto_upgrade with serde default for backward compat
- **HP regen upgrade** — `UpgradeStatKind::HpRegen` added to `MILITARY_RANGED_UPGRADES`, `MILITARY_MELEE_UPGRADES`, and `TOWER_UPGRADES`; `CachedStats.hp_regen` field wired through `resolve_combat_stats` (+0.5 HP/s per level for NPCs); `npc_regen_system` heals NPCs with hp_regen > 0 each frame; towers get +2.0 HP/s per level via `building_tower_system` regen tick
- **SFX spatial margin zero** — SFX viewport margin reduced from 200 to 0 world units; only onscreen events play sounds

## 2026-03-01u

- **initial mining radius fix** — `initial_mining_radius()` now sets radius to nearest mine distance + 50px margin (exactly 1 mine in range) instead of rounding up to 300px steps with a 2000px floor; returns 0 if no mines exist; applies to both player and AI towns at world gen
- **AI miner targeting** — miner target floor changed from 1 to `mines_in_radius` (at least 1 miner per in-radius mine); `ExpandMiningRadius` now requires all in-radius mines to be staffed before expanding
- **gold mine names** — mines display as "Gold Mine A/B/C..." instead of raw slot numbers in inspector, miner home links, and mining policy UI
- **AI Manager UI position** — moved AI Manager section to top of Policies tab (before General), since it's the most impactful toggle

## 2026-03-01t

- **tower XP, kills, and loot** — towers and fountains now earn XP (+100), kill count, and loot when they last-hit an NPC; same `level_from_xp()` and `npc_def(job).loot_drop` as NPC killers (DRY); loot deposited directly to `FoodStorage`/`GoldStorage` for the tower's town with `SetDamageFlash` visual feedback; tower inspector shows kills/level/xp when kills > 0; `BuildingInstance` gained `kills: i32` and `xp: i32` fields, saved/loaded via `PlacedBuilding`; `death_system` food/gold storage upgraded to `ResMut`
- **upgrade cost clarity** — `format_upgrade_cost` now shows explicit `"N food, N gold"` labels instead of cryptic `"100+50g"` format; matches existing wall upgrade cost style; expansion cost uses same clear format
- **roadmap cleanup** — removed accumulated "Recent move:" lines from Completed section; collapsed completed stages and checked-off items into concise references to completed.md

## 2026-03-01s

- **stamina upgrades** — `UpgradeStatKind::Stamina` added to all 4 job upgrade arrays (military ranged, military melee, farmer, miner) with MoveSpeed level 1 prereq; `-10% energy drain per level` using CooldownReduction formula `1/(1+lv*0.10)`; `CachedStats.stamina` field wired through `resolve_combat_stats` → `energy_system` drain multiplier; AI weights per personality (economic AI values stamina most for farmers/miners)
- **player AI manager** — faction 0 town gets an `AiPlayer` registered at world gen with `active: false`; Policies tab → AI Manager section with enable toggle, auto-build/auto-upgrade checkboxes, personality picker, road style picker; `build_enabled`/`upgrade_enabled` flags gate Phase 1 (building) and Phase 2 (upgrade) independently in `ai_decision_system`; `AiPlayerSave` persists new fields with `default_true` for backward compat; `FactionsParams.ai_state` upgraded from `Res` to `ResMut`
- **SFX dedup fix** — spatial camera culling now runs BEFORE per-kind dedup in `play_sfx_system`; previously off-screen events consumed the dedup slot, causing on-screen sounds to be silenced
- **SFX volume default** — default `sfx_volume` reduced from 0.5 to 0.15

## 2026-03-01r

- **arrow shoot SFX** — `fire_projectile` emits `PlaySfxMsg::ArrowShoot` with shooter position on successful fire; covers all 4 call sites (NPC→building, NPC→NPC, fountain tower, player tower) with zero duplication; `attack_system` and `building_tower_system` pass `sfx_writer` through
- **removed hit SFX** — deleted `SfxKind::Hit` variant and kenney wood impact sounds; `process_proj_hits` no longer emits SFX or reads `GpuReadState`; removed `assets/sounds/sfx/kenney-impact-sounds/`

## 2026-03-01q

- **boat loot_drop crash fix** — `Job::Boat` had `loot_drop: &[]` (empty), causing `% drops.len()` division by zero in `death_system` when something killed a boat; added food loot drop (1-3) so boats drop salvage when destroyed
- **death SFX** — NPC death emits `PlaySfxMsg::Death` with spatial position from GPU state; 24 death groan variants loaded at startup (skipping variant 2); `DeathResources` gained `sfx_writer` + `gpu_state` fields

## 2026-03-01p

- **boat as proper NPC entity** — boat is now spawned via `SpawnNpcMsg` with `Job::Boat` (index 6) instead of raw GPU slot writes; registered in `entity_map` so `build_visual_upload` renders it correctly; proper cleanup at disembark (entity despawn + unregister_npc + free slot); `NpcDef` gained `atlas: f32` field (0.0 for character NPCs, `ATLAS_BOAT` for boat); `materialize_npc` uses `def.atlas` instead of hardcoded `0.0`
- **normalized GPU health buffer** — GPU health buffer now stores normalized 0.0–1.0 values instead of raw HP; `SetMaxHealth` message sets per-slot max health for normalization; `SetHealth` and `ApplyDamage` divide by `max_healths[idx]`; render shader no longer divides by 100.0; `materialize_npc` and `place_building` emit `SetMaxHealth` before `SetHealth`; upgrade and death HP-scaling paths emit `SetMaxHealth` for correct normalization after max HP changes
- **hit SFX** — projectile impacts play random wood-impact sound variants (5 variants from kenney impact sounds); `play_sfx_system` with spatial camera culling (viewport margin check, zoom suppression at scale > 2.0), max 1 per SfxKind per frame via discriminant dedup; `load_sfx` startup system loads variant handles into `GameAudio.sfx_handles`; `PlaySfxMsg` gained `position: Option<Vec2>` field
- **generalized auto-tile** — wall-specific auto-tile system replaced with kind-agnostic `autotile_variant`/`update_autotile_around`/`update_all_autotile` functions; `BuildingDef` gained `autotile: bool` field; Road now uses auto-tile (`TileSpec::External` sprite strip + `autotile: true`); constants renamed `WALL_*` → `AUTOTILE_*`; `autotile_col`/`autotile_order`/`autotile_total_extra_layers` helpers for atlas column computation; `build_building_atlas` loops over all autotile-enabled kinds
- **town destruction cleanup** — fountain death now removes all roads belonging to the town and restores dirt cells to original terrain via `clear_town_roads_and_dirt`; `WorldCell` gained `original_terrain: Biome` field (natural terrain before `stamp_dirt`); saved/loaded with backward compat (old saves fallback to current terrain)
- **endless-mode test simplification** — deleted phases 6 and 14 (migrating-flag checks on transient NPC state), renumbered 16 → 14 phases; removed `disembarked` field from `MigrationGroup`; bumped raider phase timeouts for cumulative elapsed time
- **migration settlement terrain signal** — replaced `TilemapSpawned = false` hack with proper `TerrainDirtyMsg` + `BuildingGridDirtyMsg` dirty signals on migration settlement

## 2026-03-01o

- **unified place_building** — merged `place_building` (runtime), `place_building_instance` (data-only), `spawn_building_entities` (batch ECS+GPU), and `materialize_generated_world` into a single `place_building` function with optional `BuildContext` parameter; `ctx: Some(BuildContext)` enables runtime validation (cell checks, cost deduction, construction timer, wall auto-tile, dirty signals); `ctx: None` creates buildings at full HP for world-gen, save/load, migration, and tests; every code path now creates GPU slot + BuildingInstance + ECS entity + GPU updates in one call
- **migration invisible buildings fix** — migration settlement now calls `place_building` which creates ECS entities and GPU state; previously `place_buildings` only called `place_building_instance` (data-only), so settled towns had building data but no visual presence — dirt sprites flashed and buildings were invisible
- **deleted materialization system** — removed `materialize_generated_world`, `materialize_test_world`, `TestWorldMaterializeState` resource, and `reset_test_world_materialization_state`; buildings are now fully created at placement time with no deferred entity spawn pass

## 2026-03-01n

- **DamageMsg EntityUid migration** — `DamageMsg.entity_idx: usize` replaced with `DamageMsg.target: EntityUid` for stable identity; `damage_system` resolves UID→slot via `entity_map.slot_for_uid()`; all 7 sender sites updated (combat.rs attack_system/process_proj_hits, ai_player.rs waypoint prune, ui/mod.rs demolish/debug destroy, test); `process_proj_hits` now takes `Res<EntityMap>` parameter; eliminates class of bugs where raw slot disagrees with entity identity after slot recycling
- **NPC stat differentiation** — each NPC type now has unique base HP and speed instead of uniform 100/100: Farmer 60hp, Crossbow 70hp/85spd, Archer 80hp, Miner 80hp, Raider 120hp/115spd, Fighter 150hp/85spd; creates meaningful combat roles (glass-cannon ranged, tanky melee, fast raiders)
- **endless-mode test fix (double materialization)** — test setup no longer calls `materialize_generated_world` directly; common `materialize_test_world` system handles it once, preventing duplicate ECS building entities that caused Phase 2 to see full-HP ghost fountains
- **migration disembark race fix** — `endless_system` SETTLE check now distinguishes "NPCs not spawned yet" (`found == 0`) from "all NPCs dead" (`found > 0, count == 0`); prevents false wipeout declaration on the same frame as disembark when `SpawnNpcMsg` hasn't been processed by `spawn_npc_system` yet

## 2026-03-01m

- **gpu slot allocator lifecycle** — `GpuSlotPool` now owns full GPU state lifecycle: `alloc_reset()` queues pending resets (all 9 GPU fields zeroed to safe defaults), `free()` queues pending frees (hide + health/speed/flags zeroed); `populate_gpu_state` drains both queues before processing `GpuUpdateMsg` events; removed Deref/DerefMut to inner SlotPool, all access through explicit methods; eliminates stale GPU state on slot reuse (root cause: buildings on reused NPC slots inherited speed=100.0, causing phantom movement)
- **tower debug info** — building inspector Tower arm shows LastHitBy, combat_target, targeted-by count, GPU raw speed, GPU readback position; Copy Debug Info includes all fields

## 2026-03-01l

- **building construction time** — all runtime-placed buildings (player + AI) now have a 10-second construction period (at 1x speed, scales with time_scale); buildings start at 0.01 HP scaling to full, sprite progressively reveals bottom-to-top via shader clip on health < 1.0; spawner dormant during construction (respawn_timer = -1.0), growth system skips under-construction farms/mines; `under_construction: f32` on `BuildingInstance`, `construction_tick_system` in Step::Behavior before growth_system; world-gen buildings appear instantly; save/load persists construction state
- **tower inspector** — added `BuildingKind::Tower` match arm in building inspector showing range, damage, cooldown from `TOWER_STATS` constant + HP progress bar; towers previously showed no per-type info
- **construction inspector** — building inspector shows yellow "Under Construction" label + progress bar with percentage and time remaining; per-type details hidden during construction
- **kill stats fix** — `death_system` now properly attributes kills by faction using `last_hit_by` slot → faction lookup via EntityMap; `archer_kills` only counts enemies killed by player faction, `villager_kills` only counts player NPCs killed by enemies

## 2026-03-01k

- **tower building** — added `BuildingKind::Tower` with `DisplayCategory::Tower` tab in build menu; player-buildable defensive tower (50 food, 1000 HP) auto-shoots nearest enemy within 250px (10 dmg, 2s cooldown); uses `tower-1.png` sprite; cooldowns tracked via `TowerState.tower_cooldowns` HashMap keyed by slot; save/load via `save_key: "towers"`
- **fire_projectile DRY helper** — extracted `fire_projectile()` in combat.rs replacing 3 copies of `ProjGpuUpdate::Spawn` boilerplate across `attack_system` (building + NPC targets) and `building_tower_system` (fountain); tower loop reuses same helper
- **save load 0x speed** — `time_scale` clamp on load changed from `.max(0.5)` to `.max(0.0)` to preserve paused state; `paused` restored as `time_scale <= 0.0` instead of hardcoded `false`
- **speed controls paused state** — speed-up from paused sets 0.5x + unpauses; speed-down to 0x sets `paused = true`

## 2026-03-01j

- **tutorial update** — expanded from 20 to 24 steps: added Walls, Roads, Save/Load, and Controls rebinding steps; replaced hardcoded key names with dynamic `key_label_for_action()` so tutorial text reflects player's actual keybindings; step 2 mentions build menu Economy/Military tabs
- **game over screen** — player fountain destruction triggers `UiState.game_over` flag via `death_system`; pauses game and shows dimmed overlay with "Game Over" window displaying session stats (days survived, NPCs alive/lost, kills, food, gold); Play Again / Keep Watching / Exit to Main Menu buttons; dim overlay uses `Order::Background` so buttons are clickable
- **inspector cleanup** — removed DirectControl toggle button and dead `atk_type` code from NPC inspector; moved Faction/Home links above Loot/CarriedGold for better layout; mine assignment UI returns `InspectorAction` so clicking assigned mine navigates to the mine building
- **restart tutorial moved** — moved Restart Tutorial button from main menu to Settings panel Interface tab (accessible from both pause menu and main menu settings)

## 2026-03-01i

- **build menu category tabs** — added Economy/Military tabs to build bar using `DisplayCategory` from `BUILDING_REGISTRY`; `BuildMenuContext` gained `build_tab` field; tab switch clears selection if selected building belongs to other category; Economy shows Farm/Farmer Home/Miner Home/Road, Military shows Waypoint/Archer Home/Crossbow Home/Fighter Home/Wall
- **hard difficulty 20 towns** — increased Hard preset from 10 to 20 AI builder and raider towns
- **ai builder tooltip fix** — changed "friendly" to "rival" in AI Builder Towns tooltip

## 2026-03-01h

- **raider forage hours slider** — replaced `raider_passive_forage` boolean checkbox with `raider_forage_hours` f32 slider (0=off, 1–24 hours per 1 food); difficulty presets set Easy=12h, Normal=6h, Hard=3h; `raider_forage_system` now timer-based using `RaiderState.forage_timers` accumulation instead of flat hourly rate; SETTINGS_VERSION bumped to 13

## 2026-03-01g

- **default video settings** — changed default resolution from 1280x720 to 1920x1080 and default fullscreen from off to on
- **main menu exit button** — added Exit button to main menu; moved Debug Tests button next to it
- **autosave in settings** — moved autosave slider from main menu Options section to shared settings panel Interface tab; pause menu syncs `autosave_hours` to `SaveLoadRequest` live

## 2026-03-01f

- **endless mode always enabled** — removed endless mode checkbox and replacement strength slider from main menu; all difficulty presets now have `endless_mode: true`; save load forces `endless.enabled = true`; default settings `endless_mode` changed to `true`
- **main menu reorganization** — moved raider passive forage checkbox under AI Raider Towns indent; moved AI Think and NPC Think interval sliders from main menu Debug Options to settings panel Debug tab (live-synced to `AiPlayerConfig`/`NpcDecisionConfig` via pause menu system)
- **pause menu locals bundle** — combined `manual_save_name`, `manual_load_name`, `rebinding_action` into `PauseMenuLocals` struct to stay within Bevy 16-param system limit after adding `AiPlayerConfig`/`NpcDecisionConfig`

## 2026-03-01e

- **shared settings panel** — extracted `settings_panel_ui()` in `mod.rs` with `SettingsResponse` return struct; both pause menu and main menu call the same function; `PauseSettingsTab` gained `label()`/`title_subtitle()` methods; pause menu passes `Some(save/load names)` to show Save/Load tabs, main menu passes `None` to hide them
- **main menu settings** — added Settings button that opens floating window with full settings panel (Interface/Video/Camera/Controls/Audio/Logs/Debug); video/audio/winit side effects applied via pre/post snapshot comparison
- **fullscreen live toggle** — added `fullscreen` to pause menu change detection guard so toggling takes effect immediately without restart
- **jukebox state persistence** — added `jukebox_track`/`jukebox_paused` to UserSettings; `start_music` restores saved track and paused state on startup
- **left panel persistence** — added `left_panel_tab` (string) and `collapsed_sections` (Vec<String>) to UserSettings; `TRACKED_SECTIONS` list + `snapshot_collapsed_sections`/`restore_collapsed_sections` helpers; `tab_to_str`/`str_to_tab` converters

## 2026-03-01d

- **wire inspector NPC link action** — `inspector_content` now returns `Option<InspectorAction>` and bubbles up from `building_inspector_content`; `bottom_panel_system` calls `apply_inspector_action` to select NPC + jump camera on click; `SelectedBuilding` upgraded to `ResMut` in `BuildingInspectorData`
- **clickable home building link** — NPC inspector home coordinates rendered as `building_link` clickable link; looks up building slot via `entity_map.find_by_position(home_pos)`, clicking selects the home building and jumps camera

## 2026-03-01c

- **fix cross-town mine occupancy** — added `town_scoped` field to `WorksiteDef` (Farm=true, GoldMine=false); mine arrival, Priority 5 town validation, and claim repair now skip town check for non-town-scoped worksites; fixes miners being rejected from cross-town mines with "Mine full" despite no occupants
- **inspector clickable NPC links** — added `InspectorAction` enum + `npc_link`/`building_link`/`apply_inspector_action` helpers; building inspector spawner NPC name is now a clickable link that selects the NPC and jumps the camera; `BottomPanelData.selected` upgraded to `ResMut` for selection writes
- **fullscreen + video settings** — added `fullscreen: bool` to `UserSettings` with borderless fullscreen mode; new Video settings tab with resolution dropdown (disabled in fullscreen) + fullscreen checkbox; selection bracket width reduced (0.08→0.05); default lod_transition changed to 0.25

## 2026-03-01b

- **unified worksite occupancy (farm + mine)** — added `WorksiteDef` to `BuildingDef` in `BUILDING_REGISTRY` with `max_occupants`/`drift_radius`/`upgrade_job`/`harvest_item`; merged separate Working and MiningAtMine decision blocks into a single Priority 5 block driven by registry config; renamed `assigned_farm` → `occupied_building`, `work_position` → `target_building` throughout decision_system for clarity; mine arrival now uses `try_claim_worksite()` with max=5 cap (previously raw `claim()` with no occupancy check); flee/leash cleanup releases `occupied_building` for both farm and mine workers; removed unused `EntityMap.claim()` method

## 2026-03-01a

- **configurable controls + keybinding persistence** — added `ControlAction`/`ControlGroup` model in `settings.rs`, persisted `key_bindings` map in `UserSettings`, settings migration to `SETTINGS_VERSION=11`, and helpers for default/fallback/rebind-safe key parsing
- **new Controls settings tab** — pause menu settings now includes `PauseSettingsTab::Controls` with grouped action list, click-to-rebind flow, and reset-to-defaults button (`ESC > Settings > Controls`)
- **runtime input wiring to settings bindings** — replaced hardcoded keyboard checks in camera pan, panel toggles, squad target hotkeys, pause/time controls, and quick save/load with keybinding-driven lookups from `UserSettings`
- **docs sync for controls and settings tabs** — updated game README controls section and architecture docs to reflect default-vs-rebindable keys and the new Controls tab

## 2026-02-28n

- **fix projectiles hitting roads** — roads had `BUILDING_HITBOX_HALF = [16.0, 16.0]` and player faction, so enemy arrows collided with them despite `ENTITY_FLAG_UNTARGETABLE`; fix: zero half-size for roads in `push_building_gpu_updates`, and bind `entity_flags` buffer to projectile compute shader (new binding 17) with UNTARGETABLE skip in collision loop
- **flash-only visual upload split** — flash decay (damage flash fading) now writes `flash_only_indices` instead of `visual_dirty_indices`; `build_visual_upload` updates only the flash float in visual_data for these slots (skips full ECS query + equip rebuild); separate `equip_uploaded_indices` excludes flash-only slots from equip_data GPU upload, saving ~96B × flash_count per frame
- **coalescing gap tuning** — widened visual/equip gap thresholds: `GAP_VISUAL` 93→750 (24KB max waste/gap), `GAP_EQUIP` 31→250 (24KB max waste/gap); fewer `write_buffer` calls at ~4μs each outweighs small data overhead; `count_gap_ranges` profiler helper added for gap-based coalescing diagnostics

## 2026-02-28m

- **strict coalescing for GPU-authoritative buffers** — positions and arrivals are GPU-authoritative (compute shader updates them each frame); added `write_coalesced_exact_f32/i32` that merge only exactly-adjacent dirty indices (`saturating_add(1)` adjacency, no gap merging, no bulk fallback); debug-asserts sorted+deduped+bounds on all dirty indices; gap-based coalescing (`write_coalesced_f32/i32/u32`) retained for CPU-authoritative buffers (targets, speeds, factions, healths, flags, half_sizes); authority contract comments locked on `EntityGpuState` dirty fields and at extraction callsites
- **coalesce profiler counters** — `count_exact_ranges` helper tracks strict coalescing write count + actual uploaded bytes per frame for positions/arrivals; logged via `bevy::log::trace` when non-zero
- **coalesce safety tests** — `coalesce-movement` (2 phases): spawns 2 farmers, injects `SetPosition` on unused slot, verifies no NPC teleports; `coalesce-arrival` (2 phases): spawns 2 archers, waits for arrival, verifies arrival flag stable after unrelated activity; 7 unit tests for `count_exact_ranges` (empty/single/sparse/adjacent/stride-1/all-adjacent/gap-of-one)

## 2026-02-28l

- **coalesced GPU uploads** — replaced per-index `write_buffer` calls with `write_coalesced_f32/i32/u32` that merge pre-sorted dirty indices into contiguous ranges (one `write_buffer` per range); falls back to offset bulk write when dirty coverage exceeds 40% of the index window; gap thresholds tuned per stride for DX12 backend (~3μs per call overhead); dirty indices pre-sorted+deduped in `populate_gpu_state` so extract phase receives coalesce-ready data; removed now-redundant `dirty_positions`/`dirty_arrivals` bool flags
- **growth_system simplification** — reverted from `kind_slots()` two-pass iteration to single `iter_instances_mut()` pass with match on `BuildingKind::Farm`/`GoldMine`; precomputes per-town farm yield multiplier Vec to avoid repeated `town_levels()` + string lookup per farm; removed unused `EntityMap.kind_slots()` method
- **profiler tab caching** — added `Local<ProfilerCache>` that refreshes every 15 frames (~4 updates/sec at 60 FPS); amortizes 3 mutex locks, 2 HashMap clones, and 200K-element `top_offenders` array scan; renders only top 10 entries per section instead of all ~60 traced systems

## 2026-02-28k

- **per-dirty-index GPU uploads** — converted all remaining bulk `write_buffer` calls in `extract_npc_data` to per-dirty-index writes; speeds/factions/healths/entity_flags/half_sizes now track changed indices (like positions/arrivals/targets already did), uploading only changed bytes instead of full 80-160KB arrays per buffer
- **per-dirty visual/equip upload** — `extract_npc_data` now uploads only changed visual/equip slots via `visual_uploaded_indices` (populated by `build_visual_upload`), saving ~2.56MB/frame of unconditional GPU writes; full upload retained for startup/load via `visual_full_upload` flag
- **healing_system HashMap removal** — replaced per-frame `HashMap<i32, Vec<&HealingZone>>` allocation with direct access to `cache.by_faction` (already indexed by faction)
- **farm_visual_system cadencing** — runs every 4th frame instead of every frame (crop state changes slowly)
- **growth_system filtered iteration** — uses `EntityMap.kind_slots()` to iterate only Farm and GoldMine buildings instead of all 10K instances; added `kind_slots()` method to EntityMap

## 2026-02-28j

- **decision_system two-cadence bucket gate** — moved bucket gate to top of decision loop before any ECS queries or state reads; fighting NPCs gated every 16 frames (~267ms), non-fighting NPCs gated by adaptive `think_buckets`; reduces per-frame ECS lookups from `queries × npc_count` to `queries × (npc_count / bucket_count)` (~92% reduction at 10K NPCs)
- **farm reconciliation removal** — removed per-frame pre-scan that rebuilt 3 HashMaps (`farm_owner_counts`, `farm_owner_keep_slot`, `farm_owner_keep_rank`) by iterating all NPCs every frame; replaced with inline `occupant_count` checks that only run for the ~83 NPCs processed per bucket tick
- **position hoisting** — `npc_pos: Option<Vec2>` computed once per NPC after bucket gate replaces ~15 scattered `positions[idx * 2]` reads throughout the decision loop
- **manual timings cleanup** — removed `scope()`/`TimerGuard` RAII pattern and `Res<SystemTimings>` parameter from ~40 system functions across ~20 files; removed decision sub-profiling boilerplate (~60 lines of timing/counter accumulators); kept render-world atomic timings (not capturable by tracing) and tracing-based auto-capture
- **profiler UI simplification** — removed Manual Timings and Stats sections from profiler; added Render Pipeline section for 8 render-world timings; profiler now shows Frame time, Game Systems (auto-captured via tracing), Engine Systems, and Render Pipeline

## 2026-02-28i

- **EntityUid stable identity system** — introduced `EntityUid(u64)` as the canonical stable identity for all gameplay cross-references, replacing raw `GpuSlot(usize)` indices which suffered from ABA hazards due to LIFO slot recycling; `NextEntityUid` resource allocates monotonically increasing UIDs (0 reserved as "none"); `EntityMap` maintains bidirectional UID maps (`uid_to_slot`/`slot_to_uid`/`uid_to_entity`/`entity_to_uid`) with debug-build bijection assertions
- **ABA slot-reuse bug fixed** — `AiSquadCmdState.building_gpu_slot` → `building_uid: Option<EntityUid>`; wave end condition now correctly detects destroyed buildings (UID resolves to None after unregister, regardless of slot reuse); `slot-reuse-wave` test Phase 4 inverted to confirm fix
- **BuildingInstance.npc_uid** — replaced `npc_gpu_slot: i32` sentinel (-1) with `npc_uid: Option<EntityUid>`; spawner respawn pre-allocates UIDs via `NextEntityUid.next()` and passes through `SpawnNpcMsg.uid_override` for same-frame consistency
- **NpcWorkState UID migration** — `occupied_slot`/`work_target` (Option\<usize\>) → `occupied_building`/`work_target_building` (Option\<EntityUid\>); behavior system resolves UID→slot at loop entry, converts slot→UID at writeback; death cleanup resolves UIDs before releasing occupancy
- **Squad.members UID migration** — `Vec<usize>` → `Vec<EntityUid>`; squad cleanup, recruit, dismiss, and box-select all convert between slots and UIDs at boundaries; economy auto-recruit uses `uid_for_slot` on push
- **Save/load UID support** — `NpcSaveData.uid`, `SpawnerSave.npc_uid`, `SquadSave.member_uids`, `SaveData.next_entity_uid` fields added with `#[serde(default)]` for old-save backward compatibility; old saves get deterministic UID assignment during load + post-spawn squad member fixup

## 2026-02-28h

- **EntitySlot→GpuSlot, EntitySlots→GpuSlotPool renames** — renamed `EntitySlot` component to `GpuSlot` and `EntitySlots` resource to `GpuSlotPool` across all source, tests, and docs for clarity; no logic changes
- **shader comment annotations** — added educational section headers, mental model comments, and inline annotations to `npc_compute.wgsl`, `npc_render.wgsl`, and `projectile_compute.wgsl`; no logic changes
- **dirt roads sprite** — added `dirt_roads_131_32.png` atlas (32px tile sheet)

## 2026-02-28g

- **heartbeat-only squad commander** — removed `AiSquadsDirtyMsg` message type; `ai_squad_commander_system` now wakes purely on a 2-second heartbeat timer instead of message+heartbeat dual gating; removed message struct, all producers (spawn.rs, health.rs, DirtyWriters), consumer parameter, and registration
- **slot-reuse-wave test** — new integration test reproducing ABA bug in `SlotPool`: AI wave targets a player farm → farm destroyed → freed slot reused by new building → `resolve_building_pos` finds wrong building → wave never ends; 5-phase test (wave dispatch → destroy → slot reuse → verify wave persists → report)

## 2026-02-28f

- **building inspector GPU diagnostics** — building Copy Debug Info and inline inspector now show GPU raw state (position/target/health/faction/flags/sprite via `EntityGpuState`), slot allocator status (`entity_slots.free` membership + pool metrics), EntityMap cross-references (building instance + NPC entry + entity mapping), and selection overlay expected values; `BuildingInspectorData` gains `EntitySlots` field

## 2026-02-28e

- **GPU selection brackets** — moved selection overlay from egui CPU painter to GPU render pipeline; new `SelectionBracket` StorageDrawMode with `SelectionInstance` (slot/color/scale/y_offset), `vertex_selection` entry point reads `npc_positions[slot]` from storage buffer, procedural corner brackets rendered in fragment shader at `atlas_id=9`; cyan for selected NPC, gold for selected building, green for DirectControl group (capped at 200); LOD-aware (discarded below `lod_zoom`); removed `selection_overlay_system` + `draw_corner_brackets` from `game_hud.rs`
- **tabbed pause menu** — redesigned pause menu from flat collapsible to tabbed layout with `PauseSettingsTab` enum (Interface/Camera/Audio/Logs/Debug/SaveGame/LoadGame); left sidebar tab list + right scrollable content panel; 820×520 min size
- **named save/load** — added `named_save_path()` (sanitized filename → `Documents/Endless/saves/<name>.json`), `SaveLoadRequest.save_path` for save-to-path, `list_saves()` directory listing, manual save/load UI in pause menu SaveGame/LoadGame tabs
- **interface text size setting** — new `interface_text_size` (default 16.0) in `UserSettings` + `apply_interface_text_size` system sets global egui text styles (Heading/Body/Button/Monospace/Small) from setting
- **healing flag toggle fix** — `healing_system` now properly sets `NpcFlags.healing = false` when HP reaches cap, emitting `MarkVisualDirty` to clear heal halo sprite
- **main menu simplification** — removed world gen style selector (always Continents), capped per-town sliders to 10, stripped Miner/Fighter/Crossbow homes from player-facing menu via `strip_disabled_home_jobs`
- **WorldGrid init in all tests** — `materialize_test_world` now initializes WorldGrid (25×25, 32px cells) when `width == 0`, ensuring building atlas renders correctly in all test scenes

## 2026-02-28d

- **randomized AI road placement** — added `RoadStyle` enum (None/Cardinal/Grid4/Grid5) randomly assigned per AI town at creation, independent of personality; decoupled road patterns from `AiPersonality`; threaded `road_style` through all building/waypoint/scoring functions; persisted in save files with Grid4 default for backward compat
- **projectile self-collision skip** — GPU projectile compute now skips shooter entity (`entity_idx == proj_shooters[i]`), preventing projectiles from colliding with their source; tower `building_tower_system` now passes `bld_slot` as shooter instead of `-1`
- **GPU default speed fix** — `EntityGpuState` default speeds changed from `100.0` to `0.0` so uninitialized entity slots don't move in the spatial grid
- **archer patrol test uses spawner-driven setup** — test now places ArcherHome buildings instead of manually spawning NPCs, matching normal gameplay flow

## 2026-02-28c

- **`0x` speed now behaves as pause** - added `GameTime::is_paused()` (`paused || time_scale <= 0.0`) and switched behavior/combat/movement/economy/energy gating plus HUD pause label to use unified paused semantics
- **time controls updated for explicit `0x` state** - `-` now steps from `0.25x` to `0x`, `+` from `0x` returns to `0.25x`, and `Space` unpauses from `0x` by restoring `1.0x`
- **farm claim fairness + loser retarget fix** - duplicate farm owner resolution now prefers incumbent `Activity::Working` farmers over `GoingToWork` contenders; losing farmers immediately target home on demotion to `Idle` to avoid stale farm targets in debug/inspector

## 2026-02-28b

- **pause parity between game and test scenes** - `AppState::Running` now uses the same pause semantics in both paths, so behavior/combat/movement do not advance decisions or retarget while paused
- **documented shared test world setup contract** - test scenes continue to materialize world/buildings through the same shared helper as main gameplay startup to prevent setup drift

## 2026-02-28a

- **shared world materialization for all test scenes** — added common `materialize_test_world` hook in `tests/mod.rs` (first `Update` in `AppState::Running`, before `Step::Behavior`) that calls `world::materialize_generated_world(...)`; this makes test building/entity/GPU spawn path match main game startup and removes per-test setup drift
- **remove one-off test materialization path** — `archer_tent_reliability` no longer manually calls `spawn_building_entities`; it now relies on shared harness setup like all other tests

## 2026-02-27r

- **town grid world-edge caps** — `TownGrid` now stores `min_row_cap`/`max_row_cap`/`min_col_cap`/`max_col_cap` clamping buildable bounds to world grid boundary; `build_bounds()` uses per-axis caps instead of symmetric `MAX_GRID_EXTENT`; caps computed at world gen, endless migration (`create_ai_town`), and save load (`sync_town_grid_world_caps`); prevents AI from placing buildings outside world edges
- **focus camera on all test scenes** — added `camera_q` to `TestSetupParams` with `focus_camera()` helper; all 21 integration tests now center camera on their scene at setup

## 2026-02-27q

- **candidate-driven healing pipeline** — replaced O(50k) mutable ECS iteration with `ActiveHealingSlots` resource tracking only NPCs in healing zones; cadenced enter-check (slot % 4 bucketing via `npcs_for_town()`) + every-frame sustain-check with hysteresis radii (10% exit buffer); `HashMap<i32>` faction→zone lookup; starvation HP cap moved from `healing_system` to `starvation_system` with always-clamp for save/load safety; ~1-3ms → <0.1ms at 50k NPCs

## 2026-02-27p

- **fix food sprite persisting after delivery** — `arrival_system` delivery path (Returning→Idle) was missing `MarkVisualDirty` emit, so event-driven `build_visual_upload` never cleared the carried-food sprite; added dirty signal at delivery writeback

## 2026-02-27o

- **fix double NPC spawn at game start** — removed `spawn_npcs_from_spawners` which spawned NPCs immediately at world gen; combined with `respawn_timer: 0.0` on new homes, `spawner_respawn_system` would spawn a second NPC on the first hour tick (4 archer homes → 8 archers); homes now start with `respawn_timer: 0.0, npc_slot: -1` and `spawner_respawn_system` handles all initial spawns on the first hour tick
- **always show loot in inspector + copy debug** — CarriedGold and Loot lines now always displayed (show 0/none when empty) instead of being conditionally hidden

## 2026-02-27n

- **comprehensive NPC Copy Debug Info** — added full NpcFlags dump (healing/starving/direct_control/migrating/at_dest), raw Activity debug repr with fields (e.g. `Returning { loot: [(Food, 5)] }`), Returning loot contents, PatrolRoute current/total; replaces individual Starving+DirectControl lines with consolidated Flags line

## 2026-02-27m

- **decision-frame budgeting** — adaptive think-bucket count caps Tier 3 NPC decisions at `max_decisions_per_frame` (default 300) regardless of population; `think_buckets = max(interval × 60, npc_count / max_per_frame)` — at 50K NPCs this increases buckets from 120→167 (~300/frame, effective interval ~2.8s); at low counts the interval dominates (no change); new `NpcDecisionConfig.max_decisions_per_frame` field; new profiling counters `decision/think_buckets` and `decision/npcs_per_bucket`

## 2026-02-27l

- **event-driven visual upload** — `build_visual_upload` no longer rebuilds 100K NPC+building visual/equip buffers every frame; `NpcVisualUpload` is now persistent across frames with only dirty slots updated; new `GpuUpdate::MarkVisualDirty { idx }` variant flows through existing message channel; `EntityGpuState` tracks `visual_dirty_indices` (populated by SetSpriteFrame, SetDamageFlash, Hide, MarkVisualDirty, and flash decay) and `visual_full_rebuild` flag (defaults true for startup/load); full rebuild uses query-first ECS iteration (`Without<Building,Dead>` for NPCs, `With<Building>` for buildings); dirty path uses sort+dedup then EntityMap slot lookup with stale/unmapped slots cleared to sentinels; `Activity::visual_key()` helper distinguishes visual-relevant activity states (Resting, Returning with Gold/Food/empty); decision_system emits dirty only when visual key changes; arrival_system emits dirty on farm delivery; healing_system emits dirty on healing flag toggle; death_system emits dirty on loot drop activity change; expected saving: ~3-7ms/frame at 50K scale in steady state

## 2026-02-27k

- **fix add_instance spatial index ordering** — `spatial_insert` was called before `instances.insert`, so kind-filtered spatial buckets (`spatial_kind_town`, `spatial_kind_cell`, `spatial_bucket_idx`) were never populated on first insert; new buildings were invisible to `find_nearest_worksite` until `rebuild_spatial()` ran; fixed by saving position, inserting instance first, then calling spatial_insert
- **build_visual_upload event-driven clearing** — replaced O(entity_count) full sentinel fill (1.92M f32 writes at 60K) with event-driven hidden-slot clearing via `hidden_indices`; `GpuUpdate::Hide` now clears `sprite_indices` and `flash_values` immediately to prevent ghost visuals on slot reuse; building loop replaced with `iter_instances()` (iterates only actual buildings, not all 60K slots); building equip blocks wiped to sentinels to prevent stale NPC overlay data
- **GPU target dirty tracking** — added `target_dirty_indices` to `EntityGpuState` with dedup; `extract_npc_data` uses `write_dirty_f32` instead of `write_bulk` for the targets buffer (~480KB saved per frame); full-upload fallback on first frame or buffer resize
- **cooldown_system optimization** — removed per-NPC `entity_map.get_npc()` HashMap lookup (30K lookups/frame); query filters `(Without<Building>, Without<Dead>)` replace EntityMap guard
- **energy_system optimization** — removed EntityMap dependency entirely; Activity queried directly in main query tuple with `(Without<Building>, Without<Dead>)` filters
- **damage_system sampling gated** — health sample collection behind `damage_count > 0`; `health_samples.clear()` every frame to prevent stale debug data
- **ManualTarget clone removal** — `attack_system` matches ManualTarget by reference instead of `.cloned()` per-NPC per-frame

## 2026-02-27j

- **indexed worksite query** — replaced brute-force worksite scans in `decision_system` with kind-filtered spatial cell-ring expansion: `EntityMap` now maintains per-cell `(kind, town, cell)` and `(kind, cell)` buckets with `SpatialBucketRef` back-index for O(1) swap-remove; new `find_nearest_worksite()` with min-order tuple scoring, cell-ring expansion (r=0 center first, doubling), and `WorksiteFallback` policy (TownOnly/AnyTown); new `try_claim_worksite()` authoritative claim function (validates kind+town+occupancy before incrementing); `find_farmer_farm_target()` now delegates to `find_nearest_worksite` instead of `for_each_nearby` (only visits Farm buildings, not all 20k); miner mine selection replaced `iter_kind_for_town` linear scan + global `iter_kind` fallback with spatial `find_nearest_worksite` (AnyTown fallback); debug validation in `validate_spatial_indexes()` verifies bucket/back-index consistency
- **worksite instrumentation** — added `decision/ws_queries`, `decision/ws_fallbacks`, `decision/ws_stale` profiling counters to decision_system (gated by profiling flag)

## 2026-02-27i

- **simplify waypoint ring placement** — replaced personality-specific block-corner algorithm with simple perimeter walk: always includes 4 corners, fills non-road-slot cells with min 5 Manhattan spacing, works identically for all personalities at all grid sizes; deleted unused `road_spacing()` method
- **fix waypoint cleanup never triggering** — removed stale `is_road_slot` filter from `find_waypoint_slot()` (conflicted with new ring that includes corner road slots); changed completeness gate in `sync_town_perimeter_waypoints()` to treat blocked ideal slots (occupied by other buildings) as "covered" — prevents a single blocked slot from permanently disabling pruning
- **fresh spawn work_target fix** — `materialize_npc()` no longer sets `work_target` or `GoingToWork` on fresh spawns; only save/restore path restores explicit work targets; prevents pre-claimed farm reservations from spawn that conflict with behavior system self-claim
- **archer-tent-reliability test** — new 5-phase test: archer target lock on enemy tent, projectile activity, sustained tent damage, destruction

## 2026-02-27h

- **farm reservation lifecycle hardening** — Working farmer safety invariant now validates farm slot existence, kind, town ownership, and occupancy before allowing work; retroactively claims `work_position` if `assigned_farm` is missing; GoingToWork arrival uses `occupant_count` with owner-aware threshold (`>1` if self, `>=1` if other) and claims before harvest check; end-of-decide invariant auto-releases `assigned_farm` for farmers not in Working/GoingToWork (prevents ghost reservations)
- **remove pre-claim at spawn** — `spawner_respawn_system` and `spawn_npcs_from_spawners` no longer call `entity_map.claim(work_slot)` — farmers self-claim via behavior system, eliminating orphaned reservations from respawned farmers that die before reaching their farm
- **inspector NpcWorkState debug** — Copy Debug Info now includes `occupied_slot` and `work_target` from NpcWorkState

## 2026-02-27g

- **farmer local farm targeting** — replaced global `iter_kind_for_town` full-scan and `find_nearest_free` with `find_farmer_farm_target()`: expanding-radius local search (400→6400px, doubling) via `EntityMap.for_each_nearby()` with priority ordering (ready > growth progress > distance); proper `assigned_farm` claim/release lifecycle at every transition point (retarget, harvest, idle, death); farm contention safety guard in Working state ejects farmers when `occupant_count > 1`; removed unused `find_nearest_free` import
- **fix death double-release** — death_system now guards against releasing `work_target` when it equals `occupied_slot` (same slot released twice caused negative occupant counts)
- **authority contract** — new `docs/authority.md` defining GPU readback vs ECS source-of-truth rules; roadmap updated with authority hardening items for combat and tower systems

## 2026-02-27f

- **fix combat: use ECS faction for NPC target validation** — attack_system was reading target NPC faction from GPU readback (`gpu_state.factions`), which can return stale -1 on throttled frames; this caused `target_faction < 0` → skip, silently preventing all NPC-vs-NPC combat; fixed by reading faction from `entity_map.get_npc(ti).faction` (ECS source-of-truth) with GPU readback as fallback
- **combat test hardening** — reset SquadState in test setup (prevents stale squad config from earlier tests); set explicit policies (archer_flee_hp=0.05, recovery_hp=0.05) to prevent heal/flee breakoff; focus camera on combat area; cleanup: `entity_map.clear_npcs()` and `*squad_state = Default::default()` in test teardown
- **NPC inspector combat diagnostics** — Copy Debug Info now includes: CombatState, ManualTarget, Squad.hold_fire/patrol_enabled/rest_when_tired, town policies (archer_aggressive/leash/flee_hp, prioritize_healing, recovery_hp), GPU.combat_target[slot] with full target resolution (NPC slot/faction/hp/pos/dead, or Building kind/faction/pos)
- **building faction tint** — enemy buildings now use a subtle 30% faction tint instead of full recolor (more readable at a glance)
- **docs/messages.md** — updated staleness budget: documents always-on vs throttled readback staleness, canonical authority reference to authority.md

## 2026-02-27e

- **decision_system phase 2: eliminate archetype churn + conditional writeback** — replaced optional `AssignedFarm` + `WorkPosition` components with always-present `NpcWorkState { occupied_slot: Option<usize>, work_target: Option<usize> }` on all NPCs; eliminates per-entity `commands.insert/remove` archetype moves in decision_system; removed `Commands` param from decision_system entirely; eliminated per-patrol-NPC `Vec<Vec2>` clone by reading patrol route data inline at 2 usage sites; added conditional writeback via original-value comparison (captures discriminant/scalar originals at loop top, skips `get_mut()` for unchanged fields — most NPCs exit early with no state changes); updated arrival_system, death_system (DeathResources), save/load (SaveNpcQueries), and spawn (materialize_npc) to use NpcWorkState
- **road untargetability** — added `ENTITY_FLAG_UNTARGETABLE` (bit 2) to constants and GPU compute shader; roads spawn with this flag, preventing them from being selected as combat targets; attack_system also skips `BuildingKind::Road` targets as a CPU-side guard
- **building selection uses authoritative positions** — click_to_select_system now scans buildings via EntityMap positions (deterministic placement) instead of GPU readback positions; selection overlay uses EntityMap positions for building brackets; building inspector shows overlay debug info (GPU vs EntityMap position delta)

## 2026-02-27d

- **query-first migration: eliminate iter_npcs() + Query.get() in runtime systems** — converted 10 hot-path systems from `entity_map.iter_npcs()` HashMap scan + per-entity `Query.get()` to Bevy query-first iteration with `Without<Building>, Without<Dead>` filters: on_duty_tick, starvation, ai_squad_commander, rebuild_patrol_routes, arrival, squad_cleanup (recruit pool), box_select, attack, healing, decision; each system declares a focused per-system query with only needed columns; EntityMap retained for keyed/spatial lookups (building instances, occupancy, slot→entity bridging); AttackQueries SystemParam slimmed to 2 mutable queries (CombatState, AttackTimer) with separate read-only NPC query; decision_system outer loop uses query iteration but retains DecisionNpcState `get_mut(entity)` for mutable per-entity access (clone/writeback removal is Phase 2); game_hud and death detection left unchanged due to SystemParam borrow conflicts

## 2026-02-27c

- **ECS migration slice D: economy + AI + save/load + GPU + UI → ECS, NpcInstance deleted** — replaced 40-field NpcInstance with 6-field NpcEntry (slot, entity, job, faction, town_idx, dead); moved remaining fields to ECS components: Personality, Home, PatrolRoute, WorkPosition, AssignedFarm, CarriedGold, EquippedWeapon/Helmet/Armor, LeashRange, Stealer, HasEnergy; NpcFlags.migrating replaces NpcInstance.migrating; is_military/is_stealer replaced with Job::is_military()/Job::Raider checks; added SaveNpcQueries SystemParam bundle for save/autosave; extended BuildingInspectorData with 7 ECS queries; extended MigrationResources with NpcFlags + Home queries; EntityMap is now index-only for NPCs (slot↔Entity, npc_by_town, grid, spatial)

## 2026-02-27b

- **ECS migration slice C: combat + health + energy → ECS components** — moved 10 NPC fields from NpcInstance to ECS components: Health, Energy, Speed, CombatState, CachedStats, BaseAttackType, AttackTimer, LastHitBy as `#[derive(Component)]`; healing/starving booleans moved to NpcFlags; query-first rewrites for healing_system, cooldown_system, energy_system, starvation_system, attack_system; added AttackQueries SystemParam bundle to keep attack_system under 16-param limit; updated spawn.rs to insert all new components; updated save.rs, gpu.rs, stats.rs, behavior.rs, economy.rs, all UI panels, and 10 test files; NpcInstance now holds only identity/home/equipment/patrol/flags (Slice D target)

## 2026-02-27a

- **single-ownership cutover: remove all NPC dual-writes** — NPC ECS entities now spawn with only `EntitySlot`; all NPC runtime state lives exclusively in `NpcInstance` (stored in `EntityMap.npcs`); removed all `commands.entity().insert/remove` dual-writes for NPC markers across render.rs, behavior.rs, economy.rs, health.rs, save.rs, game_hud.rs, left_panel.rs (~20 sites); deleted 12 NPC marker structs (Archer/Farmer/Miner/Crossbow/SquadUnit/Stealer/DirectControl/AtDestination/Healing/Starving/Migrating/SquadId); stripped `#[derive(Component)]` from ~20 NPC data types; rewrote `gpu_position_readback` from ECS query to EntityMap-only; migrated 5 test files from ECS queries to EntityMap reads (archer_patrol, farmer_cycle, miner_cycle, raider_cycle, vertical_slice); removed unused `Commands` params from 8 systems; added HashSet for O(1) membership in box_select_system; added debug assertions in EntityMap insert_npc/remove_npc; buildings retain full ECS components (EntitySlot, Position, Health, Faction, TownId, Building)

## 2026-02-26m

- **fill profiling blind spots in decision_system** — added sub-timers `decision/squad` (squad rest gate + sync + redirect) and `decision/work` (Working/MiningAtMine + farmer retarget + OnDuty), plus counters `n_squad`/`n_work`/`n_transit_skip`/`n_total` for per-frame NPC throughput visibility; all guarded by profiling flag

## 2026-02-26l

- **slot-indexed occupancy** — replaced `BuildingOccupancy` resource (HashMap<(i32,i32), i32> keyed by rounded position) with `BuildingInstance.occupants: i16` field on `EntityMap`; occupancy is now O(1) slot-indexed instead of hash-by-position; added `claim(slot)`/`release(slot)`/`occupant_count(slot)`/`is_occupied(slot)` methods on EntityMap; changed `AssignedFarm(Vec2)` → `AssignedFarm(usize)` and `WorkPosition(Vec2)` → `WorkPosition(usize)` to store building slots; updated `find_nearest_free` to return `(usize, Vec2)` and `resolve_spawner_npc` to return work_slot; migrated all call sites in behavior, economy, health, UI, save/load, spawn, and tests; deleted `BuildingOccupancy` struct

## 2026-02-26k

- **rename all_building_slots → all_entity_slots** — method on `EntityMap` renamed for consistency with unified entity terminology; updated call site in world.rs and stale comment in health.rs

## 2026-02-26j

- **absorb BuildingEntityMap into EntityMap** — merged `BuildingEntityMap` entirely into `EntityMap`; one resource now holds both `entities: HashMap<usize, Entity>` (all NPC + building slot→entity) and all building instance data (instances, by_kind, by_grid_cell indexes, 256px spatial grid); removed `BuildingInstance.entity` field (entity lookup via `entities.get(&slot)`); removed `slot_to_entity`/`by_entity` maps (redundant with `entities`); renamed methods: `clear()` → `clear_buildings()`, `len()` → `building_count()`, `all_slots()` → `all_building_slots()`, `set_entity()` → `entities.insert()`, `get_entity()` → `entities.get().copied()`; merged dual `Res<EntityMap>` + `Res<BuildingEntityMap>` params into single `Res<EntityMap>` in ~15 systems (combat, economy, spawn, health, save, render, tests); updated `WorldState` SystemParam field `building_slots` → `building_data`; updated `DeathResources` to single `entity_map: ResMut<EntityMap>`; `hide_building` now calls `remove_by_slot` (removes entities + instance data); ~29 files modified

## 2026-02-26i

- **unified entity slot namespace** — NPCs and buildings now share one slot allocator (`EntitySlots`, max=MAX_ENTITIES=200K) and one CPU-side GPU state (`EntityGpuState`); each entity's slot IS its GPU buffer index — no offset arithmetic (`npc_count + bld_slot`) anywhere; removed `BuildingSlots` resource, `BuildingGpuState` struct, and 6 `Bld*` GpuUpdate variants (BldSetPosition/Faction/Health/SpriteFrame/Flags/DamageFlash); renamed `SlotAllocator` → `EntitySlots`, `NpcEntityMap` → `EntityMap`, `NpcIndex` → `EntitySlot`, `NpcGpuState` → `EntityGpuState`, `NpcGpuData` → `EntityGpuData`, `camera.npc_count` → `camera.entity_count`; damage routing uses `BuildingEntityMap` lookup then `EntityMap` lookup (replaces `entity_idx >= npc_count` branch); building entity_flags (`ENTITY_FLAG_BUILDING` ± `ENTITY_FLAG_COMBAT`) set at spawn time via unified `SetFlags`; building body instances built via `BuildingEntityMap.iter_instances()` indexing into `EntityGpuState`; tower readback uses slot directly (`combat_targets[bld_slot]`); ~40 files modified across systems, rendering, UI, save/load, and tests

## 2026-02-26h

- **remove WorldCell.building, route through BuildingEntityMap** — removed `building: Option<GridBuilding>` field from `WorldCell` and the `GridBuilding` type alias; `BuildingEntityMap` is now the sole source of truth for building presence at grid coordinates via new `has_building_at(gc, gr)` and `get_at_grid(gc, gr)` methods; migrated ~20 call sites across world.rs, render.rs, ui/mod.rs, game_hud.rs, gpu.rs, ai_player.rs, left_panel.rs, save.rs, and 4 test files; `populate_tile_flags` split into terrain pass (grid iteration) + building pass (BuildingEntityMap iteration — more efficient); `empty_slots`/`is_wall_at`/`wall_autotile_variant` signatures updated to take `&BuildingEntityMap`; save format unchanged (buildings array built from BuildingEntityMap on save, terrain-only grid on load)
- **fix squad sync healing state oscillation** — wounded squad units with `prioritize_healing` could oscillate between `GoingToHeal` and `HealingAtFountain` because the healing guard only checked `GoingToHeal` and the squad redirect exemption list was missing `HealingAtFountain`; added `Activity::HealingAtFountain { .. }` to both pattern matches in behavior.rs squad sync block
- **fix pre-existing compile errors** — save.rs TraitSave kind field type mismatch (u8 vs i32, added casts); spawn.rs personality borrow-after-move (extracted trait_id_cache before move into commands.spawn)

## 2026-02-26g

- **DRY gpu constants + shared AI desire in factions UI** — gpu.rs: replaced local `MAX_NPCS`/`MAX_PROJECTILES`/`HIT_HALF_LENGTH`/`HIT_HALF_WIDTH` with shared constants from `constants.rs` (`MAX_NPC_COUNT`, `MAX_PROJECTILES`, `PROJECTILE_HIT_HALF_LENGTH`/`WIDTH`); ai_player.rs: added `debug_food_military_desire` public wrapper exposing `desire_state` for UI/debug; left_panel.rs: factions tab food/military desire bars now call the shared AI logic instead of an inline reimplementation, added `GpuReadState`/`PopulationStats` to `FactionsParams` for threat + population lookups

## 2026-02-26f

- **fix ghost waypoints: remove legacy building identity** — removed the `(kind, data_idx)` ordinal index layer from `BuildingEntityMap` (`to_slot`/`from_slot` HashMaps and 7 legacy methods); `slot` is now the sole runtime identity for buildings; the old system used `iter_kind().count()` to assign ordinal indices, which collided after mid-sequence deletions (AI pruning inner waypoints), corrupting the lookup maps and orphaning building slots (visible on GPU but absent from click detection → unclickable ghost sprites); migrated all 10 consuming files to use `get_instance(slot)` / `iter_kind_for_town()` / `remove_by_slot()`; added `hide_npc` / `hide_building` helper functions in health.rs; removed "kill linked NPC when building dies" behavior (NPCs now outlive their home building); added debug_assert for fountain uniqueness per town

## 2026-02-26e

- **zoom & LOD settings** — added user-configurable zoom speed, min/max zoom, and LOD transition point to pause menu settings; LOD threshold moved from hardcoded WGSL constant (`LOD_SIMPLE_ZOOM`) to dynamic `camera.lod_zoom` uniform field populated from `UserSettings.lod_transition` via `CameraState` extraction; zoom speed/min/max replace hardcoded `CAMERA_ZOOM_SPEED`/`CAMERA_MIN_ZOOM`/`CAMERA_MAX_ZOOM` constants in `camera_zoom_system`; settings version bumped to 8

## 2026-02-26d

- **fix building chase and demolition bugs** — combat: added `close_chase_radius` (range + 120px) to prevent archers/raiders from chasing distant enemy buildings across the map; AI perimeter: waypoint pruning now waits until the new outer ring is fully established before destroying inner waypoints (prevents premature pruning during expansion); waypoint build target uses `max(military_homes, perimeter_ring_size)` to fill ring even when military homes lag; UI building demolition (click-destroy + process_destroy_system): resolve exact building slot by kind+town+grid coords before sending lethal DamageMsg, preventing orphaned sprites from grid-only clearing

## 2026-02-26c

- **fix dead entity crash: single Dead writer** — removed `insert(Dead)` from `destroy_building()` (was second writer of `Dead`, racing with `death_system` Phase 1); `destroy_building()` is now purely grid cleanup (cell clear + wall auto-tile + combat log); all destroy paths (UI click-destroy, inspector-destroy, AI waypoint prune) now send lethal `DamageMsg(f32::MAX)` through the normal damage pipeline instead; `death_system` Phase 1 is the single writer of `Dead` (HP ≤ 0 → insert Dead); fixes crash where `death_system` Phase 2 queued `despawn()` then called `destroy_building()` which queued `insert(Dead)` on the same entity → generation mismatch on flush
- **docs: chunked tilemap, upgrade registry, GpuReadState** — updated rendering.md (chunked tilemap description), resources.md (dynamic upgrade registry, GpuReadState mixed-cadence readback, UpgradeMsg pattern), combat.md (single Dead writer flow), ai-player.md (waypoint prune DamageMsg), README.md (destroy_building description)

## 2026-02-26b

- **movement intent system** — centralized NPC movement targeting through `MovementIntents` resource (`HashMap<Entity, MovementIntent>` with `MovementPriority` arbitration); `resolve_movement_system` (after Step::Behavior) is the sole emitter of `GpuUpdate::SetTarget` and sole recorder of `NpcTargetThrashDebug`; migrated 4 systems: `decision_system` (~35 sites → `submit_intent` helper with priority mapping), `attack_system` (4 sites → Combat priority), `death_system` (2 sites → Survival priority for loot return), `click_to_select_system` (2 sites → DirectControl priority); priority ladder: Wander < JobRoute < Squad < Combat < Survival < ManualTarget < DirectControl; change detection skips writes within 1px of current GPU target; one-time init targets (spawn, boat) still write SetTarget directly; eliminates last-writer-wins race between combat chase and behavior flee

## 2026-02-26a

- **unified death_system** — collapsed 4 separate systems (`death_system`, `xp_grant_system`, `building_death_system`, `death_cleanup_system`) into one `death_system` with two phases per frame: Phase 1 marks dead (`Without<Dead>` where health <= 0, deferred), Phase 2 processes dead (`With<Dead>` from previous frame — XP grant + level-up + building destruction + loot attribution + NPC cleanup + despawn); uses `ParamSet` to resolve query conflict between mark-dead and killer/loot access; `DeathResources` SystemParam (16 fields) merges `CleanupResources` + `WorldState` unique fields + `BuildingDeathExtra`; `LastHitBy` inserted on buildings by `damage_system` (eliminates `BuildingDeathMsg`); combat chain reduced from 8 to 6 systems; deleted `BuildingDeathMsg`, `BuildingDeathExtra`, `CleanupResources`

## 2026-02-25f

- **unified damage pipeline** — consolidated `DamageMsg` (NPC-only) + `BuildingDamageMsg` into one `DamageMsg` with `entity_idx` routing (`< npc_count` → NPC, `>= npc_count` → building); `damage_system` handles both NPC and building damage with `BldSetDamageFlash` for building hit feedback; removed GPU building-skip in `npc_compute.wgsl` combat targeting — GPU spatial grid now returns building targets directly, eliminating CPU brute-force `find_nearest_enemy_building()` scan; `attack_system` handles building targets from GPU readback (job filter: only archers/crossbows/raiders); building projectiles carry real damage (no more visual-only `damage: 0.0` hack); `process_proj_hits` writes one unified `DamageMsg` for all hits; split `building_damage_system` → `building_death_system` (death-only: loot, AI deactivation, endless respawn) moved from `Step::Behavior` to `Step::Combat` chain; deleted `find_nearest_enemy_building` and `find_nearest_enemy_building_filtered` from world.rs; updated endless_mode tests to use unified `DamageMsg` with computed `entity_idx`

## 2026-02-25e

- **fix white screen: dirty message Reader/Writer conflicts** — `ai_decision_system` and `sync_patrol_perimeter_system` both had `MessageReader<T>` + `MessageWriter<T>` (via `DirtyWriters` in `WorldState`) for the same message types, causing Bevy B0002 schedule panic; added drain system pattern: `ai_dirty_drain_system` → `AiSnapshotDirty` resource (3 message types), `perimeter_dirty_drain_system` → `PerimeterSyncDirty` resource (1 message type); each drain runs `.before()` its consumer system; also migrated `NpcPipeline` from `FromWorld`/`finish()` to `RenderStartup` system (Bevy 0.18 pattern)

## 2026-02-25d

- **bevy messages: replace DirtyFlags + CombatLog contention** — decomposed `DirtyFlags` resource (9 bools, 25+ competing systems) into 8 individual Bevy Message types (`BuildingGridDirtyMsg`, `PatrolsDirtyMsg`, `PatrolPerimeterDirtyMsg`, `HealingZonesDirtyMsg`, `SquadsDirtyMsg`, `MiningDirtyMsg`, `AiSquadsDirtyMsg`, `PatrolSwapMsg`); added `DirtyWriters<'w>` SystemParam bundle with `mark_building_changed(kind)` helper and `emit_all()` for startup/reset; converted `CombatLog` from direct `ResMut` writes (18 systems contending) to `CombatLogMsg` message pattern with `drain_combat_log` collector system; added `BuildingHealState` resource for persistent healing flag; added `AiDirtyReaders<'w, 's>` SystemParam bundle for AI system; removed dead code: `FoodEvents` (zero readers), `ResetFlag` (never set), `reset_bevy_system`; 18 files modified across messages, resources, systems, UI, and save/load

## 2026-02-25c

- **fix entity-not-spawned crash on insert(Dead)** — replaced `commands.entity()` with `commands.get_entity()` (returns `Result`) at 3 sites: `death_system` Dead insertion, `damage_system` LastHitBy insertion, `destroy_building` Dead insertion; prevents crash when cross-set commands (Combat→Behavior) despawn an entity before deferred commands apply

## 2026-02-25b

- **unified entity collision + GPU tower targeting** — merged buildings into NPC GPU buffers as unified `EntityGpuBuffers` (renamed from `NpcGpuBuffers`, sized to `MAX_ENTITIES`=200K); buildings appended at offset `npc_count` in `extract_npc_data`; added `entity_flags` bitmask (`ENTITY_FLAG_COMBAT`=bit 0, `ENTITY_FLAG_BUILDING`=bit 1) replacing fragile `speed==0` heuristic; `npc_compute.wgsl` MODE 1+2 dispatch `entity_count` threads — buildings without combat early-return, towers (building+combat) skip movement but run GPU spatial grid targeting; `projectile_compute.wgsl` renamed `npc_*` → `entity_*` bindings, collision now hits both NPCs and buildings via unified grid; `process_proj_hits` routes `hit_idx >= npc_count` to `BuildingDamageMsg` via `BuildingEntityMap`; `building_tower_system` reads GPU `combat_targets` readback instead of CPU O(n) scan; deleted `fire_towers` function; `combat_range` increased from 300→400 to cover `FOUNTAIN_TOWER.range`; save/load/cleanup reset `BuildingGpuState` + `BuildingSlots`

## 2026-02-25a

- **separate NPC and building GPU buffers** — extracted `SlotPool` shared inner type with `SlotAllocator` (NPCs, max=100K) and `BuildingSlots` (buildings, max=5K) as type-safe wrappers; added `BuildingGpuState` resource (CPU-side positions/factions/healths/sprites/flash/flags with dirty tracking); added 6 `Bld*` variants to `GpuUpdate` enum routed by `populate_gpu_state`; building rendering moved from NPC storage-buffer path to instance-buffer path (`BuildingBodyInstances` + `BuildingBodyRenderBuffers` + `DrawBuildingBody`); tower targeting rewritten CPU-side (`fire_towers` scans NPC `GpuReadState` directly); building damage applied directly by `attack_system` via `BuildingDamageMsg` (projectile is visual-only); `process_proj_hits` simplified (no building collision check); `death_cleanup_system` uses `BuildingSlots` for building branch; updated 32 files across world gen, save/load, tests, UI, combat, health, economy, rendering

## 2026-02-24e

- **roadmap cleanup** — migrated ~40 checked items from stages 14-22 to completed.md; collapsed Stage 19 into completed stages line; removed linear scan elimination section (all done); cleaned up GPU extract/rendering/buildings-as-entities completed stubs; removed 3 completed spec entries from Specs table; net -87 lines in roadmap making current sprint (Stage 14: Tension) immediately scannable

## 2026-02-24d

- **kill FarmGrowthState enum** — deleted `FarmGrowthState` enum entirely; replaced `growth_state: FarmGrowthState` with `growth_ready: bool` on `BuildingInstance` (false = growing, true = ready to harvest); updated `BuildingInstance::harvest()`, `growth_system`, `farm_visual_system` (now `Local<HashMap<usize, bool>>`), all 5 behavior.rs harvest/assignment checks, save/load, game_hud inspector/tooltips, npc_render overlay, and 5 test files; pure type simplification, no logic changes

## 2026-02-24c

- **kill GrowthStates — absorb into BuildingInstance** — deleted `GrowthStates` resource, `GrowthKind` enum, and all methods (`push_farm`, `push_mine`, `find_farm_at`, `harvest`, `tombstone`) entirely; `growth_state: FarmGrowthState` and `growth_progress: f32` fields on `BuildingInstance` are now the sole source of truth; added `BuildingInstance::harvest()` method (Ready→Growing transition + yield + combat log); added `find_farm_at[_mut]`/`find_mine_at[_mut]`/`iter_growable[_mut]` methods to `BuildingEntityMap` for O(1) spatial lookups; **CRITICAL perf fix**: farmer work assignment changed from O(N) full GrowthStates scan to O(k) via `iter_kind_for_town(Farm, town_id)`, miner assignment via `iter_kind(GoldMine)`; migrated 7 behavior.rs call sites (farmer harvest, GoingToWork arrival, raider steal, mine arrival, MiningAtMine harvest, farmer work scan, miner scan); migrated growth_system, farm_visual_system, build_overlay_instances to read from BuildingInstance; `FarmReadyMarker.farm_idx` → `farm_slot` (building slot); migrated save/load (`restore_growth_from_save` sets growth fields on BuildingInstance from save data); updated game_hud inspector (4 sections), world.rs (removed push_farm/push_mine calls), economy.rs, ui/mod.rs; removed from WorldState SystemParam, lib.rs registration; updated 12 test files; fixed spawner `respawn_timer` init from -1.0 to 0.0 in `place_building_instance`

## 2026-02-24b

- **kill SpawnerState — absorb into BuildingEntityMap** — deleted `SpawnerState` resource, `SpawnerEntry` struct, `register_spawner()`, and `spawner_kind()` entirely; moved `npc_slot: i32` and `respawn_timer: f32` fields onto `BuildingInstance` (sentinel values: -2.0 = non-spawner, -1.0 = not respawning, >= 0.0 = countdown); `spawner_respawn_system` now iterates `BuildingEntityMap::iter_instances()` filtered by respawn_timer sentinel; `mining_policy_system` uses `iter_kind_for_town(MinerHome)` instead of linear spawner scan; `combat.rs` NPC slot lookup via O(1) `find_by_position()`; added `iter_instances()`/`iter_instances_mut()` methods to `BuildingEntityMap`; replaced `economy.rs:398` enabled mine check with `HashSet<(i32,i32)>` O(1) lookup; migrated save/load (SpawnerSave serialized from BuildingInstance, restored onto instances during load); updated all 6 test files to set spawner fields directly on BuildingInstance; removed SpawnerState from WorldState SystemParam, lib.rs registration, and all 16 files

## 2026-02-24a

- **BuildingEntityMap sole source of truth** — completed full migration from `WorldData.buildings` to `BuildingEntityMap` across 24 files; deleted `WorldData.buildings: BTreeMap` and all 13 legacy accessor methods (`farms()`/`beds()`/`get()`/`get_mut()` etc.), `miner_home_at()`, `gold_mine_at()`; stripped 8 fn pointer fields from `BuildingDef` struct (`len`/`pos_town`/`count_for_town`/`save_vec`/`load_vec`/`place`/`tombstone`/`find_index`) and all 14×8 implementations from `BUILDING_REGISTRY`; migrated all callers (combat, ai_player, behavior, spawn, game_hud, render, save/load, economy) to `BuildingEntityMap` methods; removed `SelectedBuilding.index` (data_idx), standardized on GPU slot everywhere; moved `building_slots` from `LoadNpcTracking` to `SaveWorldState`; re-keyed `mine_enabled` from `Vec<bool>` to `HashMap<slot, bool>`; decoupled farm/mine growth states from sequential WorldData indices; save/load serializes building instances from `BuildingEntityMap` (backward-compatible with old save format); `build_patrol_route` uses `BuildingEntityMap::iter_kind_for_town(Waypoint)` instead of WorldData; world gen creates building instances directly in `BuildingEntityMap`; net ~350 lines deleted in final cleanup pass

## 2026-02-23e

- **separate building entity map** — replaced `BuildingSlotMap` with `BuildingEntityMap` that owns all building identity: `(kind, idx) ↔ slot ↔ Entity`; buildings removed from `NpcEntityMap` entirely — `NpcEntityMap` is now NPC-only; removed `building_query.contains()` guards from `attack_system` and `process_proj_hits` (buildings naturally rejected by `NpcEntityMap` lookup); `building_damage_system` uses `BuildingEntityMap.get_entity_by_building()` directly; `BuildingInspectorData` simplified from 2 resources to 1; abandoned phases 3-7 of buildings-as-entities spec (further merging was counterproductive)

## 2026-02-23d

- **buildings as ECS entities (phase 1+2)** — buildings now spawn as ECS entities with `Building` marker component, reusing the NPC lifecycle (`NpcEntityMap`, `death_system`, `death_cleanup_system`); deleted `BuildingHpState` entirely (~95 lines) — entity `Health` is the single source of truth for building HP; `building_damage_system` writes entity Health directly (via `BuildingDeathExtra` SystemParam to stay within Bevy's 16-param limit); `sync_building_hp_render` queries building entities; save/load uses `HashMap<String, Vec<f32>>` (identical JSON format); removed `hps`/`hps_mut` fn pointer fields from all 13 `BUILDING_REGISTRY` entries; fixed Bevy query conflict (NPC health query vs building health mutation) that silently broke the entire EguiPrimaryContextPass schedule

## 2026-02-23c

- **wall T-junctions + cross sprites** — added cross (4-way) and T-junction (3-way) wall sprites from `wood_walls_131x32.png`; 4 T-junction rotations for all orientations; nearest-neighbor atlas sampling + half-pixel UV inset to eliminate rendering artifacts at layer boundaries

## 2026-02-23b

- **wall auto-tile corner fix** — fixed all four corner atlas offsets (were diagonally opposite); replaced magic numbers with named constants (`WALL_TR`/`WALL_TL`/`WALL_BL`/`WALL_BR`) that trace atlas generation order → rotation → Y-flip to screen appearance

## 2026-02-23a

- **castle fortification system** — new Wall building type (BuildingKind::Wall) placed on town grid, blocks enemy faction NPCs via GPU tile_flags (bit 6 + faction bits 8-11); raiders target and breach walls via existing building attack fallback; 3-tier upgrade system (Wooden Palisade 80HP → Stone Wall 200HP → Fortified Wall 400HP) with per-wall click-to-upgrade in building inspector; walls compete for build slots with economy/military buildings creating strategic trade-offs

## 2026-02-22p

- **squad wounded→fountain fix** — low-HP squad members no longer oscillate between fleeing and re-engaging; squad sync now checks `prioritize_healing` policy before redirecting to squad target, sending wounded NPCs to fountain instead; added `GoingToHeal` to squad sync no-redirect list; fixed all HP percentage checks to use `CachedStats.max_health` instead of hardcoded 100.0 (4 occurrences)

## 2026-02-22o

- **AI miner food hoarding** — when miner homes are below personality minimum and food < 4 (miner home cost), AI now skips all other building to let food accumulate; without this, cheaper buildings (farms=2, houses=2) drained food before it reached 4, filling all slots with zero miner homes

## 2026-02-22n

- **Deterministic miner bootstrap** — AI now builds miner homes deterministically before the food reserve gate, bypassing both weighted random scoring and food reserve restrictions; guarantees min_miner_homes (Aggressive:1, Balanced:2, Economic:3) are built as the very first actions; fixes gold economy deadlock where growing spawner count pushed reserve above food, permanently blocking all scored building including miners

## 2026-02-22m

- **AI mining bootstrap** — initial mining radius now reaches nearest gold mine (rounded up to 300px step grid) instead of fixed default; miner target has floor of 1 when mines exist in radius; miner homes get 5× score boost until personality's min_miner_homes reached (Aggressive:1, Balanced:2, Economic:3); inlined `remove_building` into `destroy_building` (was unused public helper)

## 2026-02-22l

- **AI player per-frame overhead reduction** — `ai_squad_commander_system` now uses dirty+heartbeat gating (wakes on `dirty.ai_squads` or 2s fallback) instead of running every frame; spawner counts cached in `AiTownSnapshotCache` (recomputed only when buildings change); inlined building counts (eliminated per-tick HashMap allocation); cached waypoint ring slots in `AiTownSnapshot` (was computed 4× per tick with allocation + sort)

## 2026-02-22k

- **AI gold hoarding for expansion** — when all build slots are full, AI now reserves gold for the next expansion upgrade; non-expansion gold-costing upgrades are skipped unless surplus gold exists beyond what expansion costs; prevents AI from wasting gold on stat upgrades while unable to expand

## 2026-02-22j

- **aggressive AI attack corridors** — fixed road placement bug where `try_build_road_grid` didn't filter occupied cells (only filtered existing roads), causing all top-ranked candidates to fail `place_building`; aggressive roads now extend to 2× build radius on cardinal axes as offensive attack routes, corridor cells outside town bounds skip economy-adjacency requirement, batch size increased from 2 to 4

## 2026-02-22i

- **starvation prevention** — work score now scales down linearly when energy is below tired threshold (30), so rest naturally outcompetes work around energy ~24; previously NPCs at energy 29 would choose work (score 40) over rest (score 21), enter farm-retarget loops that burned energy to 0, and starve despite having 90K+ food available

## 2026-02-22h

- **AI military snowball fix** — balanced builder no longer spirals to 11:1 military-to-civilian ratio; added symmetric population ratio correction (over-military dampens military_desire and boosts food_desire), capped waypoint_gap at 0.5 to break self-reinforcing feedback loop, gave farmer homes a 0.5 baseline score when at target to match military's existing maintenance trickle
- **multi-drop loot tables** — NPC loot_drop changed from single LootDrop to slice; military NPCs (archers, crossbows, fighters, raiders) now drop food or gold (deterministic pick via xp%len); miners drop gold only, farmers drop food only
- **miner-cycle test scene** — new 5-phase integration test: Mining → MiningAtMine → harvest gold → deliver → rest/wake; validates full mining pipeline end-to-end

## 2026-02-22g

- **unified building placement** — collapsed `place_building`, `build_and_pay`, and `place_wilderness_building` into a single `place_building(world_pos)` that handles everything: validate cell (exists, empty, not water), reject foreign territory, deduct food, place on grid, register in WorldData, set waypoint patrol_order, push farm growth state, register spawner, push HP, allocate GPU slot, mark dirty flags; all callers (player UI + AI) now use one code path
- **military target civilian homes fix** — AI military desire now counts miner homes in civilian homes total for archer home target calculation (previously only counted farmer homes)

## 2026-02-22f

- **farmer farm-rushing fix** — farmers no longer dogpile the same farm; all occupied farms are now skipped during farm selection (previously Ready farms bypassed occupancy check), working farmers no longer abandon their current farm when any Ready crop exists elsewhere, and en-route farmers retarget to the nearest free farm if their target gets claimed by another farmer who arrived first
- **waypoint ring placement** — AI waypoints now use a personality-driven outer ring pattern (block corners on build area perimeter adjacent to road intersections) instead of territory perimeter + spacing heuristic; when the town expands, inner waypoints are pruned to maintain one ring; removed uncovered-mine wilderness waypoint logic and territory macro system
- **road grid snapping** — AI road positions are now grid-snapped via `world_to_grid`→`grid_to_world` roundtrip for pixel-perfect alignment
- **test scene inspector** — bottom panel and selection overlay now available in test scenes (AppState::Running)

## 2026-02-22e

- **road scoring reflects candidate availability** — road scoring now pre-checks actual available road-pattern slots via `count_road_candidates()` before entering the score pool; `road_need` is capped at the real candidate count, so roads score 0 when no candidates exist instead of inflating to 296+ and failing every tick

## 2026-02-22d

- **AI decision retry loop** — when a building action fails (e.g., no road candidates), the AI now removes that action variant and re-picks from remaining candidates instead of wasting the tick; previously Roads (score 48) could fail silently for hours while Farm (11) and MinerHome (9) never got a chance
- **road adjacency expanded** — road candidate adjacency check increased from Chebyshev distance 1 to 2, covering the full gap in Economic's 4×4 road pattern; previously buildings in interior rows/cols (distance 2+ from road lanes) created zero road candidates, causing road placement to fail even when empty road slots existed
- **AI decision debug logging** — new `debug_ai_decisions` setting (Settings → Debug → "AI Decision Logging"); when enabled, failed actions appear in the faction inspector's Recent Actions as `[dbg] Roads FAILED (Roads=48.0 Farm=11.4 ...)` with top scores
- **escape menu in test scenes** — pause menu (ESC) and settings now available during `AppState::Running` test scenes, not just during normal gameplay

## 2026-02-22c

- **AI road building** — AI towns now build roads in personality-specific grid patterns: Economic uses 4×4 grid, Balanced uses 3×3 grid, Aggressive uses cardinal axes from center; roads are batch-placed adjacent to economy buildings (farms, farmer homes, miner homes); non-road buildings filter out road pattern slots via `is_road_slot` to reserve space for future roads
- **Phase 1/Phase 2 AI decision split** — AI decision system now runs two independent phases per tick: Phase 1 scores and executes one building action, Phase 2 re-checks food/gold and scores one upgrade; previously buildings and upgrades competed in a single weighted pool, causing upgrades to crowd out building when scores were high
- **economy desire signal** — new `economy_desire = 1 - slot_fullness` floors food/military/gold desires so building scores never collapse to zero while the town has empty slots
- **expansion delay fix** — expansion upgrades are now delayed whenever the town has empty slots and can afford any building; previously the guard only checked specific home targets against personality quotas, allowing expansion to slip through when targets were met but slots were still unfilled (farms, waypoints, roads)
- **migration nearest-edge approach** — migration boats now spawn at the map edge closest to the settle target instead of a random edge; reduces travel time and prevents boats from crossing the entire map
- **factions copy debug** — Factions tab (I key) now has a "Copy Debug" button that builds a comprehensive debug string (resources, desires with formulas, buildings, NPCs, population, mining, upgrades, squads, recent actions, stat multipliers) and copies it to clipboard via arboard
- **AI action history** — increased `MAX_ACTION_HISTORY` from 3 to 20 for better debug visibility
- **ai-building test** — new test scene: pick an AI personality, observe it building a town with 100K food+gold and 1s decision interval; includes test time controls (Space=pause, +/-=speed) and left panel (I key for Factions tab)

## 2026-02-22b

- **DC wake on move** — DirectControl NPCs that are resting (`GoingToRest`/`Resting`) now wake to `Idle` when given a right-click move or attack command; previously they slid to the destination while still in resting state, recovering energy incorrectly
- **roads raider-buildable + indestructible** — roads are now buildable by raider AI and cannot be destroyed by projectile damage (same as gold mines)
- **foreign territory build rejection** — `place_wilderness_building` now rejects placement inside another faction's build area via `in_foreign_build_area()` check; prevents AI from placing waypoints/roads inside enemy towns

## 2026-02-22a

- **box-select inspector fix** — box-selecting NPCs now clears `SelectedNpc` and `SelectedBuilding` so the DC group inspector shows immediately; previously stale individual selections masked the group view; also added missing `dc_count` check in the `!show_npc` branch of `inspector_content`
- **DC keep-fighting toggle** — new "Keep fighting after loot" checkbox in DC group inspector; when enabled, DirectControl NPCs accumulate loot from kills without disengaging combat or walking home; loot piles up in `Activity::Returning { loot }` while NPC stays put; on DC removal, behavior system redirects to home for delivery; `Returning` NPCs arriving at non-home positions (after DC removal) now redirect home instead of discarding loot

## 2026-02-21o

- **fix farm growth index mismatch** — `GrowthStates` stores farms and mines in one flat vec, but WorldData indexes them separately; when farms are built after mines exist, WorldData farm index ≠ GrowthStates index, causing UI to show 0% growth (reading mine data), harvest to read wrong entry, and destroy to tombstone wrong entry; added `GrowthStates::find_farm_at(pos)` position-based lookup and replaced all 6 WorldData-index-based GrowthStates lookups (UI display, UI tooltip, arrival harvest, working farmer harvest, raider harvest, destroy tombstone) with position-based lookups
- **farmer post-delivery idle** — farmers now go Idle after delivering food (not GoingToWork to old farm); decision system re-evaluates best target, allowing farmers to redirect to ready farms instead of returning to a growing farm
- **working farmer ready-check** — working farmers scan for Ready same-faction farms; if found, release current farm and go Idle so decision system redirects to the ready crop

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
