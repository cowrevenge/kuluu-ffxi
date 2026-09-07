# DLSS — How to Use

Part of the DLSS doc set: [How to Use](How_to_Use.md) · [History](History.md) · [Architecture](Architecture.md). Forensic dumps: [`logs/`](logs/).

**Current state (2026-09):** the default local build is **non-DLSS**. The
`dlss` feature is opt-in and deliberately off until the NVIDIA whitepaper /
SDK material lands — `build_cowland.bat` builds without it on purpose. All
menu rows still exist in every build; they read `N/A` while the feature is not
compiled in or unsupported. How to turn DLSS back on: the "Building with
DLSS" section below (machine-specific commands for this checkout) plus
README §"Optional DLSS builds" (machine-independent).

---

## What you get

- **Super Resolution (SR)** — upscaling + anti-aliasing in one pass, driven
  from the same graphics menus as every other setting. Requires an RTX GPU on
  the **Vulkan** backend (Windows or Linux). Never available on wasm/macOS/
  browser viewer.
- **Neural Uplift (NR / "DLSSNR")** — NVIDIA DLSS 5 Neural Rendering, driven
  by calling `nvngx_dlssnr.dll`'s Vulkan NGX API directly from Rust (no
  ReShade, no D3D12 detours). Experimental, Windows-only; additionally needs
  the NR runtime DLL + the project's forwarder staged next to the exe.

## Menus — current state

Two surfaces share one underlying state (`anti_aliasing` + `dlss_quality` in
graphics.json):

- **Anti-Aliasing cycler** (main graphics list): plain slots only —
  Off / MSAA2 / MSAA4 / MSAA8. DLSS is *not* a slot here; it was removed from
  the cycler and lives solely on its own row below.
- **DLSS row** (right under Anti-Aliasing): an explicit On/Off mirror of
  `anti_aliasing == AaMode::Dlss`. Reads `N/A` and refuses to toggle while
  unsupported. Turning it off lands on AA `Off`; re-pick MSAA/TAA in the
  cycler if you want them back.
- **DLSS Config** — in-game: the `DLSS Config` row near the bottom of the
  Graphics menu (opens a submenu); launcher: the `> DLSS configuration`
  disclosure. Contents:
  - `DLSS Quality` — live knob: Auto / DLAA / Quality / Balanced / Performance
    / Ultra Perf. Auto lets DLSS pick from the output resolution; DLAA is
    anti-aliasing only at native res.
  - `Neural Uplift` — master toggle for the NR pipeline (`graphics/dlss_nr.rs`).
    Live on dlss builds with an RTX GPU + `nvngx_dlssnr.dll` staged next to
    the exe; `N/A` otherwise. NR only evaluates while the AA mode is Dlss:
    cycling to MSAA/TAA stands it down entirely (the toggle persists as a
    setting, but nothing runs — see `GraphicsSettings::nr_active`).
  - `NR Intensity`, `Local Tone Strength`, `Structure Strength` — the addon's
    three knobs; live while supported.
  - `RR Preset`, `SR Preset`, `RR Responsivity`, `Sharpness` — inert
    placeholders, always `N/A` (see "Placeholders" below).

Quality presets (Low/Medium/High/Ultra) never own DLSS: no preset turns it on,
and picking or cycling a preset does not turn it off or touch the tier.
`Reset to High` does turn it off (it is a full reset) but never un-detects
support.

## Behavior while active

- MSAA and TAA are forced off (the AA respawn owns this).
- The manual Render Scale row parks: it reads `DLSS` and refuses to cycle, and
  the off-screen composite path stands down — DLSS owns internal resolution
  and upscaling. Both come back the moment DLSS is off.
- Changing the quality tier respawns the operator camera (a blink). Deliberate:
  a fresh view entity guarantees the DLSS context re-creates at the new
  internal resolution. In-place tier mutation is a possible later optimization
  once a dlss build can be A/B tested on real hardware.
