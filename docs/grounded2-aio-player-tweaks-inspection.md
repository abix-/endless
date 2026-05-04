# Grounded 2 -- Mod Inspection (Worked Examples)

How to fully decompile and understand Grounded 2 (Unreal Engine 5) mods
so that you can list every change a mod makes, identify the exact assets
it touches, and diagnose why a previously-working mod is now broken.

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

**Conclusion:** the entire All-in-One Player Tweaks v13.1.6 mod is a
single overridden Blueprint, `BP_SurvivalPlayerCharacter`, which is
the player character class. All gameplay tweaks (movement, stamina,
hunger/thirst, damage, etc.) are encoded as property defaults on the
CDO of this Blueprint, plus any patched EventGraph/Function bytecode
inside the `.uexp` payload.

To enumerate exact property values, the next step is to decode the
`.uexp` -- either via FModel GUI on `mod_legacy.pak`, or via
UAssetGUI for byte-level property tables.

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

### Hypothesis -- where the bug actually is

Since the override resolves but the mod doesn't take effect, the bug
must be **inside the widget content itself**, not in container
plumbing. Three plausible failure modes:

1. **Stale parent class.** The mod was packaged Jan 28 against a
   prior build's `UContainerWidget` C++ parent. If the parent class
   added/removed virtual functions or properties, the modded
   widget's serialised property block no longer matches the new
   parent and either fails to deserialise (silent), or deserialises
   into a partial/zeroed state.
2. **Stale child-widget references.** UMG widgets reference child
   sub-widgets by FName + path. If the modder hand-edited the child
   layout to add slots, but the underlying inventory grid is now
   driven by a different child container class, the modded layout
   loads but the slot count comes from a code-side query that
   ignores the widget hierarchy.
3. **Capacity is data-driven, not widget-driven.** Most likely
   explanation: backpack size is stored in a DataAsset/struct on
   the player or item-component side, not in the widget. The
   widget renders whatever count the data side gives it. A
   "widget-only" mod can paint extra slots in the layout, but the
   game's inventory component caps usable slots at the data-side
   value, so the extras render empty or the patched layout gets
   re-laid-out at runtime back to vanilla dimensions.

The third hypothesis fits the "mod stops working entirely" symptom
better than the first two -- a partial layout failure usually
shows visual artefacts, while "no effect at all" suggests the data
side is overriding what the widget tries to display.

### Next step to confirm

Decode `UI_Container_BackpackSide.uexp` and compare grid-dimension
properties (`NumSlotsX`, `NumSlotsY`, `MaxItems`, or whatever the
widget calls them) against vanilla. If they DO differ, the mod's
intent is widget-side and the broken behaviour is data-side
clamping. If they DO NOT differ, the mod must be modifying an
EventGraph hook that has changed signature in the new build.

Verification commands (next session):

```bash
# Use FModel on bb_legacy.pak with parser set to UE 5.4.
# Right-click each widget -> Export Properties (.json).
# Compare against vanilla widget JSON dumps.
#
# Or use UAssetGUI CLI:
#   UAssetGUI.exe tojson UI_Container_BackpackSide.uasset out.json --version VER_UE5_4
```

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
