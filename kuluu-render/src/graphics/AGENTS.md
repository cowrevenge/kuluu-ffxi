# AGENTS.md — graphics settings (scoped)

`CLAUDE.md` is a symlink to this file. This note is loaded whenever you edit
anything under `kuluu-render/src/graphics/`.

This module owns the **user-facing graphics settings model**: `GraphicsSettings`
(`settings.rs`) plus the `GraphicsField` enum, the `*_SECTIONS` page lists, and
the `apply_*_system` functions that push each setting onto the live render
pipeline.

## The rule: every graphics option is user-facing

A new graphics knob is **not done** until it appears in the in-app graphics menu.
Both surfaces — the launcher (pre-login) screen in
`kuluu/src/view_native/launcher_ui/graphics.rs` and the in-game menu
(`hud::menu`) — derive their rows from the same `GraphicsSection` lists
(`GRAPHICS_SECTIONS`, `CONFIG_SECTIONS`, `DLSS_CONFIG_SECTIONS`). Placing a
`GraphicsField` in a section is the single act that surfaces it in both. Don't
add a setting that's only reachable from a slash command or an env var, and
don't add a row with nothing behind it — a control that always reads `N/A` and
cycles nothing is a promise, not an option.

## Parity is a property of each value, not of the row

Every settings enum implements `ParityValue` (`name` + `parity`), and two-state
rows declare `GraphicsField::bool_parity`. `value_label` joins them through
`parity_label`, so a value renders as `Fade (Vanilla)` / `Luminous (Enhanced)`
with no call site spelling its own suffix. A row may offer several Vanilla
values and several Enhanced ones, and `Off` is frequently the Vanilla choice
(the original client anti-aliases nothing, draws no zone shadow map, has no
bloom). `Reduced` is for dropping *below* the original client to buy
performance; `Neutral` is for host knobs it had no say over (window mode, VSync,
frame cap).

The user-facing word is **Vanilla** — the `VANILLA`/`ENHANCED`/`REDUCED`
constants. "Retail" is a development word and is pinned out of the UI by the
`ui_never_says_retail` test.

Rows with no original-client equivalent *at all* (Bloom, DoF, Camera Spring,
volumetric fog, AA, DLSS) live in the trailing `ENHANCED_SECTION` group, pinned
by `enhanced_only_rows_sit_in_the_trailing_group`. A row that merely has an
Enhanced *value* (Shading, Minimap, Texture Filtering) stays in its topical
section.

## Checklist — adding a graphics option (do every step)

1. **Field** on `GraphicsSettings` (`settings.rs`). Add `#[serde(default = "…")]`
   (or `#[serde(default)]`) so old `graphics.json` files still load — persistence
   is automatic via `Serialize`/`Deserialize` (`kuluu/src/graphics_store.rs`).
2. **Enum** variant on `GraphicsField` + arm in `GraphicsField::label()`.
   Indent the label with two leading spaces only if it's an "advanced" child knob
   (see `is_advanced`).
3. **Display** arm in `GraphicsSettings::value_label()` — a `ParityValue`
   enum's own `label()`, or `toggle_label` for an on/off row (plus its
   `bool_parity` arm). Never hand-write a `(Vanilla)`/`(Enhanced)` suffix.
4. **Cycle** arm in `GraphicsSettings::cycle()`, backed by a `const *_SLOTS` array
   (use `cycle_slot*`). If the knob is a quality lever, set
   `self.preset = QualityPreset::Custom`; if it's orthogonal to the tier (e.g.
   sky style, zone lines), leave the preset alone — and add a test pinning that.
5. **GUI** — add the variant to the section it belongs to (`GRAPHICS_SECTIONS`,
   `CONFIG_SECTIONS` or `DLSS_CONFIG_SECTIONS`). That is the only step: both
   menus derive their rows, labels and cursor mapping from it. There is no
   second label array to keep in sync.
6. **Presets** — set the field in **all** `for_preset()` arms (Minimum/Low/
   Medium/High/Ultra/Maximum; Custom inherits Low). Minimum and Maximum are the
   floor and ceiling: every quality lever sits at the bottom and top of its slot
   array respectively. `preset_values_are_slot_aligned` will fail if a preset
   value isn't in its slot array.
7. **Apply** — write an `apply_<thing>_system` (mirror `apply_bloom_system` /
   `apply_anti_aliasing_system`) and register it in `crate::ViewerCorePlugin`
   (`lib.rs`), gated `run_if(resource_changed::<GraphicsSettings>)` unless it must
   track something else every frame.
8. **Tests** — extend the `settings.rs` test module: value_label smoke, cycle
   wrap, preset-cycle preservation/reset, and JSON roundtrip.
9. *(optional)* a `/<name>` slash command in
   `kuluu/src/view_native/slash_commands.rs` for quick in-session tweaking.

Keep the launcher renderer dumb: it should never need editing to show a new
option — if it does, the abstraction (iterate `GRAPHICS_FIELDS`) has leaked.