- State: on/off rides `anti_aliasing`, the tier rides `dlss_quality` (plus
  `neural_uplift` / `nr_*` fields) in graphics.json. Capability
  (`dlss_supported`) is runtime-detected every launch and never persisted, so a
  config written on an RTX box is a harmless no-op elsewhere — the DLSS row
  just reads `N/A` until you cycle away.

## Building with DLSS

The `dlss` feature is **opt-in** (`kuluu/Cargo.toml`:
`dlss = ["native-window","kuluu-render/dlss","bevy/dlss"]`). The default build
graph contains no DLSS; SDK-less environments (CI runners, release legs, Steam
Deck docker) build with `--no-default-features --features native-window`, which
keeps bevy/dlss out of the link graph entirely and leaves every row permanently
`N/A`.

Build-time requirements (all from dlss_wgpu 4.0.0, which bevy's `dlss` feature
pulls in; its build.rs panics without the first two):

1. Clone the NVIDIA DLSS SDK, tag `v310.5.3`, and comply with its license:
   `git clone --branch v310.5.3 https://github.com/NVIDIA/DLSS` — this repo's
   dev checkout lives in `Dlss_Nvidia_dll/sdk`.
2. Set `DLSS_SDK` to the SDK root (dev machine: `<repo>\Dlss_Nvidia_dll\sdk`).
3. Install the Vulkan SDK and set `VULKAN_SDK` (this repo's dev checkout:
   `Dlss_Nvidia_dll/vulkan-sdk`).
4. Install clang — bindgen needs libclang (`LIBCLANG_PATH=<repo>\Dlss_Nvidia_dll\llvm\bin`).

All NVIDIA-supplied material lives in [`Dlss_Nvidia_dll/`](../Dlss_Nvidia_dll/README.md)
(git-ignored except its README — supply your own copies).

Local one-shot: `.\build_cowland.bat` is a **non-DLSS** build — it builds kuluu
with the local feature set and syncs the exe to the repo root via
`logs\sync_exe.ps1`, nothing more. It sets no SDK vars, builds no forwarder,
and stages no DLLs (whitepaper hold). Re-enabling DLSS is manual: add `dlss`
to the cargo feature line, set the three env vars below, build the forwarder
(`kuluu-ngx-fwd`), and stage the runtime DLLs. Manual equivalent
(build only):

```bat
set "DLSS_SDK=C:\Cow_Kuluu_ffxi-engine\Dlss_Nvidia_dll\sdk"
set "VULKAN_SDK=C:\Cow_Kuluu_ffxi-engine\Dlss_Nvidia_dll\vulkan-sdk"
set "LIBCLANG_PATH=C:\Cow_Kuluu_ffxi-engine\Dlss_Nvidia_dll\llvm\bin"
cargo build --release -p kuluu --features debug-menu,enhanced-mob-hp-under,enhanced-job-display,dlss
```

`scripts/checks.sh` auto-sets the three vars from `Dlss_Nvidia_dll/` when
unset, so gate runs work without user env vars.

## Running / distributing

You do not ship the SDK. Next to the built binary, place:

| File | Source in this checkout | Needed for |
|---|---|---|
| `nvngx_dlss.dll` | `Dlss_Nvidia_dll\nvngx_dlss.dll` (from the SDK: `lib/Windows_x86_64/rel/nvngx_dlss.dll`; Linux: `$DLSS_SDK/lib/Linux_x86_64/rel/libnvidia-ngx-dlss.so.310.5.3`) | DLSS SR (always) |
| `nvngx_dlssnr.dll` (~166 MB, v310.8.0) | `Dlss_Nvidia_dll\nvngx_dlssnr.dll` | Neural Uplift only |
| `nvngx.dll_kuluu.dll` | built as `kuluu_ngx_fwd.dll` by `cargo build -p kuluu-ngx-fwd`, then copied under this name next to the exe | Neural Uplift only |

