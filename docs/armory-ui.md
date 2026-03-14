# Armory UI

Replace the current left-panel armory tab with a centered modal window that feels intentional and game-like, while preserving the existing equipment logic and town inventory behavior.

## Goal

The armory should stop reading like a debug panel. It should open as a dedicated window, show the selected unit and its gear as visual slots, and let the player manage town equipment without losing any of the current equip, unequip, auto-equip, or filtering behavior.

The blackjack window in `rust/src/ui/blackjack.rs` is the layout reference for window treatment only: centered modal, custom frame, clear hierarchy, and visible state changes. The armory should use its own military/equipment visual theme rather than copying the casino palette.

## Current Behavior

The current armory lives in `rust/src/ui/left_panel/inventory_ui.rs` and is opened through `LeftPanelTab::Inventory`.

Existing functional behavior that must be preserved:

- manual equip through `EquipItemMsg`
- manual unequip through `UnequipItemMsg`
- town-wide and selected-unit auto-equip through `AutoEquipNowMsg`
- inventory filtering by `ItemKind`
- view modes for unequipped, equipped, and all items
- rarity coloring
- stat comparison tooltip when a unit is selected
- "Sell All Common" cleanup action

The redesign should change presentation and window flow, not replace the underlying equipment systems.

## Scope Decisions

Initial delivery for this issue includes:

- a centered armory modal window
- a roster rail for quick switching between equippable NPCs in the current town
- visual equipment slots for the selected NPC
- town inventory presentation that keeps click-to-equip and hover details
- existing bulk auto-equip actions

Initial delivery does not require:

- drag-and-drop
- animation-heavy transitions
- new equipment rules or stat systems
- item crafting, vendors, or loadout presets

Click-to-equip is the required interaction path for the first pass. Drag-and-drop can be added later if the window already feels good without it.

## Window Model

Add a dedicated armory window state in `UiState`, separate from `left_panel_open`.

Opening sources:

- `ControlAction::ToggleInventory`
- the HUD armory button
- the inspector's "Open Armory >" button

Behavior:

- opening the armory opens a centered modal instead of selecting `LeftPanelTab::Inventory`
- opening the armory should close the old inventory tab if it is currently open
- `Esc` should close the armory before the left panel, matching the existing floating-window close order used by blackjack and the tech tree
- the window should be non-collapsible and non-resizable for the first pass

## Visual Direction

The armory should feel like equipment storage, not paperwork.

Use a distinct palette from blackjack:

- dark steel or charcoal base
- olive or desaturated green accents
- brass, leather, or canvas highlights for emphasis
- stronger selected-state highlight than the current left-panel tab uses

The frame should be custom, with visible border and spacing, but still use standard egui primitives so the implementation stays maintainable.

## Layout

Use a three-column layout inside the modal.

### Left: Roster Rail

Show all equippable NPCs for the active town.

- each row shows name, job, and a compact readiness summary
- the selected NPC gets a clear highlight treatment
- clicking a row updates `SelectedNpc`
- if the current `SelectedNpc` is not equippable or belongs to another town, the armory falls back to the first equippable NPC in town 0

### Center: Equipment Board

Show the selected NPC's gear as visual slot cards instead of raw label rows.

- all current equipment slots remain supported: helm, armor, weapon, shield, gloves, boots, belt, amulet, ring 1, ring 2
- weapon and armor must be visually distinct at minimum to satisfy issue acceptance
- filled slots show item name, rarity color, and stat bonus
- empty slots show a muted placeholder state
- clicking a filled slot unequips that item

The center panel is the main identity piece for the redesign. It should feel like inspecting a kit layout, not reading a table.

### Right: Town Inventory

Show town inventory as item cards or rows with stronger visual treatment than the current text list.

- preserve the existing view modes: unequipped, equipped, all
- preserve the slot filter controls
- keep rarity color visible at rest
- on hover, show item stats and the comparison against the selected NPC's current item for that slot
- clicking an unequipped inventory item equips it to the selected NPC
- equipped items shown in the "equipped" or "all" views must still support unequip from this panel

Keep the current "Sell All Common" action available in this panel, but style it as a secondary maintenance action rather than the main call to action.

## Interaction Rules

Selection:

- `SelectedNpc` remains the authoritative selected unit
- armory roster clicks update `SelectedNpc`
- opening the armory from the inspector should preserve the currently selected NPC when possible

Equip:

- equipping still sends `EquipItemMsg`
- the current ring behavior stays unchanged: a ring fills `ring1` first, then `ring2`

Unequip:

- unequipping from a slot or equipped-item list still sends `UnequipItemMsg`
- ring slot actions must preserve the existing `ring_index` contract

Bulk actions:

- keep "Auto-equip Town Now"
- keep "Auto-equip Selected"
- do not add new mass-equip logic in this issue unless it falls out naturally from the refactor

## Data and System Boundaries

Reuse the current data flow wherever possible.

- keep using `InventoryParams`
- keep using `SelectedNpc`
- keep `NpcEquipment` as the source of equipped items
- keep `TownAccess` as the source for town inventory and gold
- keep `inv_view_mode` and `inv_slot_filter` unless there is a compelling cleanup during implementation

Recommended code shape:

- add a dedicated armory window system under `rust/src/ui/`
- move reusable inventory helpers out of `left_panel/inventory_ui.rs` as needed
- keep the equip, unequip, and auto-equip message contracts unchanged

## Implementation Order

Implement in this order so each step is testable and low-risk:

1. Add `UiState` armory-open state and route the existing armory open actions to a centered window.
2. Move selected-NPC presentation into a roster rail plus equipment board.
3. Move town inventory, filters, equipped/unequipped views, and bulk actions into the modal.
4. Polish hover states, selected states, spacing, and framing.

Do not start with drag-and-drop. Get the modal, roster, slots, and click-to-equip path correct first.

## Acceptance Mapping

This spec satisfies the issue acceptance by requiring:

- a centered armory window instead of the side panel
- visually distinct equipment slots
- hoverable inventory presentation with rarity and stat details
- a clearly highlighted selected NPC in a roster
- preserved auto-equip and manual equip/unequip behavior
- no regression in the existing equipment backend

## Verification

Required verification for implementation turns:

- `cargo test`
- in-game visual check that `ToggleInventory` opens the centered armory window
- confirm roster selection updates the selected NPC
- confirm equip, unequip, and both auto-equip actions still work
- confirm rarity coloring and hover comparison remain visible
- confirm `Esc` closes the armory window before the left panel
