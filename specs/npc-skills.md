# NPC Skills

Per-NPC skill proficiency system. NPCs accumulate skill values (0.0-100.0) by performing work. Higher skill yields gameplay bonuses via `proficiency_mult()`.

## Goal

Provide the data foundation for NPC specialization. Skills grow over time (slices 2-5) and feed into damage, cooldown, and dodge calculations.

## Behavior

- Every NPC spawns with `NpcSkills::default()` (all fields 0.0).
- Skill values are clamped to `[0.0, MAX_PROFICIENCY]` (0.0 to 100.0).
- `proficiency_mult(value)` converts a skill value to a damage/cooldown multiplier:
  - 0 -> 1.0x (no bonus)
  - 50 -> 1.25x
  - 100 -> 1.5x
- Dodge chance scales with `dodge` skill up to `DODGE_PROF_MAX_CHANCE` (25%) at 100.

## Data Model

### NpcSkills component (`components.rs`)

```rust
#[derive(Component, Clone, Default, Reflect, serde::Serialize, serde::Deserialize)]
#[reflect(Component)]
pub struct NpcSkills {
    pub farming: f32,   // 0.0-100.0
    pub combat: f32,    // 0.0-100.0
    pub dodge: f32,     // 0.0-100.0
}
```

### Constants (`constants/mod.rs`)

```rust
pub const FARMING_SKILL_RATE: f32 = 0.02;   // per game hour tending
pub const COMBAT_SKILL_RATE: f32 = 1.0;     // per kill
pub const DODGE_SKILL_RATE: f32 = 0.5;      // per dodge event
pub const MAX_PROFICIENCY: f32 = 100.0;
pub const DODGE_PROF_MAX_CHANCE: f32 = 0.25; // 25% max dodge at prof 100
```

### Helper (`systems/stats/mod.rs`)

```rust
pub fn proficiency_mult(value: f32) -> f32 {
    1.0 + (value.clamp(0.0, 100.0) / 100.0) * 0.5
}
```

## k8s Compliance

- `NpcSkills` is a CR (instance) component -- it holds accumulated state per entity, not config.
- Skill rates and caps belong in the Def layer (registry). Currently stored as global constants; per-job rates are a future enhancement.
- Growth systems (future slices) are Controllers: read Def rates, write back to `NpcSkills`.

## Edge Cases

- Old saves without a `skills` field deserialize correctly via `#[serde(default)]`.
- Skill values above 100.0 are clamped at the `proficiency_mult` call site; the component itself does not enforce the cap to avoid double-clamping.

## Integration

- Spawn: `NpcSkills::default()` is added to the NPC entity bundle in `materialize_npc()`.
- Save: `NpcSaveData` includes `#[serde(default)] pub skills: NpcSkills`. Serialized from ECS on save, restored via `NpcSpawnOverrides` on load.
- Type registry: `NpcSkills` is registered via `.register_type::<NpcSkills>()` in `lib.rs`.
- Building skills (`BuildingSkills`) follow the same k8s pattern but are a separate component and separate issue.

## Acceptance Criteria

- [x] `NpcSkills` component with `farming`, `combat`, `dodge` fields (f32, 0.0-100.0)
- [x] All NPCs spawn with `NpcSkills::default()` (all 0.0)
- [x] Save/load round-trip preserves skill values
- [x] Old saves load correctly (default to 0.0)
- [x] `proficiency_mult()` helper tested (0 -> 1.0, 50 -> 1.25, 100 -> 1.5)
- [x] `cargo clippy --release -- -D warnings` passes