plus the copyright/license text from section 9.5 of the SDK's programming guide
if distributing. The forwarder's staged name **must contain `nvngx.dll`** — it
is deliberately distinct from the driver's own `nvngx.dll` (see [Architecture](Architecture.md), "The
'nvngx.dll' calling-module gate"). If any DLL is missing, or the GPU/backend
can't do DLSS, nothing breaks: the renderer just never reports support and the
menu rows stay `N/A`.

Expect some Vulkan validation errors with DLSS active; per dlss_wgpu these come
from a bug in DLSS itself and are safe to ignore.

## Known limitations

- **Nameplate wall-occlusion under DLSS.** The nameplate pass draws into the
  full-res post-upscale image, but the scene depth buffer only holds valid
  geometry in its render-res sub-rectangle (bevy sizes the texture from
  physical_target_size; the main pass writes a top-left viewport). A hardware
  attachment test against that buffer occludes plates against stale texels, so
  under any upscaler the pass runs a `Subrect` depth mode instead: it binds the
  single-sample scene depth as a texture and does one nearest load per fragment
  at `fragment_coord * (render_res / target_size)` — every fragment lands inside
  the sub-rect where valid geometry lives, so walls still occlude plates
  post-upscale. Plates are drawn AFTER the upscaler in all modes, so they are
  never scaled or temporally filtered by SR/NR.
- **Quality-tier changes blink** (camera respawn, see above).
- **HDR pipeline note**: the operator camera is already Hdr, which DLSS
  requires; nothing to do here — just don't remove it.
- **Neural Uplift: zero-MVec stand-in.** Bevy produces no motion-vector texture
  for our camera, so NR evaluates against a zero-filled Rg16Float stand-in — an
  explicit "no camera motion". NVIDIA confirmed ([Architecture](Architecture.md), §"Official DLSS 5 knowledge") that the model receives
  exactly two runtime inputs: the frame and the motion vectors. Expect
  shimmer/ghosting specifically during camera movement; fix path = bevy's real
  `MotionVectorPrepass` when SR is active, then drop the stand-in.

## Placeholders (inert rows)

The DLSS Config surface intentionally shows more rows than are wired, so the
menu structure matches where this is going (the RenoDX Control add-on is the
reference UX). They are inert on every build and read `N/A`
(`GraphicsField::is_dlss_placeholder()`):

- `RR Preset` / `RR Responsivity`: Ray Reconstruction. bevy_anti_alias 0.19
  exposes SR only; RR types exist in dlss_wgpu but there is no bevy plumbing.
- `SR Preset` (the J/K/L/M model presets): not surfaced by dlss_wgpu 4.0.
- `Sharpness`: wireable today via bevy's ContrastAdaptiveSharpening; left inert
  with the rest for now, and the obvious first placeholder to bring to life.

## Feature-gating map (what compiles when)

Unconditional (every build): `AaMode::Dlss`, `DlssQuality`, the
`dlss_quality`/`dlss_supported` fields, every menu row and label, the `Subrect`
nameplate depth mode (keyed on MainPassResolutionOverride presence, not on DLSS
types). `dlss_supported` can only ever become true when the feature is compiled
in, so all of it is dead-quiet on SDK-less builds.

`#[cfg(feature = "dlss")]` only: `kuluu-render/src/graphics/dlss.rs` (capability
probe, tier mapping, project id), `kuluu-render/src/graphics/dlss_nr.rs` (the NR
pipeline — gated at runtime by `GraphicsSettings::nr_active`, which requires the
AA mode to be Dlss as well as support) and its `kuluu-dlss-nr` FFI crate, the
`Dlss` component insert in `camera.rs`, the availability system registration in
`kuluu-render/src/lib.rs`, and the `DlssProjectId` resource insert in
`kuluu/src/view_native/mod.rs` (`DlssInitPlugin` itself is added by Bevy's
DefaultPlugins under the feature).

The DLSS project id (`KULUU_DLSS_PROJECT_ID` in graphics/dlss.rs) is fixed for
the lifetime of the project; NVIDIA's driver keys per-app behavior on it, so
**never regenerate it**.
