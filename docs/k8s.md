# K8s Architecture (Def → Instance → Controller)

Every entity type follows a three-layer pattern borrowed from Kubernetes Custom Resource Definitions.

## K8s ↔ Endless

In Kubernetes, you extend the API by defining a **CRD** (schema), storing **CR** instances in **etcd**, and running a **Controller** that watches for changes and reconciles desired state with actual state. Endless applies the same separation to game entities.

| K8s Concept | What it is in K8s | Endless Equivalent | NPC Example |
|---|---|---|---|
| **CRD** | Schema definition — declares a new Kind with typed fields (`spec`, `status`) | `NpcDef` struct — declares the shape of an NPC type | `struct NpcDef { base_hp, base_damage, sprite, ... }` |
| **kind** | Discriminator that selects which resource type applies (`kind: Pod`, `kind: Deployment`) | Enum variant that selects which registry entry to use | `Job::Archer`, `BuildingKind::Farm`, `TownKind::AiRaider` |
| **API Server + etcd** | Stores CRD schemas and CR instances; serves them via REST | `NPC_REGISTRY` static array + `npc_def(job)` lookup | Compile-time store, one entry per Job variant |
| **CR** (Custom Resource) | One instance of a CRD — a YAML object with `metadata`, `spec`, `status` | ECS entity + components — one spawned NPC | Archer entity: NpcStats + CachedStats + Health + ... |
| **spec** (desired state) | What the user declared they want (`replicas: 3`, `image: nginx`) | Def base values + upgrade levels + equipment bonuses | `NpcDef.base_hp * upgrade_mult * equip_bonus` |
| **status** (observed state) | What the controller observed is actually true right now | ECS component values after reconcile | `CachedStats.max_health = 150.0` |
| **Controller** | Watch loop: observe CRs, compare spec vs status, take action to converge | Systems that read Def + inputs → write/update components | `resolve_combat_stats()`, `process_upgrades_system` |
| **`kubectl apply`** | Create or update a CR — triggers controller reconcile | `materialize_npc()` — creates entity from Def + overrides | Spawn entity, resolve stats, init GPU buffer |
| **Reconcile** | Controller detects drift between spec and status, re-derives | System re-resolves stats when inputs change | Upgrade purchased → re-run `resolve_combat_stats()` |

**Key distinction:** CRD = schema (`NpcDef` struct), etcd = storage (`NPC_REGISTRY` array), CR = instance (ECS entity). The struct and registry live in the same file but serve different roles — one defines the shape, the other stores the entries.

**Naming inconsistency:** The `kind` discriminator exists on all Defs but uses different field names — Buildings, Towns, and Items use `kind`, NPCs use `job`, Activities use `activity`. All serve the same K8s `kind` role: the enum variant that selects a registry entry.

## The Three Layers

### 1. CRD — Schema (`XxxDef` struct)

Defines the shape of a type. Immutable. No runtime state.

```rust
// constants.rs — the CRD schema
pub struct NpcDef {
    pub job: Job,
    pub label: &'static str,
    pub base_hp: f32,
    pub base_damage: f32,
    pub base_speed: f32,
    pub default_attack_type: BaseAttackType,
    // ... sprite, flags, upgrade_category
}
```

K8s equivalent: `apiVersion: endless/v1, kind: NpcDef` — the type declaration that says "an NPC has these fields".

### 2. etcd — Registry (`XXX_REGISTRY` array)

Stores all instances of the schema. One entry per variant. Compile-time constant.

```rust
// constants.rs — etcd storage
pub const NPC_REGISTRY: &[NpcDef] = &[ /* one per Job variant */ ];
pub fn npc_def(job: Job) -> &'static NpcDef { /* lookup by key */ }
```

K8s equivalent: `kubectl get npcdefs` — the stored definitions that controllers read from.

### 3. CR — Instance (ECS entity + components)

A single runtime object created from a registry entry. Each component owns one concern.

