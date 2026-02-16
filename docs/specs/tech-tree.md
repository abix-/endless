# Tech Tree Upgrade Graph (v1)

Stage 20. Implementation spec for remaining chunks (3-4). Chunks 1-2 are complete.

## Completed

Chunks 1-2 implemented: prereqs, multi-resource costs, 16 per-NPC-type nodes, registry-driven UI. See [completed.md](../completed.md) "Tech Tree (Chunks 1-2)" section.

## Remaining: Chunk 3 — Energy Nodes

- [ ] Add `UpgradeType` variants: `MilitaryStamina`, `FarmerStamina`, `MinerStamina` (bump `UPGRADE_COUNT`)
- [ ] Wire into `energy_system`: per-town per-job drain modifier based on stamina upgrade level
- [ ] Prereqs: MilitaryStamina after MoveSpeed, FarmerStamina after FarmerMoveSpeed, MinerStamina after MinerMoveSpeed
- [ ] AI weights for new nodes

System wiring:
- `energy_system`: apply per-town energy modifiers by exact job type; per-type stamina nodes reduce drain for their target job only; keep clamp behavior (`0..100`) and starvation interaction unchanged
- Dodge runtime wiring: map dodge node levels to per-job dodge strength and dodge cooldown multipliers; apply in projectile avoidance/dodge path (GPU or CPU path), with explicit cooldown reduction by level

## Remaining: Chunk 4 — Player AI Manager

- [ ] Tech-tree unlock node for `Player AI Manager`
- [ ] `PlayerAiManager` resource: `unlocked`, `enabled`, `build_enabled`, `upgrade_enabled`
- [ ] Reuse `AiKind::Builder` decision logic for faction 0 town, gated by unlock + toggle
- [ ] UI: hidden until unlocked, then show enable toggle + build/upgrade toggles + status label

Player AI manager model:
- Add `PlayerAiManager` resource with: `unlocked: bool` (derived from tech tree node), `enabled: bool`, `build_enabled: bool`, `upgrade_enabled: bool`, `aggression/defense/economy bias` controls (or simple profile enum)
- Keep enemy AI resources unchanged; player manager should call shared decision helpers so behavior parity is maintained
- Safety guard: player manager only manages player town (`faction == 0`) and never controls enemy settlements

UI (`ui/left_panel.rs`):
- Player AI manager controls: hidden/disabled until `Player AI Manager` node is unlocked
- Once unlocked: show enable toggle + core settings (build/upgrade toggles, interval/profile)
- Show status label (Disabled / Active) for quick feedback

## Files to change

- `rust/src/systems/stats.rs`
- `rust/src/systems/energy.rs`
- `rust/src/systems/ai_player.rs`
- `rust/src/ui/left_panel.rs`
- `rust/src/ui/game_hud.rs` (if label wiring is needed)
- `rust/src/resources.rs` (`AutoUpgrade` sizing stays tied to `UPGRADE_COUNT`)
- `rust/src/settings.rs` (persist player AI manager UI settings/toggles if desired)

## Validation

1. `cargo check` passes
2. Can only buy a node when all prerequisites are satisfied
3. Auto-upgrade never buys locked nodes
4. AI never queues locked nodes
5. Per-type stamina upgrades affect only their target NPC type
6. `Player AI Manager` is unavailable before unlock, then becomes configurable after unlock
7. With manager enabled, player town performs the same automation class as enemy builder AI

## Test additions

- `tests/tech_tree.rs`: prerequisites block purchase until required levels are reached; queue entries for locked nodes are ignored
- `tests/energy_upgrades.rs`: guard drain with/without Guard Stamina differs as expected; farmer/miner drain with/without Worker Stamina differs
- `tests/ai_upgrades.rs`: AI upgrade choice skips locked nodes and buys unlocked affordable nodes
- `tests/player_ai_manager.rs`: unlock gating works; enabling manager triggers player-town AI actions after unlock; manager never issues actions for non-player towns
