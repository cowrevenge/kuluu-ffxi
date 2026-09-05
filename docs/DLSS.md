# DLSS Super Resolution

NVIDIA DLSS SR support for the native viewer: upscaling + anti-aliasing in one
pass, driven from the same graphics menus as every other setting. OPT-IN via
the `dlss` kuluu feature (`cargo build -p kuluu --features dlss`): with it on,
the runtime plumbing compiles and the menu rows go live on capable hardware.
SDK-less environments (CI runners, release legs, Steam Deck docker) build with
`--no-default-features --features native-window`; there the plumbing is not
compiled in and the menu rows permanently read `N/A`.

Requires an RTX GPU on the Vulkan backend (Windows or Linux). Never available
on wasm.

## Using it

Two menu surfaces, same state underneath:

- **Anti-Aliasing cycler**: `DLSS` appears as one more slot (after TAA) while
  the runtime supports it. It is mutually exclusive with MSAA/TAA by
  construction — it IS the anti-aliasing.
- **DLSS row** (right under Anti-Aliasing): a plain On/Off mirror of the same
  state. Reads `N/A` and refuses to toggle while unsupported. Turning it off
  lands on AA `Off`; re-pick MSAA/TAA in the cycler if you want them back.
- **DLSS Config**: in-game it's the `DLSS Config` row near the bottom of the
  Graphics menu (opens a submenu); in the launcher it's the
  `> DLSS configuration` disclosure. Contents:
  - `DLSS Quality` — the live knob: Auto / DLAA / Quality / Balanced /
    Performance / Ultra Perf. Auto lets DLSS pick from the output resolution;
    DLAA is anti-aliasing only at native res.
  - `Neural Uplift` — master toggle for the NR pipeline (`graphics/dlss_nr.rs`).
    Live on dlss builds with an RTX GPU + `nvngx_dlssnr.dll` staged next to
    the exe; `N/A` otherwise. NR only evaluates while the AA mode is Dlss:
    cycling to MSAA/TAA stands it down entirely (the toggle persists as a
    setting, but nothing runs — see `GraphicsSettings::nr_active`).
  - `NR Intensity`, `NR Local Tone Strength`, `NR Structure Strength` — the
    addon's three knobs; live while supported.
  - `RR Preset`, `SR Preset`, `RR Responsivity`, `Sharpness` — inert
    placeholders, always `N/A` (see "Placeholders" below).
  - `Reset DLSS to defaults` — quality back to Auto, NR off + knob defaults.

Quality presets (Low/Medium/High/Ultra) never own DLSS: no preset turns it on,
and picking or cycling a preset does not turn it off or touch the tier. `Reset
to High` does turn it off (it is a full reset) but never un-detects support.

While DLSS is active:

- MSAA and TAA are forced off (the AA respawn owns this).
- The manual Render Scale row parks: it reads `DLSS` and refuses to cycle,
  and the off-screen composite path stands down, because DLSS owns internal
  resolution and upscaling. Both come back the moment DLSS is off.
- Changing the quality tier respawns the operator camera (a blink). That is
  deliberate: a fresh view entity is guaranteed to re-create the DLSS context
  at the new internal resolution. In-place tier mutation is a possible later
  optimization once a dlss build can be A/B tested on real hardware.

State: on/off rides `anti_aliasing` and the tier rides `dlss_quality` in
graphics.json. Capability (`dlss_supported`) is runtime-detected every launch
and never persisted, so a config written on an RTX box is a harmless no-op on
anything else — the AA row just reads `DLSS (N/A)` until you cycle away.

## Building with DLSS

Build-time requirements (all from dlss_wgpu 4.0.0, which bevy's `dlss`
feature pulls in; its build.rs panics without the first two):

1. Clone the NVIDIA DLSS SDK, tag `v310.5.3`, and comply with its license:
   `git clone --branch v310.5.3 https://github.com/NVIDIA/DLSS` — this repo's
   dev checkout lives in `streamline/sdk`.
