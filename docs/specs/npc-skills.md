# NPC Skills & Proficiency

Per-NPC skill progression with unclamped unclamped scaling. Level 9999 = godlike.

## Goal

Add persistent, per-NPC proficiency that improves with experience and directly impacts how well NPCs perform their work (farm, fight, dodge, etc.). High proficiency should feel massively rewarding -- a level 9999 NPC is a god compared to a fresh spawn.

## Design constraints

- Job determines what an NPC can do. Skills determine how well they do it.
- Proficiency is additive/scalar, stacks with existing job stats/upgrades/traits.
- No artificial caps on the multiplier -- let the numbers grow. Only the proficiency VALUE caps at MAX_PROFICIENCY (9999).
- Keep deterministic enough for tests and profiling; avoid expensive per-frame randomness.

## Data model

- `NpcSkills` component: `farming: f32`, `combat: f32`, `dodge: f32`
- Range: 0.0 to MAX_PROFICIENCY (9999.0)
- Stored as f32, displayed as integer in UI
- Skill belongs to NPC instance -- newly spawned replacement starts at 0
- Persisted via save/load with serde(default) backward compat

## Scaling formula (unclamped)

One formula for all skills:

```rust
pub fn proficiency_mult(value: f32) -> f32 {
    1.0 + value * 0.01
}
```

| Prof  | Multiplier | Feel           |
|-------|-----------|----------------|
| 0     | 1.0x      | Fresh spawn    |
| 100   | 2.0x      | Experienced    |
| 500   | 6.0x      | Veteran        |
| 1000  | 11.0x     | Elite          |
| 5000  | 51.0x     | Legendary      |
| 9999  | 101.0x    | Godlike        |

No clamp on the multiplier. MAX_PROFICIENCY (9999) only caps the proficiency value, not the effect.

### Application per skill

- **Combat**: `damage *= proficiency_mult(combat)`, `cooldown /= proficiency_mult(combat)`
- **Farming**: `growth_rate *= proficiency_mult(farming)` for tended farms
- **Dodge**: miss chance = `1.0 - 1.0 / proficiency_mult(dodge)`. At 0: 0%, 100: 50%, 1000: 91%, 9999: 99%.

All three skills use the same proficiency_mult function. Dodge converts the multiplier to a probability via `1 - 1/mult`, which naturally approaches but never reaches 100%.

## Skill gain rates

- `COMBAT_SKILL_RATE = 1.0` per kill
- `FARMING_SKILL_RATE = 0.02` per game hour tending
- `DODGE_SKILL_RATE = 0.5` per dodge event
- All capped at `MAX_PROFICIENCY = 9999.0`
- No diminishing returns on gain rate -- linear accumulation, unclamped

## Constants

```rust
pub const FARMING_SKILL_RATE: f32 = 0.02;
pub const COMBAT_SKILL_RATE: f32 = 1.0;
pub const DODGE_SKILL_RATE: f32 = 0.5;
pub const MAX_PROFICIENCY: f32 = 9999.0;
```

## System integration

- `systems/stats.rs`: resolve_combat_stats takes prof_combat, applies proficiency_mult to damage and inverse to cooldown
- `systems/economy.rs`: growth_system applies proficiency_mult to tended farm growth rate
- `systems/combat.rs`: process_proj_hits applies dodge miss chance (capped)
- `systems/health.rs`: death processing grants combat skill on kill
- `systems/spawn.rs`: NpcSkills::default() on spawn, overrides for save restore

## UI

- Inspector Skills tab: progress bars 0-9999, color-coded (gray <2500, white 2500-7499, green >=7500)
- Roster Prof column: top skill value, sortable
- Effect descriptions show current multiplier

## Testing

- proficiency_mult(0) == 1.0
- proficiency_mult(100) == 2.0
- proficiency_mult(9999) ~= 100.99
- No clamp test (values above 9999 would give higher mult, but skill gain caps at 9999)
- Dodge chance at prof 100 = 50%, prof 1000 = 91%, prof 9999 = 99% (via 1 - 1/mult)
