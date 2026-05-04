# Grounded 2 -- Mod Inspection (Worked Examples)

How to fully decompile and understand Grounded 2 (Unreal Engine 5) mods
so that you can list every change a mod makes, identify the exact assets
it touches, and diagnose why a previously-working mod is now broken.

> **This is generic UE5 modding.** Grounded 2 ships standard UE 5.4
> cooked content with no encryption, no custom container format, and
> no special anti-mod measures. Every technique, tool, and workflow
> in this document works against any other UE 5.x game (Hogwarts
> Legacy, Stalker 2, Lies of P, Black Myth Wukong, Abiotic Factor,
> Crash Team Rumble, etc.). The only game-specific knowledge needed
> is which asset paths and property names control the behaviour you
> want to change. Once you have that, the cooking / packing pipeline
> is the same. So a "Standard UE5 modding guide" applies directly --
> see [External resources](#external-resources--modding-ecosystem-references)
> for the canonical ones.

Two worked examples in this document:

1. **All-in-One Player Tweaks v13.1.6** -- a working mod that overrides
   one Blueprint (`BP_SurvivalPlayerCharacter`).
2. **Bigger Backpack v37.1.2** (`ContainerWidgetTweaks_00054_P`) --
   a mod that no longer works in the current Grounded 2 build.
   Used here to show how to root-cause a broken mod.

## Table of contents

- [TL;DR](#tldr)
- [File format primer](#file-format-primer)
- [Phase 1 -- Tooling](#phase-1----tooling)
- [Phase 2 -- Inventory (what files does it touch?)](#phase-2----inventory-what-files-does-it-touch)
- [Phase 3 -- Bulk extract](#phase-3----bulk-extract)
- [Phase 4 -- Vanilla baseline](#phase-4----vanilla-baseline)
- [Phase 5 -- Diff](#phase-5----diff)
- [Phase 6 -- Interpret each asset type](#phase-6----interpret-each-asset-type)
- [Phase 7 -- Sanity checks](#phase-7----sanity-checks)
- [Phase 8 (optional) -- Behavioural verification](#phase-8-optional----behavioural-verification)
- [Quick reference -- minimum-effort path](#quick-reference----minimum-effort-path)
- [CLI-driven alternative path (retoc)](#cli-driven-alternative-path-retoc)
- [Worked example 1: All-in-One Player Tweaks](#worked-example-1-all-in-one-player-tweaks)
- [Worked example 2: Bigger Backpack (broken)](#worked-example-2-bigger-backpack-broken)
- [Mod interaction analysis: AIO + Bigger Backpack](#mod-interaction-analysis-aio--bigger-backpack)
- [Building our own backpack mod -- requirements](#building-our-own-backpack-mod----requirements)
- [SDK research findings (definitive technical path)](#sdk-research-findings-definitive-technical-path)
- [Caveats](#caveats)

## Mod locations on this machine

```
C:\Users\Abix\AppData\Roaming\Vortex\grounded2\mods\
  All-in-One Player Tweaks-13-1-6-1776519922\
    Augusta\Content\Paks\
      AIOPlayerTweaks_00012_P.{pak,ucas,utoc}
  Bigger Backpack-37-1-2-1769621822\
    Augusta\Content\Paks\
      ContainerWidgetTweaks_00054_P.{pak,ucas,utoc}
```

## TL;DR

UE5 mod paks are sparse asset overrides. The fastest path to 100%
understanding is:

1. Open the mod in **FModel** alongside the base game.
2. Export every overridden asset as JSON.
3. Export the same paths from vanilla.
4. **WinMerge** (or `code --diff`) the two folders.

Each diff entry = one tweak. Done.

There is no executable code to "decompile" -- the payload is cooked
UAsset binaries containing data tables, curves, and Blueprint default
property blocks. Tools surface those as JSON; differences are the mod's
entire feature set.

## Game-level findings (2026-05-04 session)

Probed with `retoc info` against the actual files on disk:

- **Game install:** `C:\Games\Steam\steamapps\common\Grounded2\`
- **Game version string:** `++Augusta+release-0.4.0.2-CL-2673661`
  (from `Grounded2.exe` ProductVersion -- this is the bootstrap shim;
  UE engine version is inferred from container format flags below).
- **Base game paks (Augusta\Content\Paks\):** one base pak
  `Augusta-WinGRTS.{pak,ucas,utoc}` plus `global.{ucas,utoc}`. No
  patch paks shipped. Single monolithic loadout. The base pak is
  **32 GB** containing **61,449 packages** / 100,196 chunks.
- **No AES encryption.** Both the global and mod containers report
  `container_flags: 0x0` / `Indexed` -- the Encrypted flag is absent.
  No AES key needed; retoc/FModel can read everything directly.
- **TOC version:** `ReplaceIoChunkHashWithIoHash` (latest IoStore TOC).
- **Container header version:** `SoftPackageReferencesOffset` (latest).
  Together these confirm UE 5.4+ as the engine version.
- **Mod mount point:** `../../../` (standard UE relative-from-Paks
  mount; resolves into the game's `/Game/...` virtual path).

Implication: clean baseline. No encryption barrier; standard IoStore
format; single base pak makes targeted lookups fast via
`retoc list --path | grep`.

## File format primer

| File           | Role                                                        |
|----------------|-------------------------------------------------------------|
| `*.pak`        | Legacy mount stub. Tiny. UE's mount API entry point.        |
| `*.utoc`       | IoStore Table of Contents. Chunk-id index.                  |
| `*.ucas`       | IoStore Container Archive Storage. Compressed payload.      |

Naming convention `AIOPlayerTweaks_00012_P`:

- `_P` suffix flags the file as a **patch pak**. UE loads patch paks
  last so they shadow base game assets at the same path.
- `00012` is the load priority. Higher numbers win when multiple mods
  touch the same asset. This mod is at slot 12.
- `Augusta` (the parent folder) is Obsidian's internal project name for
  Grounded 2. Mods must mirror the project's `Content/Paks/` path or UE
  will not mount them.

A mod pak only contains the assets it overrides -- the entire base game
is not duplicated. So the file tree IS the changelist of WHAT gets
touched. The diffing phase tells you HOW MUCH each asset changed.

## Phase 1 -- Tooling

| Tool          | Purpose                                       | Source                                |
|---------------|-----------------------------------------------|---------------------------------------|
| retoc (CLI)   | Probe + unpack `.utoc`/`.ucas`, Zen->Legacy   | github.com/trumank/retoc/releases     |
| FModel (GUI)  | Browse paks, export properties to JSON        | github.com/4sval/FModel/releases      |
| WinMerge      | Folder/file diff for the JSON outputs         | https://winmerge.org                  |
| UAssetGUI     | Byte-level UAsset inspection (rarely needed)  | github.com/atenfyr/UAssetGUI          |

Optional: VS Code's built-in `code --diff a.json b.json` works fine for
spot diffs without WinMerge.

### Local install used for this worked example

Both tools are portable single-binary apps; no installer required.

```
C:\Tools\retoc\retoc.exe       (v0.1.5,  ~7 MB,  CLI)
C:\Tools\FModel\FModel.exe     (Dec 2025, ~46 MB, GUI, self-contained)
```

Download via curl + extract via unzip:

```bash
mkdir -p /c/Tools/retoc /c/Tools/FModel
curl.exe -L -o /c/Tools/retoc/retoc.zip \
  https://github.com/trumank/retoc/releases/download/v0.1.5/retoc_cli-x86_64-pc-windows-msvc.zip
curl.exe -L -o /c/Tools/FModel/FModel.zip \
  https://github.com/4sval/FModel/releases/download/dec-2025/FModel.zip
cd /c/Tools/retoc  && unzip -o retoc.zip
cd /c/Tools/FModel && unzip -o FModel.zip
```

### retoc CLI subcommands actually used

| Subcommand    | Purpose                                              |
|---------------|------------------------------------------------------|
| `info <utoc>` | Show container metadata (chunks, packages, version) |
| `list <utoc>` | List chunk IDs (raw, not directory paths)            |
| `manifest`    | Extract manifest                                     |
| `to-legacy`   | Convert Zen-format IoStore to legacy `.uasset` pak   |
| `unpack`      | Extract chunks to files                              |

The `to-legacy` command is the most useful: it produces a legacy
`.pak` containing readable `.uasset`/`.uexp` files that downstream
tools (UAssetGUI, FModel, kismet-analyzer) all understand directly.

## Phase 2 -- Inventory (what files does it touch?)

1. Install FModel.
2. **Settings -> General -> Output Directory**: pick a working folder,
   e.g. `C:\fmodel_out\`.
3. **Settings -> Game -> Detect**: point at
   `<SteamLibrary>\steamapps\common\Grounded 2\Augusta\Content\Paks\`.
   FModel auto-fills the UE version. Verify it matches the game's
   actual UE version (right-click `Grounded 2.exe` -> Properties ->
   Details -> Product Version).
4. Copy the three mod files (`*.pak`, `*.ucas`, `*.utoc`) into that same
   `Paks\` directory temporarily. FModel needs the base game's
   `global.utoc` to resolve chunk references the mod makes.
5. Load FModel. In the left **Archives** panel, locate
   `AIOPlayerTweaks_00012_P` and expand its tree.
6. Save the file tree as text (right-click root -> Copy Path tree, or
   screenshot per folder). This is your master inventory.

Typical paths to expect for a "Player Tweaks" mod:

```
/Game/Data/Player/...
/Game/Blueprints/Character/BP_PlayerCharacter
/Game/Data/Survival/DT_*
/Game/Curves/CT_*
/Game/UI/...           (rare)
```

## Phase 3 -- Bulk extract

Right-click the mod's archive in FModel -> **Export Folder's Packages
Data (.json)**. Every `.uasset` in the mod becomes a `.json` file in
your output directory, mirroring the in-game folder layout.

Result: `C:\fmodel_out\Augusta\Content\Data\...\DT_PlayerStats.json` and
so on. This is the entire mod payload in human-readable form.

## Phase 4 -- Vanilla baseline

The mod JSON shows final values, not deltas. To diff, you need the same
paths exported from vanilla.

1. In FModel, uncheck the mod in the Archives panel (or move the three
   mod files out of `Paks\`).
2. Reload. The same paths now resolve to the base game's versions.
3. Set **Output Directory** to a parallel folder, e.g.
   `C:\vanilla_out\`.
4. For each path the mod touched, export properties to JSON (same
   right-click action, but on the vanilla file).

Faster batched approach: for each top-level folder the mod touches,
right-click the vanilla version of that folder -> Export Folder's
Packages Data. You will export more than you need, but disk is cheap
and the diff step ignores files that have no counterpart.

## Phase 5 -- Diff

```powershell
# Folder-level summary of which files differ
Compare-Object `
  (Get-ChildItem C:\vanilla_out -Recurse -File -Filter *.json | Select-Object -ExpandProperty FullName) `
  (Get-ChildItem C:\fmodel_out  -Recurse -File -Filter *.json | Select-Object -ExpandProperty FullName)

# Per-file diff in VS Code
code --diff C:\vanilla_out\path\to\DT_PlayerStats.json C:\fmodel_out\path\to\DT_PlayerStats.json
```

Or open WinMerge in folder-compare mode pointed at both directories.
WinMerge highlights the changed files in red, and double-clicking opens
a side-by-side view with property-level highlights.

## Phase 6 -- Interpret each asset type

| Asset type   | JSON shape                                | What "tweaks" usually mean                  |
|--------------|-------------------------------------------|---------------------------------------------|
| `DT_*`       | `Rows: { RowName: { ...properties } }`    | Stat rows changed: damage, weight, stamina  |
| `CT_*`       | `RowMap: { CurveName: { Keys: [...] } }`  | Curve points moved: XP, decay, scaling      |
| `BP_*`       | `Properties` block on the CDO             | Default field changed: `MaxHealth: 100->250`|
| `*Settings`  | Flat property block                       | Toggle flags flipped                        |
| Char movement defaults | Properties on a `CharacterMovementComponent` | `MaxWalkSpeed`, `JumpZVelocity`, etc. |

For each diff entry record:

- Asset path
- Property name
- Vanilla value
- Modded value
- Plain-English meaning (what gameplay system this controls)

Once every changed file is processed, you have a 100% inventory of the
mod's data-driven changes.

## Phase 7 -- Sanity checks

Things that a naive FModel-only pass can miss:

1. **Chunk count cross-check.** Run ZenTools and compare to FModel:

   ```powershell
   ZenTools.exe unpack AIOPlayerTweaks_00012_P.utoc C:\zentools_out\
   (Get-ChildItem C:\zentools_out -Recurse -File).Count
   ```

   This should match the count of leaf nodes in FModel's tree. A
   mismatch means FModel filtered or skipped something.

2. **Encrypted blobs.** If FModel shows entries marked `<encrypted>` or
   `<unknown>`, you are missing an AES key. Grounded 2 ships with one;
   community wiki / FModel Discord publishes it. Without it, `.ucas`
   bytes are unreadable and "100%" is impossible.

3. **Override conflict check.** Other installed mods with priority
   higher than `_00012_P` will shadow this mod for any overlapping
   asset paths. List installed mod paks and their priorities before
   declaring final values authoritative.

4. **External asset references.** Mods sometimes reference assets that
   live in vanilla:

   ```powershell
   Select-String -Path C:\fmodel_out\*.json -Pattern '"PackagePath"' `
     | Group-Object Line `
     | Sort-Object Count -Descending
   ```

   Unfamiliar paths in the output may indicate a dependency on game
   content the modder did not ship.

## Phase 8 (optional) -- Behavioural verification

Static JSON analysis covers DataTables, CurveTables, and Blueprint
property defaults. It does NOT cover Blueprint EventGraph logic
(visual-scripting nodes serialised as kismet bytecode).

Most "All-in-One Tweaks" mods are pure data and do not touch
EventGraphs. To verify:

- Look for `BP_*.json` files in your export with non-trivial size and a
  `FunctionsBytecode` field. Trivial CDO-only overrides have no
  bytecode.
- If a BP has bytecode that differs from vanilla, dump it via
  `kismet-analyzer` or open in UAssetGUI for manual inspection.
- Failing that: empirical test with the mod toggled on/off, comparing
  the specific in-game system you suspect.

For a tweaks mod (vs a content/scripted mod), data coverage is
effectively complete coverage.

## Quick reference -- minimum-effort path

Skip all phases except 3 and 5 if you only need a rough sense of
what the mod does:

1. FModel -> load mod -> right-click pak -> Export Folder's Packages
   Data (.json).
2. Skim every JSON. Property names tell you the system; values tell you
   the new setting.
3. Manually look up vanilla values for the 3-5 properties whose changes
   actually matter to you.

Gets you ~90% understanding in ~15 minutes. Phases 4-7 raise that to
100% over a couple of hours.

## CLI-driven alternative path (retoc)

For this mod the workflow is faster via retoc CLI than via FModel GUI,
because there is exactly one package to extract.

```bash
# 1. Stage globals + mod into one working dir (retoc to-legacy needs both).
mkdir -p /c/Tools/work/inputs /c/Tools/work/extracted
cp /c/Games/Steam/steamapps/common/Grounded2/Augusta/Content/Paks/global.utoc \
   /c/Games/Steam/steamapps/common/Grounded2/Augusta/Content/Paks/global.ucas \
   /c/Tools/work/inputs/
cp "/c/Users/Abix/AppData/Roaming/Vortex/grounded2/mods/All-in-One Player Tweaks-13-1-6-1776519922/Augusta/Content/Paks/AIOPlayerTweaks_00012_P."{pak,ucas,utoc} \
   /c/Tools/work/inputs/

# 2. Probe: confirms encryption status, version, package count.
/c/Tools/retoc/retoc.exe info /c/Tools/work/inputs/AIOPlayerTweaks_00012_P.utoc

# 3. Convert Zen-format IoStore -> legacy pak with readable .uasset files.
/c/Tools/retoc/retoc.exe to-legacy /c/Tools/work/inputs \
                                   /c/Tools/work/mod_legacy.pak

# 4. List the converted pak's contents (which asset paths were touched).
/c/Tools/retoc/retoc.exe list /c/Tools/work/mod_legacy.pak
```

Then load `mod_legacy.pak` in FModel (or UAssetGUI) for property
decoding -- legacy `.uasset` files are the well-supported common
format across all UE inspection tools.

### Listing a legacy pak

`retoc list` only handles IoStore `.utoc`. To list a legacy `.pak`
produced by `to-legacy`, use **repak** (companion CLI by the same
author):

```bash
# One-time install, single-binary portable.
mkdir -p /c/Tools/repak
curl.exe -L -o /c/Tools/repak/repak.zip \
  https://github.com/trumank/repak/releases/download/v0.2.3/repak_cli-x86_64-pc-windows-msvc.zip
cd /c/Tools/repak && unzip -o repak.zip

# List + unpack
/c/Tools/repak/repak.exe list   /c/Tools/work/mod_legacy.pak
/c/Tools/repak/repak.exe unpack /c/Tools/work/mod_legacy.pak --output /c/Tools/work/mod_unpacked
```

### Targeted vanilla extraction

Do NOT run `retoc to-legacy` on the entire base game pak unless you
have ~100 GB free and an hour to spare. Grounded 2's
`Augusta-WinGRTS.ucas` is 32 GB / 61,449 packages.

Instead, list the vanilla index and pull only the chunks you need:

```bash
# 1. Print every asset path in vanilla, grep for what the mod overrides.
/c/Tools/retoc/retoc.exe list --path \
  /c/Games/Steam/steamapps/common/Grounded2/Augusta/Content/Paks/Augusta-WinGRTS.utoc \
  | grep -i "BP_SurvivalPlayerCharacter"

# 2. (Workflow: extract just that chunk -- TBD; retoc unpack is
#     all-or-nothing right now. Practical alternative: use FModel GUI
#     to navigate to the path and Save Package.)
```

## Worked example 1: All-in-One Player Tweaks

After running the CLI workflow against
`AIOPlayerTweaks_00012_P.{pak,ucas,utoc}`:

```
$ retoc info AIOPlayerTweaks_00012_P.utoc
  container_flags: EIoContainerFlags(Indexed)
  version: ReplaceIoChunkHashWithIoHash
  mount_point: ../../../
  chunks: 2
  packages: 1
  container_header_version: Some(SoftPackageReferencesOffset)

$ retoc to-legacy ./inputs ./mod_legacy.pak
  Extracted 1 (0 failed) legacy assets to "mod_legacy.pak"
  Extracted 0 shader code libraries to "mod_legacy.pak"

$ repak list ./mod_legacy.pak
  BP_SurvivalPlayerCharacter.uasset    (133,919 bytes)
  BP_SurvivalPlayerCharacter.uexp      (197,482 bytes)
  scriptobjects.bin                    (retoc bookkeeping)
```

Vanilla path of the overridden asset (from `retoc list --path`):

```
../../../Augusta/Content/Blueprints/Player/BP_SurvivalPlayerCharacter.uasset
```

### What it actually does

A string-dump of the `.uasset`+`.uexp` reveals the mod is **not** a
stat tweaker -- it is a **cheat / unlock mod** that runs gameplay-tag
commands at spawn time. The Blueprint contains command literals like:

```
AddBuggyUpgradePoints 44
AddPartyUpgradePoints 30
AddPersonalUpgradePoints 47
UnlockAllPerks
UnlockAllRecipes Default
UnlockAllTechTrees
UnlockSCAB
UnlockBuggyUpgrade BuggyHealing
UnlockBuggyUpgrade BuggyHealthPoolOnRevive
UnlockBuggyUpgrade BuggyInventorySize
UnlockBuggyUpgrade BuggyMaxHealth
UnlockBuggyUpgrade BuggyMaxStamina
UnlockItemStackUpgrade StackSize.Ammo
UnlockItemStackUpgrade StackSize.Food
UnlockItemStackUpgrade StackSize.Resource
UnlockPlayerUpgrade Health
UnlockPlayerUpgrade healing
UnlockPlayerUpgrade perks
UnlockPlayerUpgrade stamina
UnlockPlayerUpgrade thirst
```

These look like console-style cheat commands fed through the
`UseGameplayCheatCommand`-style API. Plus toast strings:
`"Recipes unlocked."`, `"Tech trees unlocked."` -- the mod surfaces
on-screen confirmations.

**Important terminology note:** in Grounded 2, **"Buggy" = mountable
creature** (Ladybug, Orb Weaver, etc. -- the ride-able insects with
their own saddlebag inventories), NOT the player's backpack.
Confirmed by vanilla asset paths like `CA_MountBuggy`,
`AS_LadyBug_Buggy_ShootIdle`, `Table_Items_Ammo_LadybugBuggy`,
`MI_BLD_BuggyBuff_A`. So `UnlockBuggyUpgrade BuggyInventorySize`
unlocks **the mount creature's saddlebag** upgrade, not the player's
backpack.

**Conclusion:** AIO Player Tweaks unlocks the entire tech tree,
recipes, perks, and gives upgrade points across player + party + buggy
upgrade trees. It does NOT directly modify the player's backpack
capacity. The closest backpack-related thing it does is grant stack
size upgrades (`StackSize.Ammo|Food|Resource`), which let single
slots hold more items.

## Worked example 2: Bigger Backpack (broken)

Goal: identify why "Bigger Backpack" v37.1.2 is no longer working in
the current Grounded 2 build (game version
`++Augusta+release-0.4.0.2-CL-2673661`).

### Recon

The Vortex display name is "Bigger Backpack" but the internal pak is
`ContainerWidgetTweaks_00054_P` -- a **UI** widget tweak, not an
inventory data-model tweak. That is the first major clue.

```
$ retoc info ContainerWidgetTweaks_00054_P.utoc
  container_flags: EIoContainerFlags(Indexed)
  version: ReplaceIoChunkHashWithIoHash
  mount_point: ../../../
  chunks: 4
  packages: 3
  container_header_version: Some(SoftPackageReferencesOffset)

$ retoc list --path ContainerWidgetTweaks_00054_P.utoc
  9776fd889ac44a7c00000001 ExportBundleData ../../../UI_Container_BackpackSide.uasset
  87682ba793f6f4e100000001 ExportBundleData ../../../UI_Container_ContainerSide.uasset
  3c31abdd0e09f75d00000001 ExportBundleData ../../../UI_ContainerInterface.uasset
  9c6034ae72115fce00000006 ContainerHeader  -
```

Three overridden UMG widgets:

| Widget                          | Schema (.uasset) | Payload (.uexp) |
|---------------------------------|------------------|-----------------|
| `UI_Container_BackpackSide`     | 15.8 KB          | 13.6 KB         |
| `UI_Container_ContainerSide`    | 26.7 KB          | 33.8 KB         |
| `UI_ContainerInterface`         | 79.7 KB          | 145.4 KB        |

### Vanilla cross-reference (chunk-ID match)

Looking up the same chunk IDs in vanilla `Augusta-WinGRTS.utoc`:

```
$ retoc list --path Augusta-WinGRTS.utoc | grep -iE 'UI_Container_(BackpackSide|ContainerSide)|UI_ContainerInterface'
  9776fd889ac44a7c00000001 ../../../Augusta/Content/UI/Container/UI_Container_BackpackSide.uasset
  87682ba793f6f4e100000001 ../../../Augusta/Content/UI/Container/UI_Container_ContainerSide.uasset
  3c31abdd0e09f75d00000001 ../../../Augusta/Content/UI/Container/UI_ContainerInterface.uasset
```

**Chunk IDs match exactly.** All three widgets still exist at the
expected vanilla paths with identical chunk IDs. So the override
**resolves correctly** -- the asset itself is being loaded at runtime.

### Path discrepancy in mod TOC (cosmetic, not the bug)

The mod's TOC shows paths as `../../../UI_Container_BackpackSide.uasset`
(missing the `Augusta/Content/UI/Container/` directory tree), but
IoStore lookup is by chunk ID hash, not directory path. The chunk-ID
hash matches, so the override works. The stripped-down path is
cosmetic -- likely a side-effect of how the modder packaged the
files. Not the cause of the breakage.

### What it actually changes

A string-dump of all three modded widgets shows that out of every
grid-dimension property name UE supports (NumColumns, NumRows,
MaxRows, MaxColumns, RowCount, ColumnCount, GridSize, SlotCount,
CellSize, etc.), exactly **one** appears: `MaxRows`.

Plus references to relevant inventory-side concepts:

```
/Game/UI/UI_InventoryGrid                  <- generic grid widget class
/Game/UI/Container/UI_Container_BackpackSide
/Game/UI/Container/UI_Container_ContainerSide
PlayerInventoryGrid                        <- the bound grid instance
MountInventoryComponent                    <- mount creature inventory
GetContainerInventory
GetInventoryFromBackpackCategory
ContainerSpaceForItem
InventorySpaceForItem
ClearInventoryGrid
bRefreshOnOwnerInventoryChange
```

So the mod's strategy is:

1. Override the three container widgets (`UI_Container_BackpackSide`,
   `UI_Container_ContainerSide`, `UI_ContainerInterface`) so they
   embed an `UI_InventoryGrid` instance with a higher `MaxRows`
   value than vanilla.
2. Hope that the embedded grid's `MaxRows` actually drives the
   number of slots rendered (and that the inventory data side
   allows that many items to be stored).

### Hypothesis -- where the bug actually is

Since the override resolves correctly but the mod has no effect, the
bug is **inside the widget logic**, not in container plumbing. Most
likely:

- **Capacity is data-driven, not widget-driven.** The current
  Grounded 2 build apparently sources the rendered slot count from
  the player's `MountInventoryComponent` / inventory-data side,
  not from the widget's `MaxRows`. The widget property the modder
  bumped is now cosmetic at best -- when the grid binds, it uses
  the data-side value and ignores the layout hint.
- This fits the symptom "mod loads but does nothing in the
  current build" better than a serialisation failure (which would
  cause crashes or visible UI corruption).

### Next step to confirm

Decode `UI_Container_BackpackSide.uexp` and read the actual
`MaxRows` value the mod sets. If it is, e.g., `12` while vanilla
sets `6`, the mod's intent is confirmed -- and the failure is purely
on the game's read-path no longer honouring it. Use FModel on
`bb_legacy.pak` (parser set to UE 5.4) and right-click each widget
-> Export Properties (.json), then compare against vanilla widget
JSON dumps.

## Mod interaction analysis: AIO + Bigger Backpack

**Initial conclusion (revised):** an earlier draft of this document
claimed these mods do not interact. **That was wrong.** Web research
against the mod authors' own documentation on Nexus confirms they
**must be paired**:

> "The actual change to backpack size is on the Player Tweaks side,
> and you need any version with 60 slots backpack (in Optional
> Files). The Bigger Backpack mod only changes the UI widget for
> containers to let you seamlessly manage all 60 slots when
> interacting with containers."
> -- summary of the Bigger Backpack Nexus comments thread.

So the correct division of labour is:

| Mod                  | Touches                              | Mechanism                                 | Role |
|----------------------|--------------------------------------|-------------------------------------------|------|
| Player Tweaks (60-slot variant) | `BP_SurvivalPlayerCharacter`  | Sets data-side capacity on the inventory component to 60 | Enables 60-slot storage |
| Bigger Backpack      | 3 UMG container widgets              | Sets widget layout (`MaxRows`) so all 60 slots are visible | Renders the extra slots |

Why the v13.1.6 AIO variant we have on disk does NOT include the
60-slot bump:

- The AIO build's string dump shows only cheat-tag commands (recipe
  unlocks, perk unlocks, point grants, mount/buggy inventory size
  unlock). It contains no literal `60` or `MaxInventorySize` ASCII
  marker that we could grep for. Numeric property defaults in
  `.uexp` are stored as raw bytes, not strings, so a grep miss is
  inconclusive -- but combined with the Nexus author's note that
  the 60-slot version is in **Optional Files** (a separate
  download), the inference is that the standard AIO this user
  downloaded is the cheats-only variant without the slot bump.

Why Bigger Backpack appears "broken" for this user:

- The user has AIO Player Tweaks v13.1.6 installed (the
  cheats-only variant) but **not** the 60-slot Player Tweaks
  variant. Therefore the data-side capacity is still vanilla 40,
  and Bigger Backpack's UI expansion has only 40 actual slots to
  bind to -- the visible result is "no extra slots", which the
  user reports as "doesn't work".

Conflict surface (now relevant): both AIO and the 60-slot Player
Tweaks variant override the same asset
(`BP_SurvivalPlayerCharacter`). Per Nexus comments,
"AIO_Player_Tweaks conflicts with Bigger Backpack when used
together with Convenience Tweaks, AIO Player Tweaks Plus 60
slots backpack..." This means **only one Player Tweaks variant
can be installed at a time**. The user must choose between:

- AIO (current install) -- cheats and unlocks but no slot bump
- AIO + 60-slot variant (a combined "Plus 60 slots" optional
  file from the Player Tweaks page on Nexus) -- both cheats AND
  slot bump
- 60-slot-only variant -- slot bump only, no cheats

## Recommended path forward (low effort)

**Easiest fix for "I want a bigger backpack":**

1. Go to the Player Tweaks Nexus page
   (https://www.nexusmods.com/grounded2/mods/13).
2. Open the **Optional Files** tab.
3. Download the variant that contains "60 slots" (likely titled
   "AIO Player Tweaks Plus 60 slots backpack" or "Player Tweaks 60
   slots only").
4. Replace the current `AIOPlayerTweaks_00012_P.{pak,ucas,utoc}`
   in Vortex with this variant.
5. Keep the existing Bigger Backpack install
   (`ContainerWidgetTweaks_00054_P`) as-is -- it will start
   functioning the moment the data-side capacity is in place.

This is a **"download the right file"** fix, not a build-our-own-mod
job. No DIY engineering required.

## DIY backpack mod (if no acceptable variant exists)

If the Optional Files on Nexus don't have a usable 60-slot
variant for the current Grounded 2 build (`++Augusta+release-0.4.0.2-CL-2673661`),
or the user wants a different slot count, here is how we'd
build one ourselves.

### What the 60-slot Player Tweaks variant must do

Based on the Nexus author note, the variant overrides
`BP_SurvivalPlayerCharacter` and increases a numeric property
that controls the player's backpack capacity. The property is
NOT a string we can grep for (numeric properties live in the
`.uexp` as raw bytes), so identifying it precisely requires a
proper UAsset deserialiser.

### Tooling needed for DIY mod authoring

| Tool | Role | Install |
|------|------|---------|
| **UAssetGUI** | Edit cooked .uasset properties; export to/from JSON | github.com/atenfyr/UAssetGUI |
| **FModel** (already installed) | Identify property names + values via Save Properties (.json) | C:\Tools\FModel\ |
| **Mappings file** | Required for UE 5.3+ to decode property names | Generated via Dumper-7 against the running game, OR community-published .usmap |
| **retoc** (already installed) | Repack legacy assets back to Zen IoStore for the final mod pak | C:\Tools\retoc\ |
| **repak** (already installed) | Build the legacy `.pak` shell | C:\Tools\repak\ |

The mappings file is the missing piece. UE 5.3+ stripped property
names from cooked assets to save space; without an external
mappings file, FModel/UAssetGUI can decode structure but property
names show as `Property_<hash>`. Either:

- Generate one with **Dumper-7** against the live game process
  (https://github.com/Encryqed/Dumper-7), which dumps SDK + mappings.
- Find a community-published `.usmap` for Grounded 2 (the modding
  Discord likely has one).

### Build workflow

```bash
# 1. Extract vanilla BP_SurvivalPlayerCharacter from the base pak.
#    Find chunk ID:
/c/Tools/retoc/retoc.exe list --path \
  /c/Games/Steam/steamapps/common/Grounded2/Augusta/Content/Paks/Augusta-WinGRTS.utoc \
  | grep -i "BP_SurvivalPlayerCharacter\.uasset"
# Extract via FModel (Browse to the path, right-click Save Package)
# OR via retoc unpack (whole-pak operation -- slow).

# 2. Open in UAssetGUI with Mappings file loaded, parser at UE 5.4.
#    Find the inventory component's capacity default. Likely candidates:
#    - InventorySize, MaxInventorySize, MaxItems, NumSlots, Capacity
#    - On a child component named PlayerInventoryComponent or similar
#    - May be a struct field, e.g., InventoryConfig.MaxSize

# 3. Edit the value to 60 (or whatever target).

# 4. Save the modified .uasset. Save .uexp alongside it.

# 5. Build a legacy pak with the modified files.
/c/Tools/repak/repak.exe pack ./our_mod_files ./our_mod_legacy.pak

# 6. Convert the legacy pak to Zen IoStore for the actual mod.
/c/Tools/retoc/retoc.exe to-zen ./our_mod_legacy.pak our_mod.utoc \
                                 --version UE5_4

# 7. Name the output with a load-priority suffix higher than other
#    Player Tweaks variants but distinct, e.g.
#    BackpackOnly_00099_P.{pak,ucas,utoc}.

# 8. Drop into Vortex / Paks dir, launch game, verify in-game.
```

### Risks / unknowns for DIY path

- **Property name discovery without mappings is hard.** If we
  can't get a `.usmap`, we cannot reliably identify which
  numeric default to edit. UAssetGUI on a no-mappings install
  shows raw byte offsets but not human-readable property names.
- **Save compatibility.** A save made with capacity-60 may not
  load cleanly if the mod is later uninstalled (the extra items
  may be lost or corrupted). Player should empty extra slots
  before uninstall.
- **Multiplayer.** Server-authoritative inventory caps may
  reject client-side capacity bumps. The mod's effect could be
  silently overridden in multiplayer sessions where the host
  doesn't have the same mod.

### When DIY makes sense

- The Optional Files variant exists but conflicts with another
  mod the user wants to keep.
- The user wants a non-standard slot count (e.g. 50, 80, 100).
- The 60-slot variant breaks against a future Grounded 2 patch
  and has not been updated, but the property location is known.

For the immediate "I want a bigger backpack" need, **the
recommended path forward** above (download the 60-slot variant)
is the right answer. DIY is the fallback.

## External resources / modding ecosystem references

No official Obsidian / Grounded 2 modding SDK or documentation
exists. The community ecosystem covers the gap. As of May 2026:

### Authoritative mod pages on Nexus

- **Player Tweaks** (`mods/13`) -- the primary "AIO + capacity"
  mod family. Optional Files contain the 60-slot data-side bump.
  https://www.nexusmods.com/grounded2/mods/13
- **Bigger Backpack** (`mods/37`) -- UI-side widget mod. Pairs
  with Player Tweaks. https://www.nexusmods.com/grounded2/mods/37
- **Better Storages** (`mods/25`) -- world chest/storage capacity.
  Different system from player backpack.
  https://www.nexusmods.com/grounded2/mods/25
- **MoreInventory_Buggy** (`mods/102`) -- mount saddlebag size.
  Different system from player backpack.
  https://www.nexusmods.com/grounded2/mods/102
- **Bigger Stacks** (`mods/8`) -- per-item stack size up to x999.
  https://www.nexusmods.com/grounded2/mods/8
- **Grounded 2 Command List** (`mods/19`) -- comprehensive console
  command reference. Useful for finding gameplay tags.
  https://www.nexusmods.com/grounded2/mods/19

### General UE5 modding tooling

- **UAssetGUI** (atenfyr/UAssetGUI) -- the canonical cooked-uasset
  editor. https://github.com/atenfyr/UAssetGUI
- **Unofficial Modding Guide** -- UAssetGUI walkthrough, applicable
  to any UE 5.x cooked-asset target.
  https://unofficial-modding-guide.com/posts/uassetmodding/
  Plus an introduction at https://unofficial-modding-guide.com/posts/thebasics/
- **UE Modding Tools databank** (Buckminsterfullerene02/UE-Modding-Tools)
  -- comprehensive list of reverse-engineering and modding tools
  spanning multiple UE games.
  https://github.com/Buckminsterfullerene02/UE-Modding-Tools
- **Buckminster's UE Modding dev-guide** -- a comprehensive
  introduction including pak handling and patching.
  https://buckminsterfullerene02.github.io/dev-guide/Basis/DealingWithPaks.html
  https://buckminsterfullerene02.github.io/dev-guide/Basis/PakPatching.html
- **Dmgvol/UE_Modding** -- UE4/5 modding guides repo, includes
  IoStore packing instructions. Applies to any UE5 game.
  https://github.com/Dmgvol/UE_Modding
  https://github.com/Dmgvol/UE_Modding/blob/main/BasicModding/IoStorePacking.md
- **Dumper-7** (Encryqed/Dumper-7) -- generates SDK + mappings file
  by attaching to a running UE game. Required for property-name
  resolution on stripped-cooked assets in UE 5.3+.
  https://github.com/Encryqed/Dumper-7
- **UnrealReZen** (rm-NoobInCoding/UnrealReZen) -- alternative
  Zen packer if `retoc to-zen` does not work for the target game.
  https://github.com/rm-NoobInCoding/UnrealReZen
- **mod.io UGC Best Practices** (Unreal-specific) -- Epic's
  ecosystem partner; useful for understanding what cooked-asset
  patching is officially expected to look like.
  https://docs.mod.io/unreal/ugc-best-practices

### Grounded-2-specific reverse engineering

- **Grounded2Minimal** (x0reaxeax/Grounded2Minimal) -- DLL-based
  cheat/mod/debug tool. Likely contains class names, struct
  layouts, and offset tables relevant to the inventory component.
  Source available in repo.
  https://github.com/x0reaxeax/Grounded2Minimal
- **Grounded Wiki** (Storage & Utilities entry) -- canonical for
  in-game crate/chest capacity values, but does NOT cover player
  backpack mechanics in depth.
  https://grounded.wiki.gg/wiki/Storage_%26_Utilities_(Grounded_2)

### Information NOT available

- No official Obsidian modding SDK or API documentation.
- No `.usmap` mappings file shipped with the game; must be
  generated via Dumper-7 or sourced from the modding community.
- No public C++ headers or Unreal Engine project file from
  Obsidian for Grounded 2.
- The Grounded Wiki does not cover player backpack mechanics in
  modder-relevant detail; player-side numbers are scattered
  across forum threads and mod descriptions.

## Building our own backpack mod -- requirements

Bigger Backpack (the only existing backpack mod for Grounded 2) is
broken in the current build and uses a strategy (`MaxRows` on a UMG
widget) that the game no longer respects. Rather than try to fix
their approach, we author a fresh mod against the current build
using only the structural information we have on disk.

### Functional requirements

1. **More usable inventory slots** for the player's backpack.
   "Usable" means items can be placed into them, persist across
   save/load, and survive combat/ragdoll/death-respawn.
2. **Visual rendering matches storage.** If we increase storage to
   N slots, all N must render in the inventory UI -- no invisible
   slots, no truncated rows.
3. **Stable across game patches** as long as the underlying
   inventory component class signature does not change. (We accept
   that a major engine/inventory rewrite would break the mod;
   minor patches should not.)
4. **Coexists with All-in-One Player Tweaks** at load priority 12.
   Our mod must use a different load priority and must not
   override the same asset (`BP_SurvivalPlayerCharacter`).
5. **Reversible**: uninstalling the mod must not corrupt saves.
   This is the standard property of UE asset overrides -- but
   becomes a risk if we increase storage and the player fills the
   extra slots before uninstalling.
6. **Single-pak deployment**: one `.pak` + `.ucas` + `.utoc` set,
   no installer, drop into Vortex like any other mod.

### Non-goals

- Not increasing mount/buggy saddlebag capacity (the AIO mod's
  `UnlockBuggyUpgrade BuggyInventorySize` already covers that).
- Not increasing stack sizes (AIO's
  `UnlockItemStackUpgrade StackSize.{Ammo,Food,Resource}` already
  covers that).
- Not adding new equipment slots, hotbar slots, or quickbar slots.
- Not modifying chest/crate container capacity.

### Constraints (verified)

- Game ships UE 5.4+ format, unencrypted, single base pak. Standard
  IoStore. No tooling barriers.
- Asset overrides are by chunk-ID hash of the package name; modding
  resolves at the file level without further effort.
- Two relevant property bindings observed in the existing widgets:
  `MountInventoryComponent` (the inventory component class the
  player widget binds to -- shared between mounts and the player)
  and `MaxRows` (a widget-side hint that the current build appears
  to ignore).
- The string `MountInventoryComponent` appearing inside the
  player's container widgets is the major clue: the player's
  backpack and the mount's saddlebag use **the same inventory
  component class** (`UMountInventoryComponent` or similar).
  Therefore, the property that controls capacity likely lives on
  that component or on a per-instance default that the player
  Blueprint sets.

### Open research questions (must answer before authoring)

These answers are obtainable from the vanilla pak we already have
on disk -- no further tooling needed beyond `retoc list --path`,
`retoc to-legacy` for targeted assets, and `repak unpack` plus the
string-dump script `C:\Tools\work\dump_strings.py`.

1. **Where is the player's backpack capacity set?**
   - Is it a property default on `BP_SurvivalPlayerCharacter`'s
     inventory subcomponent?
   - Is it on the `UMountInventoryComponent` C++ class default
     (which we cannot mod) or on a Blueprint subclass like
     `BP_PlayerInventoryComponent` (which we can)?
   - Is it driven by a DataTable / DataAsset (e.g.
     `DT_PlayerInventoryConfig`)?
   - Is it driven by a player-upgrade gameplay tag the way mount
     capacity is driven by `BuggyInventorySize`?

2. **What is the property called?** Candidates suggested by the
   widget bindings: `MaxRows`, `NumRows`, `MaxItems`, `Capacity`,
   `SlotCount`, `MaxInventorySize`. Need to find the actual name.

3. **Is the upgrade-tag system involved?**
   - Is there a `PlayerBackpackSize` or `BackpackCapacity` tag
     analogous to `BuggyInventorySize`?
   - If yes, does AIO Player Tweaks already unlock it? (Looking at
     AIO's tag list -- `UnlockPlayerUpgrade Health|healing|perks|stamina|thirst`
     -- there is no backpack/inventory entry, so AIO does NOT
     unlock player backpack capacity even if a tag exists.)
   - If a tag exists and unlocks levels, what level value does
     vanilla cap at? Can we add a tag-unlock that grants a
     higher-than-vanilla level?

4. **Are there hard caps elsewhere?** Even if we find the property,
   the game might cap requested capacity at a hardcoded ceiling
   in C++. We can't verify that statically -- only by testing.

### Approach options (to be picked after research)

| Option | Strategy                                                        | Pros                                | Cons                                       |
|--------|-----------------------------------------------------------------|-------------------------------------|--------------------------------------------|
| A      | Override the player's inventory component default property     | Direct, version-stable              | Only works if the property is Blueprint-side, not C++ |
| B      | Override a DataTable/DataAsset that defines per-character caps | Survives Blueprint structure changes | Only works if such a data asset exists    |
| C      | Add a gameplay-tag unlock that pushes player upgrade level     | Same mechanism AIO uses (proven)    | Only works if a player-side capacity tag exists |
| D      | Override `BP_SurvivalPlayerCharacter` (replaces AIO)           | Maximum control                     | Conflicts with AIO at the same asset path  |
| E      | Hybrid: data-side capacity bump + widget MaxRows raise         | Belt-and-braces                     | More files to maintain, potential conflict surface |

We will not pick an option until research questions 1-3 are
answered. Tentative ranking based on what we already know: C > A
> B > D > E. C is most consistent with the proven mechanism
(gameplay-tag commands) and avoids overriding any vanilla file at
all -- maximum forward compatibility.

### Acceptance criteria

The mod is considered "working" when, with the mod installed and
the game launched against
`++Augusta+release-0.4.0.2-CL-2673661`:

- The player's backpack UI renders strictly more slots than vanilla.
- Items can be placed into the new slots and survive a full
  save -> quit -> reload cycle.
- AIO Player Tweaks remains functional (cheats still applied,
  toasts still shown).
- No load-time errors in the game's log.
- Uninstalling the mod (with empty extra slots) does not corrupt
  the save.

### Research execution plan

Each step uses tools already installed at `C:\Tools\`:

```bash
# Q1, Q2: find inventory capacity properties in vanilla.
/c/Tools/retoc/retoc.exe list --path \
  /c/Games/Steam/steamapps/common/Grounded2/Augusta/Content/Paks/Augusta-WinGRTS.utoc \
  > /c/Tools/work/vanilla_paths.txt

grep -iE 'inventory|backpack|capacity' /c/Tools/work/vanilla_paths.txt | head -40
grep -iE '/Game/.*Player.*Inventory|InventoryComponent' /c/Tools/work/vanilla_paths.txt
grep -iE 'DT_.*Inventory|DA_.*Inventory|/Upgrade' /c/Tools/work/vanilla_paths.txt

# Q3: find upgrade tag definitions.
grep -iE 'PlayerUpgrade|UpgradeTag|GameplayTag.*Inventory' /c/Tools/work/vanilla_paths.txt

# Then targeted extraction of the most promising assets via
# `retoc get <chunk_id>` (single-chunk extraction) and
# string-dump for property names.
```

Q4 (hard caps) is non-static and parked until in-game test.

## SDK research findings (definitive technical path)

This section captures findings from analysing the Dumper-7 SDK
generated against Grounded 2 v0.4.0.2 (matches our installed game
exactly) -- effectively a complete class-and-property dump of the
entire game's UE reflection data, including every Blueprint and
C++ class signature.

Cross-checked against the older `sdk-0.4.0.1` release: the
inventory class (`UInventoryComponent`), the property
(`DefaultMaxSize` at offset `0x01E0`), and the player's three
inventory components on `ASurvivalCharacter`
(`0x13B8`/`0x13C0`/`0x13C8`) are bit-identical between 0.4.0.1
and 0.4.0.2. Patch was content/balance, not engine-level
restructuring.

### How we got the SDK (no game modification required)

```bash
mkdir -p /c/Tools/work/sdk
gh release download sdk \
  --repo x0reaxeax/Grounded2Minimal \
  --pattern '*.zip' \
  --dir /c/Tools/work/sdk
cd /c/Tools/work/sdk && unzip -q -o Grounded2Minimal-SDK.zip
find /c/Tools/work/sdk/SDK -name '*.hpp' | wc -l
# 3427 header files generated by Dumper-7 against game version 0.4.0.1
```

This SDK release was generated by the `x0reaxeax` modding community
using **Dumper-7** (`Encryqed/Dumper-7`) attached to a running
Grounded 2 process. Every UE reflection-visible class and property
appears here as a C++ header with byte offsets.

### The property that controls player backpack capacity

```cpp
// Class Maine.InventoryComponent
class UInventoryComponent final : public UActorComponent
{
public:
    int32  DefaultMaxSize;   // 0x01E0 (Edit, BlueprintVisible,
                             //         BlueprintReadOnly,
                             //         IsPlainOldData,
                             //         NativeAccessSpecifierProtected)
    int32 GetMaxSize() const;
    // ... plus many other members
};
```

- **Module:** `Maine` (Grounded 2's game module name)
- **Class:** `UInventoryComponent`
- **Property:** `DefaultMaxSize` at offset `0x01E0`
- **Type:** `int32`
- **UPROPERTY flags:** `Edit, BlueprintVisible, BlueprintReadOnly`
  -- editable in the Blueprint editor (so it CAN be overridden via
  CDO patching), readable from Blueprint at runtime, but the
  runtime cannot change it directly. Only CDO defaults can.

### How the player owns the inventory components

The player's three inventory components are inherited from
`ASurvivalCharacter` (the C++ parent of `ABP_SurvivalPlayerCharacter_C`):

```cpp
// On Maine.SurvivalCharacter (offsets ~0x13B8 .. 0x13C8)
class UInventoryComponent*           InventoryComponent;          // 0x13B8 -- main backpack
class UInventoryComponent*           MountInventoryComponent;     // 0x13C0 -- mount saddlebag
class UProximityInventoryComponent*  ProximityInventoryComponent; // 0x13C8 -- nearby storage
```

The player has THREE inventory components, all instanced
sub-objects. The main backpack is `InventoryComponent` (NOT
`MountInventoryComponent`). To increase player backpack capacity,
override `DefaultMaxSize` on the **main** `InventoryComponent`
sub-object's CDO defaults inside `BP_SurvivalPlayerCharacter`.

`MountInventoryComponent` is what gets used when the player is
riding a Buggy mount -- so `UnlockBuggyUpgrade BuggyInventorySize`
(which AIO Player Tweaks runs) presumably resizes that one's
capacity, not the main backpack.

### `BP_Backpack_Player` is just the wearable item, not capacity

```cpp
// BlueprintGeneratedClass BP_Backpack_Player.BP_Backpack_Player_C
// Inherits from ABP_DeathLootBag_Base_C (a death-loot container)
class ABP_Backpack_Player_C : public ABP_DeathLootBag_Base_C
{
public:
    class UDropletContentsComponent* DropletContents;     // visual flair
    int32                            CurrentPlayerIndex_0;
};
```

The visible backpack the player wears is just a wearable mesh +
drop-on-death loot container. Capacity is NOT here. Don't waste
time editing this asset.

### Cheat-manager unlock functions

The cheat manager exposes exactly three unlock paths. AIO Player
Tweaks invokes all three:

```cpp
void UnlockBuggyUpgrade(class FName UpgradeName);
void UnlockItemStackUpgrade(const struct FGameplayTag& ItemStackTag);
void UnlockPlayerUpgrade(class FName UpgradeName);
```

Comparing each function to AIO's actual command list:

| Function                  | AIO arguments                                                    | Notes |
|---------------------------|------------------------------------------------------------------|-------|
| `UnlockBuggyUpgrade`      | `BuggyHealing`, `BuggyHealthPoolOnRevive`, `BuggyInventorySize`, `BuggyMaxHealth`, `BuggyMaxStamina` | Mount-side only |
| `UnlockItemStackUpgrade`  | `StackSize.Ammo`, `StackSize.Food`, `StackSize.Resource`         | Per-item-category stack bumps |
| `UnlockPlayerUpgrade`     | `Health`, `healing`, `perks`, `stamina`, `thirst`                | Player-side; **NO inventory/backpack entry** |

### Critical negative finding: there is no player backpack upgrade

`Table_PlayerUpgrades.uasset` (chunk
`5002054ff403aa6600000001`) contains only 5 UI-icon references:
`Health`, `Mutations`, `Stamina`, `Healing`, `Rates`. None is
inventory/backpack related. So:

- The player **does not** have a native upgrade-driven backpack
  capacity expansion mechanism.
- `UnlockPlayerUpgrade BackpackSize` would not work -- there is no
  such row.
- The cheat-command channel that AIO uses cannot grow the player
  backpack. The only option is direct CDO override on
  `BP_SurvivalPlayerCharacter`'s main `InventoryComponent`
  sub-object.

This is also why the "60-slot Player Tweaks variant" must be a
CDO-override mod (not a cheat-command mod) -- it has no choice.
It directly bumps `DefaultMaxSize` on the player's main inventory
component.

### Did AIO v13.1.6 set DefaultMaxSize?

Strong but not definitive evidence: yes.

- `DefaultMaxSize` appears in the AIO mod's FName table at byte
  offset 22460 of the `.uasset`. UE's cooked serialiser usually
  emits FName entries for properties that the asset overrides
  relative to its parent (delta serialisation). A property name
  appearing in the FName table is consistent with the override
  setting it.
- Reading the actual int32 value requires a proper UAsset
  deserialiser (UAssetGUI + Grounded 2 mappings). Cannot be done
  via grep alone.
- If AIO v13.1.6 sets `DefaultMaxSize: 40` (vanilla) it's a no-op
  override; if it sets it to e.g. 60, then this is in fact a
  60-slot Player Tweaks build and the user's "Bigger Backpack
  doesn't work" report has a different cause.

**Recommended next step to confirm:** open
`/c/Tools/work/mod_unpacked/BP_SurvivalPlayerCharacter.uasset`
in UAssetGUI with a Grounded 2 mappings file loaded. Find the
`DefaultMaxSize` property on the main `InventoryComponent`
subobject. Read the value.

### How to author the DIY mod (definitive)

Now that we know the exact property, class, and offset:

1. **Acquire a Grounded 2 mappings file (`.usmap`).** Two paths:
   - Generate via Dumper-7 with `--mappings` flag against the
     running game.
   - Convert the SDK we already have on disk into a `.usmap` via
     a script (the SDK contains every property name + type
     needed; community tools like `UnrealMappingsDumper` can
     produce a usmap from Dumper-7 output).
2. **Open `BP_SurvivalPlayerCharacter.uasset`** in UAssetGUI with
   the mappings file loaded. Find the `InventoryComponent`
   instanced sub-object, then the `DefaultMaxSize` property.
3. **Set the value** to the desired slot count (e.g. 60, 80, 100).
4. **Save** the modified `.uasset` and `.uexp` together.
5. **Repack as a legacy pak** with repak:
   ```bash
   /c/Tools/repak/repak.exe pack ./our_mod_files ./our_mod_legacy.pak
   ```
6. **Convert to Zen IoStore** for the actual game-mountable mod:
   ```bash
   /c/Tools/retoc/retoc.exe to-zen ./our_mod_legacy.pak \
                                   ./BackpackOnly_00099_P.utoc \
                                   --version UE5_4
   ```
7. **Drop the resulting `.pak`/`.ucas`/`.utoc` triple** into
   Vortex or directly into
   `C:\Games\Steam\steamapps\common\Grounded2\Augusta\Content\Paks\`.
8. **Verify in-game.** Pair with Bigger Backpack
   (`ContainerWidgetTweaks_00054_P`) to also widen the rendered
   slots.

### Why this approach is robust

- It targets a **named property** (`DefaultMaxSize`) on a
  **named class** (`UInventoryComponent`) that is part of the
  game's stable C++ API surface. As long as Obsidian doesn't
  rename or restructure the inventory component, the override
  continues to apply.
- It uses the **chunk-ID hash** (= hash of package name) for
  override resolution, which is stable across UE 5.x versions.
- It is **load-priority-independent** of AIO Player Tweaks
  except that both touch `BP_SurvivalPlayerCharacter` -- so
  exactly one Player Tweaks variant can be installed at a
  time. That's the same constraint the existing 60-slot Player
  Tweaks variant has.

## Caveats

- Grounded 2 ships UE 5.4+ (TOC version `ReplaceIoChunkHashWithIoHash`,
  container header `SoftPackageReferencesOffset`). Set FModel's parser
  to UE 5.4 -- using the wrong version yields unreadable property
  blocks.
- Grounded 2 ships **unencrypted** containers (verified 2026-05-04 via
  `retoc info`: `container_flags: 0x0` on global,
  `container_flags: Indexed` on the mod, no Encrypted flag). No AES
  key handling needed in FModel.
- This document covers inspection only. Editing values and repacking
  back into a working mod is a separate workflow (UAssetGUI for the
  edit, retoc `to-zen` for the repack).