```
NPC Entity (= one CR instance)
  ├─ NpcStats        { name, xp }           // metadata — identity + progression
  ├─ CachedStats     { damage, range, ... }  // status — resolved from spec
  ├─ NpcEquipment    { helm, armor, ... }    // spec input — per-slot loot items
  ├─ Activity        { kind, target_pos }    // status — current behavior
  ├─ Health(f32)                              // status — runtime HP
  ├─ Speed(f32)                              // status — resolved movement speed
  └─ GpuSlot(usize)                         // metadata — GPU buffer index
```

Lightweight index entry in `EntityMap` for slot↔entity mapping (like a K8s label index):
```rust
// entity_map.rs
pub struct NpcEntry {
    pub slot: usize,
    pub entity: Entity,
    pub job: Job,
    pub faction: i32,
    pub town_idx: i32,
}
```

### 4. Controller — Systems (reconcile loop)

Systems read the registry (etcd) and instance state (CR), then reconcile. Never cache Def fields on instances.

```
materialize_npc()           ← "kubectl apply" — reads NpcDef from registry, spawns entity + components
resolve_combat_stats()      ← reconcile — reads NpcDef.base_hp/damage/speed + upgrades + equipment → CachedStats
process_upgrades_system()   ← re-reconcile — upgrade purchased → re-resolve stats from Def
```

Key rule: the registry Def is the **source of truth** for base values (spec). Systems derive runtime state (status) from `Def × upgrades × equipment × level × personality`. If a base stat changes in the registry, all entities pick it up on next reconcile.

## Building Example (100%)

Same pattern, different shape:

| Layer | K8s Analogue | Implementation |
|-------|-------------|---------------|
| **CRD** | Schema | `BuildingDef` struct — cost, hp, tile, tower_stats, spawner config |
| **etcd** | Storage | `BUILDING_REGISTRY` array + `building_def(kind)` lookup |
| **CR** | Instance | Slim `BuildingInstance` (5 fields: kind, position, town_idx, slot, faction) + ECS components: `ProductionState`, `TowerBuildingState`, `SpawnerState`, `ConstructionProgress`, `WaypointOrder`, `WallLevel`, `MinerHomeConfig`. Occupancy tracked separately in `EntityMap.occupancy` |
| **Controller** | Reconcile | `place_building()` reads BuildingDef → spawns entity + components. `BuildingOverrides` for initial config |

## 100% Compliance Checklist

An entity type is fully compliant when:

- [x] **CRD:** Static `XxxDef` struct in `constants.rs`
- [x] **etcd:** `XXX_REGISTRY` array + `xxx_def(key)` lookup
- [x] **CR:** All runtime state lives on ECS components (no parallel arrays, no god-structs)
- [x] **CR index:** Slim index entry if needed (spatial/slot lookup only — no gameplay state)
- [x] **Controller:** Systems read Def at spawn/reconcile time, never cache Def fields on instances
- [x] **Extensibility:** Adding a new variant = 1 enum variant + 1 registry entry

Buildings satisfy all six. Use Buildings as the reference when bringing other entity types to compliance.

## Current Compliance

| Entity | CRD (schema) | etcd (registry) | CR (instance) | Controller | Score |
|--------|-------------|-----------------|---------------|------------|-------|
| NPCs | `NpcDef` | `NPC_REGISTRY` | ECS components (NpcStats, CachedStats, etc.) | `resolve_combat_stats`, `process_upgrades_system` | 95% |
| Buildings | `BuildingDef` | `BUILDING_REGISTRY` | Slim `BuildingInstance` (5-field identity) + ECS components | `place_building` | 100% |
| Activities | `ActivityDef` | `ACTIVITY_REGISTRY` | Fieldless `ActivityKind` + `Activity` struct | `def()` lookups | 100% |
| Towns | `TownDef` | `TOWN_REGISTRY` | Slim 4-field index + ECS components via `TownAccess` SystemParam | spawn + economy systems | 100% |
| Items | `ItemDef` | `ITEM_REGISTRY` + `item_def(kind)` | `LootItem` + `NpcEquipment` | `roll_loot_item()` reads `item_def()` | 85% |
