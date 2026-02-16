# NPC Skills & Proficiency

Stage 16b. Implementation spec for per-NPC skill progression.

## Goal

Add persistent, per-NPC proficiency that improves with experience and directly impacts how well NPCs perform their work (farm, fight, dodge, etc.).

## Design constraints

- Job determines what an NPC can do. Skills determine how well they do it.
- Proficiency is additive/scalar, not a replacement for existing job stats/upgrades/traits.
- Keep deterministic enough for tests and profiling; avoid expensive per-frame randomness.

## Data model

- Add `NpcSkills` component (or resource-backed cache keyed by `NpcIndex`):
  `farm: u16`, `combat: u16`, `dodge: u16`, optional future fields (`craft`, `leadership`, etc.)
- Range: store as integer points 0..1000 internally; expose as 0.0..100.0 in UI
- Add helper functions: `skill_to_pct(points) -> f32`, `skill_multiplier(points, max_bonus) -> f32`, `add_skill_xp(points, delta_xp)`
- Persist across respawn only if desired by design; default v1 behavior: skill belongs to NPC instance (newly spawned replacement starts at baseline)

## v1 math (safe, bounded)

- Farming: `farm_mult = 1.0 + farm_prof * 0.005` (max +50%) — apply to tended farm growth/harvest throughput
- Combat: `combat_mult = 1.0 + combat_prof * 0.004` (max +40%) — apply to effective damage and/or cooldown efficiency
- Dodge: `dodge_mult = 1.0 + dodge_prof * 0.006` (max +60%) — apply to dodge decision weight / projectile avoidance strength

## XP gain model

- Farming XP: gain when farm work ticks and on successful harvest
- Combat XP: gain on attack attempts and bonus on confirmed hit/kill
- Dodge XP: gain when near-miss/projectile avoidance logic triggers
- Diminishing returns: scale XP gain by `(1.0 - proficiency_pct)` so early growth is faster than late growth

## System integration points

- `systems/economy.rs`: farm growth/harvest uses farming proficiency multiplier
- `systems/stats.rs` / combat pipeline: incorporate combat proficiency in resolved/effective combat output path
- GPU dodge path (`gpu/npc_compute.wgsl` + sync path): pass dodge proficiency signal into avoidance weighting (or CPU-side precomputed dodge factor buffer)
- `systems/spawn.rs`: initialize `NpcSkills` baseline by job
- `systems/health.rs` and `systems/behavior.rs`: optional hooks for dodge/combat XP triggers

## UI/UX

- Inspector (`ui/game_hud.rs`): show Skill panel: Farm / Combat / Dodge proficiency bars + numeric values
- Roster (`ui/left_panel.rs`): optional columns/sort for top relevant proficiency by job
- Tooltip copy: explain that proficiency increases with activity and improves effectiveness

## Balancing guidance

- Keep proficiency impact weaker than major tech-tree upgrades at low-mid values
- Cap total stacked multipliers (traits + upgrades + proficiency) to avoid runaway scaling
- Profile impact under high NPC counts; prefer cached multipliers updated on skill change, not recomputed everywhere each frame

## Testing

- `tests/skills_progression.rs`: verifies farming/combat/dodge proficiency increases under expected actions
- `tests/skills_effects.rs`: higher farm proficiency yields faster output than baseline; higher combat proficiency yields better duel outcome; higher dodge proficiency reduces projectile hits
- Regression: ensure no-skill baseline behavior remains close to current gameplay
