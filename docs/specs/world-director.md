# World Director -- Random Events

An invisible AI that periodically triggers random events -- natural disasters, plagues, blessings, raids. All tuning is player-configurable with Easy/Normal/Hard presets.

## Goal

Create emergent narrative through periodic random events targeting all towns fairly. Events should feel impactful, create tension, and reward preparation. Player controls intensity via settings.

## Event Registry (k8s pattern)

- **CRD**: `EventDef` struct in `constants/events.rs`
- **etcd**: `EVENT_REGISTRY: &[EventDef]` + `event_def(kind)` lookup
- **CR**: `ActiveEvent` ECS component on spawned event entities
- **Controller**: `world_director_system` -- periodic roll, spawn events, apply effects

```rust
pub enum EventKind { Fire, Earthquake, Blight, Bounty, RaidSurge }
pub enum EventCategory { Natural, Plague, Blessing, Political }

pub struct EventDef {
    pub kind: EventKind,
    pub label: &'static str,
    pub category: EventCategory,
    pub weight: f32,              // frequency weight in random pool
    pub min_severity: f32,        // 0.0-1.0
    pub max_severity: f32,
    pub radius: f32,              // affected area (world units)
    pub duration_hours: f32,      // how long the event lasts
    pub warning_hours: f32,       // 0 = surprise, >0 = advance warning
    pub effects: &'static [EventEffect],
}
```

Adding a new event type = 1 `EventKind` variant + 1 `EVENT_REGISTRY` entry + effect handlers.

## MVP Events (5)

| Event | Category | Weight | Radius | Duration | Warning | Effect |
|-------|----------|--------|--------|----------|---------|--------|
| Fire | Natural | 1.0 | 200 | 2hr | 0 (surprise) | Damage buildings in radius, spread to adjacent |
| Earthquake | Natural | 0.5 | 400 | instant | 1hr | Damage all buildings in radius by severity% |
| Blight | Plague | 0.8 | 300 | instant | 0.5hr | Reset ProductionState on all farms in radius |
| Bounty | Blessing | 0.6 | N/A | instant | 0 | Add food+gold to target town stores |
| Raid surge | Political | 0.7 | N/A | instant | 2hr | Spawn extra hostile NPCs targeting a town |

## EventEffect enum

```rust
pub enum EventEffect {
    DamageBuildings { pct: f32 },      // damage buildings by severity * pct of max_hp
    DamageNpcs { pct: f32 },           // damage NPCs in radius
    KillFarms,                          // reset ProductionState.progress on farms in radius
    SpawnHostiles { count: i32 },       // spawn enemy NPCs at map edge
    BonusResource { kind: ResourceKind, amount: i32 },
}
```

## ActiveEvent Component

```rust
#[derive(Component)]
pub struct ActiveEvent {
    pub kind: EventKind,
    pub target_town: i32,           // town_idx targeted
    pub position: Vec2,             // center of effect
    pub severity: f32,              // 0.0-1.0, scaled by settings
    pub remaining_hours: f32,       // 0 = instant, >0 = ongoing
    pub warning_remaining: f32,     // >0 = still in warning phase
    pub effects_applied: bool,      // true after effects fire
}
```

## World Director System

`world_director_system` runs in FixedUpdate, cadenced by game hours:

1. Skip if paused, or game_day < 5 (grace period), or events_enabled == false
2. Each game-hour tick: roll `rng.random::<f32>() < event_frequency`
3. If triggered:
   - Pick random town from all alive towns (player + AI + raider -- fair targeting)
   - Weighted random pick from EVENT_REGISTRY, filtered by positive_ratio setting
   - Roll severity: `rng.random_range(def.min_severity..def.max_severity) * severity_scale`
   - Spawn `ActiveEvent` entity with warning timer (if warning_enabled)
4. Each tick for active events:
   - If `warning_remaining > 0`: decrement, show warning banner
   - If `warning_remaining <= 0 && !effects_applied`: apply effects, mark applied
   - If `remaining_hours <= 0`: despawn event entity

### Effect application

- **DamageBuildings**: iterate `entity_map.iter_instances()` in radius, emit `DamageMsg` for each building
- **KillFarms**: iterate farms in radius via `entity_map`, set `ProductionState.progress = 0.0, ready = false`
- **SpawnHostiles**: call `materialize_npc()` for N hostiles at map edge nearest to target town
- **BonusResource**: `FoodStore.0 += amount`, `GoldStore.0 += amount` on target town entity
- **Fire spread**: for ongoing Fire events, each hour check adjacent buildings and add to damage set

### Mitigation hooks

Each effect application should check for a `mitigation_mult` before applying:
```rust
let final_severity = severity * (1.0 - mitigation_mult);
```
For MVP, `mitigation_mult` is always 0.0. Future upgrade slices will populate it from town upgrades.

## Settings

```rust
pub struct WorldDirectorSettings {
    pub events_enabled: bool,
    pub event_frequency: f32,      // chance per game-hour (0.0-1.0)
    pub severity_scale: f32,       // multiplier on event severity
    pub positive_ratio: f32,       // chance that an event is a blessing (0.0-1.0)
    pub warning_enabled: bool,     // show advance warning for applicable events
    pub scaling_with_size: bool,   // events scale with town wealth/size
}
```

| Setting | Easy | Normal | Hard |
|---------|------|--------|------|
| event_frequency | 0.05 | 0.15 | 0.30 |
| severity_scale | 0.5 | 1.0 | 2.0 |
| positive_ratio | 0.5 | 0.3 | 0.1 |
| warning_enabled | true | true | false |
| scaling_with_size | false | true | true |

Settings persisted in save file via `serde(default)` for backward compat.

## UI

### Settings panel
- "World Events" section in settings/new game menu
- Preset buttons: Easy / Normal / Hard / Custom
- When Custom: individual sliders for each setting
- Toggle: "Enable World Events" master switch

### Event notifications
- Warning phase: amber banner at top "Earthquake detected -- 1 hour warning"
- Active phase: red banner "Earthquake striking [Town Name]!"
- Blessing: green banner "Bountiful harvest at [Town Name]!"
- Events log to combat log with outcomes

## Performance

- System is cadenced (per game-hour, not per-frame) -- negligible CPU cost
- Effect application: O(buildings_in_radius) via EntityMap spatial query, bounded by event radius
- Max 1 active disaster at a time (blessings can stack)
- No per-frame NPC iteration added
- No GPU readback used -- fully CPU-authoritative

## Authority

All World Director data is CPU-only and CPU-authoritative:
- Reads EntityMap for town/building positions
- Emits DamageMsg (CPU-authoritative Health)
- Modifies ProductionState, FoodStore, GoldStore (CPU-only)
- Spawns NPCs via materialize_npc (CPU-authoritative)

## Edge cases

- Events don't fire during pause
- Grace period: no events in first 5 game-days
- Max 1 active disaster at a time (prevents stacking catastrophes)
- Blessings can overlap with disasters
- Save/load preserves active events + settings
- Softlock protection: events that would destroy the last farm or last spawner are severity-capped
- Towns with 0 buildings are skipped as targets

## Out of scope (future slices)

- Tornado, Flood, Drought, Meteor, Monster horde
- Plague/sickness (NPC health debuff)
- Refugees, Trade caravans, Desertion
- Counterplay upgrades (Resilience tree)
- Visual effects (fire particles, screen shake) -- placeholder colors only in MVP