2. Set `DLSS_SDK` to the SDK root (dev machine: `<repo>\streamline\sdk`).
3. Install the Vulkan SDK and set `VULKAN_SDK` (the LunarG installer sets it
   up; this repo's dev checkout: `streamline/vulkan-sdk`).
4. Install clang (bindgen needs libclang; this repo's dev checkout:
   `LIBCLANG_PATH=<repo>\streamline\llvm\bin`).

Set the three variables once in your user environment and every local build
passes `--features dlss` (e.g. `cargo run -p kuluu --features dlss`).
`scripts/checks.sh` auto-sets them from `streamline/` when they are unset, so
gate runs work without the user env vars.

SDK-less environments (CI runners, release legs, Steam Deck docker) build with
`--no-default-features --features native-window`; that keeps bevy/dlss out of
the link graph entirely.

## Running / distributing

You do not ship the SDK. Next to the built binary, place:

- Windows: `$DLSS_SDK/lib/Windows_x86_64/rel/nvngx_dlss.dll`
- Linux: `$DLSS_SDK/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.310.5.3`

plus the copyright/license text from section 9.5 of the SDK's programming
guide if distributing. If the DLL is missing, or the GPU/backend can't do
DLSS, nothing breaks: the renderer just never reports support and the menu
rows stay `N/A`.

Expect some Vulkan validation errors with DLSS active; per dlss_wgpu these
come from a bug in DLSS itself and are safe to ignore.

## Known limitations

- **Nameplate wall-occlusion under DLSS.** The nameplate pass draws into the
  full-res post-upscale image, but the scene depth buffer only holds valid
  geometry in its render-res sub-rectangle (bevy sizes the texture from
  physical_target_size; the main pass writes a top-left viewport). A hardware
  attachment test against that buffer occludes plates against stale texels,
  so under any upscaler the pass runs a `Subrect` depth mode instead: it binds
  the single-sample scene depth as a texture and does one nearest load per
  fragment at `fragment_coord * (render_res / target_size)` — every fragment
  lands inside the sub-rect where valid geometry lives, so walls still occlude
  plates post-upscale. Plates are drawn AFTER the upscaler in all modes, so
  they are never scaled or temporally filtered by SR/NR.
- **Quality-tier changes blink** (camera respawn, see above).
- **HDR pipeline note**: the operator camera is already Hdr, which DLSS
  requires; nothing to do here, just don't remove it.

## Placeholders

The DLSS Config surface intentionally shows more rows than are wired, so the
menu structure matches where this is going (the RenoDX Control add-on is the
reference UX). They are inert on every build and read `N/A`:

- `RR Preset` / `RR Responsivity`: Ray Reconstruction. bevy_anti_alias 0.19
  exposes SR only; RR types exist in dlss_wgpu but there is no bevy plumbing.
- `SR Preset` (the J/K/L/M model presets): not surfaced by dlss_wgpu 4.0.
- `Sharpness`: wireable today via bevy's ContrastAdaptiveSharpening; left
  inert with the rest for now, and the obvious first placeholder to bring to
  life.

## Feature-gating map (what compiles when)

Unconditional (every build): `AaMode::Dlss`, `DlssQuality`, the
`dlss_quality`/`dlss_supported` fields, every menu row and label, the
`Subrect` nameplate depth mode (keyed on MainPassResolutionOverride presence,
not on DLSS types). `dlss_supported` can only ever become true when the
feature is compiled in, so all of it is dead-quiet on SDK-less builds.

`#[cfg(feature = "dlss")]` only (opt-in: local dev/test builds pass
`--features dlss`; SDK-less environments simply don't): `kuluu-render/src/graphics/dlss.rs`
(capability probe, tier mapping, project id), `kuluu-render/src/graphics/dlss_nr.rs`
(the NR pipeline — gated at runtime by `GraphicsSettings::nr_active`, which
requires the AA mode to be Dlss as well as support) and its `kuluu-dlss-nr`
FFI crate, the `Dlss` component insert in `camera.rs`, the availability system
registration in `kuluu-render/src/lib.rs`, and the `DlssProjectId` resource
insert in `kuluu/src/view_native/mod.rs` (`DlssInitPlugin` itself is added by
Bevy's DefaultPlugins under the feature).

The DLSS project id (`KULUU_DLSS_PROJECT_ID` in graphics/dlss.rs) is fixed
for the lifetime of the project; NVIDIA's driver keys per-app behavior on it,
so never regenerate it.
