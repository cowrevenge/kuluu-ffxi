# DLSS — Architecture (how it works)

Part of the DLSS doc set: [How to Use](How_to_Use.md) · [History](History.md) · [Architecture](Architecture.md). Forensic dumps: [`logs/`](logs/).

**Raw forensic dumps:** the disassembly/export dumps backing this section live in [`logs/`](logs/) (our analysis artifacts only — no NVIDIA binaries, no per-run SL logs).

## Big picture

Two independent paths share one menu state:

- **SR** rides bevy's own pipeline: `bevy_anti_alias` → `dlss_wgpu` 4.0.0,
  which drives the NGX Vulkan API with raw handles extracted from wgpu HAL.
  `DlssInitPlugin` is auto-added by Bevy's DefaultPlugins under the feature —
  **kuluu must NOT add it again** (duplicate-plugin panic); kuluu only inserts
  the project-id resource.
- **NR ("Neural Uplift")** is a custom pipeline around `nvngx_dlssnr.dll`
  v310.8.0, driven directly from Rust through two committed crates:
  `kuluu-dlss-nr/` (FFI — the single home for every unsafe touch of the NGX ABI;
  kuluu-render keeps `#![forbid(unsafe_code)]`) and `kuluu-ngx-fwd/` (the
  forwarder, §"The 'nvngx.dll' calling-module gate").

## SR path

`kuluu-render/src/camera.rs` inserts
`bevy::anti_alias::dlss::Dlss<DlssSuperResolutionFeature>` on the operator
camera when `dlss_active()`. Bevy's `prepare_dlss` ORs `STORAGE_BINDING` into
the main texture usages (NGX writes through storage ops) and runs its node in
EarlyPostProcess. Resources are wrapped as `NVSDK_NGX_Resource_VK` from wgpu HAL
raw handles; the SR evaluate call goes through dlss_wgpu's typed helper
(`NGX_VULKAN_EVALUATE_DLSS_EXT`).

## NR pipeline (as built)

```
main world                          render world
─────────────                       ────────────
GraphicsSettings                    NrState (resource, lazy)
  neural_uplift / nr_* knobs          ├─ runtime: Option<NrRuntime>   (LoadLibraryW once)
        │ apply_neural_uplift_system    ├─ handles: VulkanHandles      (as_hal extraction once)
        ▼ (every frame; self-heals     └─ initialized: bool           (Init_Ext once, retry ≤1/s)
          across camera respawns)
OperatorCamera + NrEnabled{intensity,tone,structure}   ← ExtractComponentPlugin extracts it
│
        ▼  prepare_nr (Render/PrepareViews, before prepare_view_targets)
            ├─ usages |= STORAGE_BINDING on main texture
            ├─ create zero-filled Rg16Float MVec stand-in alongside the feature (cleared once)
            └─ create/recreate feature 0x12 at full window res (own encoder + queue.submit;
               wait_device_idle THEN release old first — ReleaseFeature is handle-only in this ABI)
        ▼  nr_node (Core3d/PostProcess, after SR's EarlyPostProcess; .before(tonemapping))
            ├─ Color = main_texture_view() (pre-flip), MVec = stand-in view,
            │   Depth = prepass depth if sample_count==1 (+ SubrectWidth/Height always: render res when
            │   MainPassResolutionOverride present, full size otherwise)
            ├─ post_process_write() → Output = destination
            ├─ Reset=1 when (has_depth, sub_w, sub_h) changed or first frame after create; else 0
            ├─ barriers on shared encoder (incl. MVec→shader-read); EvaluateFeature in own command buffer;
            │   preserve_main_after_flip() copy if the flip happened but evaluate failed to encode
            └─ ctx.add_command_buffer(cb)
```

Node placement: Core3d set order is `(Prepass, MainPass, EarlyPostProcess,
PostProcess).chain()`; SR lives in EarlyPostProcess → NR sits in PostProcess so
it enhances whatever SR produced when both are on. Per-frame command flow
replicates dlss_wgpu's pattern: barriers via `transition_resources` on the
shared encoder (source→RESOURCE, depth→RESOURCE, output→STORAGE_READ_WRITE),
then EvaluateFeature encoded into a SEPARATE encoder's command buffer and
submitted right after the main one.

## The "nvngx.dll" calling-module gate

The v310.8 NR backend DLL refuses to run unless **the module that called it is
named `nvngx.dll`**. Every exported entry point except `EvaluateFeature`
carries its own copy of this prologue:

```text
Init_Ext(app_id, data_path, instance, pd, device, version, params):
  1. GetModuleHandleExW(6 /* FROM_ADDRESS|UNCHANGED_REFCOUNT */, retaddr, &out)
     fail → "Unable to determine calling module" → return 0xBAD0_0002
  2. get that module's file name (wide, ≤260 chars)
  3. case-insensitive SUBSTRING search for L"nvngx.dll" in the full file name
     mismatch → "Not called from N…" → return 0xBAD0_0002   ← kuluu.exe fails here
  4. only then: real init with the original args
```

That is NVIDIA's trust boundary: feature backends (`nvngx_dlssnr.dll`, and by
the same pattern `dlisr/dlslowmo/dlinpainting`) are meant to be initialized *by
the main NGX runtime* (`nvngx.dll` — what a native DLSS 5 game ships), never
directly by a game or third-party tool. Verified scope in this DLL: Init_Ext,
Init_Ext2, CreateFeature, CreateFeature1, GetFeatureRequirements-ish,
GetScratchBufferSize-ish, PopulateParameters_Impl, ReleaseFeature, Shutdown,
Shutdown1 are all gated; the plain `_Init` stubs just return
FeatureNotSupported (0xBAD0_0001); **only `EvaluateFeature` is ungated** — its
prologue goes straight to lock + FNV-hash dispatch.

## The forwarder (`kuluu-ngx-fwd`)

RenoDX/OptiScaler defeat the gate with a forwarder DLL, and so do we: a tiny
module whose file name contains `nvngx.dll`, which makes the gated calls on our
behalf so the return address lands in the right module. **No byte-patching
anywhere — the gate is defeated by caller identity, not modification.**

- Shape: `cdylib`, zero dependencies, std only (~140 lines). Exports:
  `kuluu_ngx_fwd_vulkan_init_ext(target, app_id, data_path, instance, pd, device, version, params)`,
  `…_init_ext2(…)`, and `kuluu_ngx_fwd_abi_version() -> 1`. `target` is the real
  entry-point pointer kuluu-dlss-nr already resolved; the forwarder loads
  nothing itself — kuluu-dlss-nr stays the single owner of the NR DLL module
  handle and error path. The forwarder's only job is to be the module in which
  the `call` instruction lives.
- **The one thing that can silently break it:** the call MUST compile to a real
  `call`, never a tail `jmp`. A `jmp` leaves kuluu.exe's return address on the
  stack and the gate fails exactly as before. The result goes through
  `std::hint::black_box` so the call is never in tail position at any opt level.
- Naming: cargo builds it as `kuluu_ngx_fwd.dll`; it is staged next to kuluu.exe
  as **`nvngx.dll_kuluu.dll`**. Do NOT name it plain `nvngx.dll`:
  `nvsdk_ngx_s.lib` loads the driver's real `nvngx.dll` for SR, and a same-named
  file next to the exe is a shadowing risk. Unlike the NR DLL, the forwarder is
  ALWAYS re-copied on every build (ours; changes with every build).
- Return codes seen by kuluu-dlss-nr:

| code | meaning |
|---|---|
| `0x1` | success, NR init done |
| `0xBAD0_0002` | still module-gated on THAT call: forwarder missing/stale next to the exe — applies to Init_Ext, CreateFeature AND ReleaseFeature |
| `0xF0F0_0001` | forwarder got a null target (our load-order bug) |
| any other `0xBAD*` | real NGX error, gate passed |

End-to-end: kuluu.exe does `call fwd_init_ext(Some(init_ext), …)` — that return
address sits inside nvngx.dll_kuluu.dll; the forwarder does `call target(…)` and
its return address is also inside nvngx.dll_kuluu.dll → gate passes. Init_Ext,
CreateFeature and ReleaseFeature all go through trampolines; only the per-frame
hot path (EvaluateFeature) stays a direct call.

## Binary ground truth (`nvngx_dlssnr.dll` v310.8.0)

**Export table** (55 exports total). Vulkan entry points we can call directly:

```
NVSDK_NGX_VULKAN_Init            @ RVA 0x13F50   (stub, shared with other backends)
NVSDK_NGX_VULKAN_Init_Ext        @ RVA 0x25050
NVSDK_NGX_VULKAN_Init_Ext2       @ RVA 0x251A0
NVSDK_NGX_VULKAN_CreateFeature   @ RVA 0x24A20
NVSDK_NGX_VULKAN_CreateFeature1  @ RVA 0x24B70   (adds VkDevice as first arg)
NVSDK_NGX_VULKAN_EvaluateFeature @ RVA 0x24C80
NVSDK_NGX_VULKAN_GetFeatureRequirements / GetScratchBufferSize
NVSDK_NGX_VULKAN_GetFeatureInstanceExtensionRequirements / ...DeviceExtensionRequirements
NVSDK_NGX_VULKAN_PopulateParameters_Impl @ RVA 0x252F0   (purpose unconfirmed; not needed)
NVSDK_NGX_VULKAN_ReleaseFeature  @ RVA 0x25410
NVSDK_NGX_VULKAN_Shutdown / Shutdown1
```

Plus CUDA/D3D11/D3D12 variants and generic getters. **NOT exported:**
`AllocateParameters`, `DestroyParameters`, `GetCapabilityParameters`,
`Init_with_ProjectID` (the RenoDX addon's *reference* NR build does export the
D3D12 flavor of these — we use the static host lib's instead, which is strictly
better). Imports: only KERNEL32/USER32/ADVAPI32/VERSION → fully self-contained.

**NR FeatureId = 0x12 (decimal 18)** — from the RenoDX addon's CreateFeature
call site (`mov edx,0x12`). The public enum ends at `RayReconstruction = 13`;
18 is the new v310.8 feature (presumably `NVSDK_NGX_Feature_DLSSNR` /
NeuralRendering).

**Confirmed signatures** (from public SDK headers, stable across versions):

```c
NVSDK_NGX_Result NVSDK_NGX_VULKAN_Init_Ext(
    unsigned long long InApplicationId,      // u64 project id (lower 64 bits of KULUU_DLSS_PROJECT_ID)
    const wchar_t *InApplicationDataPath,    // NGX logs/models land here — we use the OS temp dir
    VkInstance InInstance, VkPhysicalDevice InPD, VkDevice InDevice,
    NVSDK_NGX_Version InSDKVersion,          // pass 0x15 (NGX_VERSION_DOT 1.5.0)
    const NVSDK_NGX_Parameter *InParameters);// we pass NULL

NVSDK_NGX_Result NVSDK_NGX_VULKAN_CreateFeature(
    VkCommandBuffer InCmdList, NVSDK_NGX_Feature InFeatureID /* 0x12 for NR */,
    const NVSDK_NGX_Parameter *InParameters, NVSDK_NGX_Handle **OutHandle);

NVSDK_NGX_Result NVSDK_NGX_VULKAN_EvaluateFeature(
    VkCommandBuffer InCmdList, const NVSDK_NGX_Handle *InFeatureHandle,
    const NVSDK_NGX_Parameter *InParameters, PFN_NVSDK_NGX_ProgressCallback /* NULL */);

NVSDK_NGX_Result NVSDK_NGX_VULKAN_ReleaseFeature(NVSDK_NGX_Handle *InHandle);  // handle-only, NOT cmd-encoded
NVSDK_NGX_Result NVSDK_NGX_VULKAN_Shutdown1(VkDevice InDevice);
```

**Handle ABI — OPAQUE 64-bit object pointer, NOT the public header's
`{ unsigned int Id; }`** (build-11 finding): backend create mallocs a 0xb8-byte
handle object on success (vtable ptr @ +0, refcounts @ +8/+0xC) and stores it
into *OutHandle as a full qword. Internal create registers the object in an FNV
table under key = `*(u32*)obj` (low 32 bits of the vtable address — a build
constant). EvaluateFeature and internal release both read `*(u32*)InHandle` and
FNV-lookup that table; miss ⇒ InvalidParameter 0xBAD0_0005. Consequence: the
caller must pass back the **exact 64-bit value CreateFeature wrote**, cast to a
pointer — never a pointer to its own u32 storage.

**The NR parameter contract** (internal parser at RVA `0x19F30` walks our
caller-supplied opaque parameter object and looks up each field by name):

- **Textures** (value = pointer to an `NVSDK_NGX_Resource_VK`; each also accepts
  `…SubrectBaseX/BaseY/Width/Height` u32 params, default 0):
  `DLSSNR.Color`, `MVec`, `Depth`, `Output`, `ControlMask`, `UI`, `UIAlpha`,
  `Backbuffer`, `BidirectionalDistortionField`.
- **Floats:** `MVecScaleX/Y` (def 1.0), `Intensity` (def 1.0 — the addon menu's
  "NR Intensity", default 1.01 there), `LocalToneStrength` (1.0),
  `SkinStructureStrength` (def −1.0 = "unset"), `LocalStructureStrength`
  (stored as f64, def 1.0).
- **u32/bool:** `UseAutoMask` (0), `Reset` (0), `DepthInverted` (default **1**),
  `Enabled` (default **1**), `UICorrection` (0), `Style`, and the oddballs
  `DLSS.Indicator.Invert.X.Axis` / `.Y.Axis` (0).
- **Getter-slot map** (build-12 disasm of parser + backend eval — CRITICAL for
  the Set* choice): the parser does NOT read every param through a type-matched
  getter. GetI (+0x58) reads `Reset`, `Enabled`, `DepthInverted` and ALL subrect
  params; GetUI (+0x60) reads only `Style`; resources via GetVoidPointer (+0x40);
  floats via GetF (+0x70). A value stored via SetUI is INVISIBLE to the GetI
  lookups — so subrects/Reset/Enabled/DepthInverted all go through `set_i`.
- **Quirk:** `DLSSNR.ScalingRatio` is looked up but then unconditionally
  overwritten to 1.0 locally — accepted, not honored by this build.

**The parameter-object ABI — fully decoded.** The opaque `NVSDK_NGX_Parameter*`
is not a flat name/value array:

```
param_ptr → [ +0x00 : inner_obj* ]          (first qword of the handle)
inner_obj = struct of function pointers, called as fn(param_ptr, name_str, out_or_value):
  +0x00 SetVoidPointer      +0x40 GetVoidPointer   ← resource lookups (out=qword, def NULL)
  +0x08 SetD3d12Resource    +0x48 GetD3d12Resource
  +0x10 SetD3d11Resource    +0x50 GetD3d11Resource
  +0x18 SetI                +0x58 GetI             ← int lookups (out=dword, def 0)
  +0x20 SetUI               +0x60 GetUI
  +0x28 SetD                +0x68 GetD
  +0x30 SetF                +0x70 GetF             ← float lookups (out=dword f32, def 1.0f)
  +0x38 SetULL              +0x78 GetULL
```

Evidence: disassembled `nvsdk_ngx_parameters_lib.obj` from the static host lib —
every exported `NVSDK_NGX_Parameter_Set*/Get*` does exactly
`obj = *param_ptr; fnptr = obj->slot_off; jmp-rax-trampoline(fnptr)`. The v310.8
runtime parser uses slots +0x40/+0x58/+0x70 — identical offsets in the v310.5.3
host layer (cross-version ABI confirmed compatible for everything NR needs).

Consequence: we do NOT hand-build any struct layout. We call the static lib's
own `NVSDK_NGX_VULKAN_AllocateParameters(&map)` +
`SetVoidPointer/SetI/SetF(map, "DLSSNR.…", …)`, then pass that map pointer
straight into `nvngx_dlssnr.dll!VULKAN_EvaluateFeature` — exactly what the RenoDX
addon does with its bundled host layer. Bonus: those Allocate/Destroy exports
come from **the very static lib dlss_wgpu already links** (`nvsdk_ngx_s.lib`,
`lib/Windows_x86_64/x64/`) — kuluu-dlss-nr just declares the externs; no new
link dependency, no bindgen. Note: `DlssSdk.parameters` in dlss_wgpu is
`pub(crate)` → we allocate our own map for NR (clean isolation; SR path
untouched).

**Raw Vulkan handles — all available from Bevy resources** (wgpu/wgpu-hal 29.0.4,
ash 0.38): dispatchable ash handles (`Instance`, `PhysicalDevice`, `Device`) are
`#[repr(transparent)] struct X(*mut u8)` → `.as_raw() -> u64` via the public
`Handle` trait (tuple fields are private); non-dispatchable (`Image`,
`ImageView`) are `struct X(u64)`. Device: `device.as_hal::<Vulkan>()` →
`shared_instance()` / `raw_physical_device()` / `raw_device()`. Textures:
`TextureView::as_hal::<Vulkan>()?.raw_handle() -> vk::ImageView`, same for
`vk::Image`; raw format via `adapter…texture_format_as_raw(fmt).as_raw()`.
Command buffers: `encoder.as_hal_mut::<Vulkan,_,_>(|enc| enc.raw_handle())` —
the same pattern dlss_wgpu uses.

**Bevy-side ground truth (0.19.1 / wgpu 29.0.4):**

- **Depth prepass under SR**: `prepare_prepass_textures` sizes the depth texture
  at full physical target size, but the prepass node applies
  `Viewport::from_viewport_and_override(camera.viewport, MainPassResolutionOverride)`
  — only `physical_size` changes, offset stays top-left. So with SR active the
  full-size depth texture has valid data only in its top-left subrect = render
  resolution → we pass `DLSSNR.Depth.SubrectWidth/Height` from
  `MainPassResolutionOverride`; without SR no subrect params (whole texture
  valid). Color never needs a subrect.
- **MSAA caveat**: prepass depth uses `msaa.samples()` — with MSAA on, depth is
  multisampled and unsampleable by NGX → NR passes Depth only when
  `sample_count() == 1`, else runs color-only (parser tolerates missing depth).
- **Ping-pong**: `ViewTarget::post_process_write()` flips main texture A↔B and
  returns `PostProcessWrite { source, source_texture, destination,
  destination_texture }` — fields, not methods; caller MUST write destination.
  `main_texture_view()` gives the current view WITHOUT flipping, so we build the
  color resource first and only flip once committed to encoding.
- **Extraction is opt-in**: component needs `SyncComponent { type Target = Self }`
  + `ExtractComponent` and an `ExtractComponentPlugin::<T>` in the main app;
  removal propagates automatically.
- **NGX data path**: dlss_wgpu passes `env::temp_dir()` to its init → NR matches
  (no new folders next to the exe).

## Official DLSS 5 knowledge (researched post-launch, 2026-09-04)

NVIDIA released DLSS 5 on **September 3, 2026** (debut title: NBA 2K27; Game
Ready driver + GeForce NOW). What they published, and what it means for our
integration:

- **Research page + paper**: [DLSS 5: Generative Neural Rendering](https://research.nvidia.com/labs/adlr/DLSS5)
  (NVIDIA ADLR, Sept 1, 2026). DLSS 5 is "a real-time generative rendering
  stage" using **3D-guided neural rendering** — a one-step pixel-space diffusion
  model conditioned at inference on *the current rendered frame, engine motion
  vectors, carried temporal state, and artistic-direction values*; causal +
  deterministic; strict per-frame compute budget; real-time up to 4K. "First DLSS
  technology to generate the final displayed appearance rather than reconstruct a
  higher-cost reference output." Runs locally on **GeForce RTX 50 Series**.
- **Gamescom technical briefing** (Edward Liu + Gabriele Leone), covered by
  [TechPowerUp](https://www.techpowerup.com/review/nvidia-dlss-5-technical-preview/)
  — the best public source on runtime behavior.

**Developer docs status:** no developer-facing NR integration doc exists as of
launch day. The public Streamline SDK is still at v2.12.0 (June 23, 2026),
released *before* DLSS 5; its feature list covers SR / RR / MFG only. Our direct-
Vulkan-NGX path remains the only way in — **watch `NVIDIA-RTX/Streamline`
releases** for an NR plugin + programming guide; that would replace our
reverse-engineered parameter names with a supported ABI, and is the first thing
to check after any driver/SDK update.

**Runtime inputs — the big one:** at runtime the model receives exactly two
things — **the rendered output frame and the motion vectors. Nothing else.**
Depth/normals/albedo/lighting are used during training only. Also confirmed:
runs at native output resolution; placement is end of pipeline after all SR
upscaling outputs (our `nr_node` in PostProcess after EarlyPostProcess is
correct); works on any input, no other DLSS tech required; garbage in, garbage
out — TAA/SR pixelation and ghosting are reproduced pixel for pixel; the model
adds lighting/material response but "cannot touch the geometry underneath".

**Implication:** our zero-filled MVec stand-in is confirmed as the prime suspect
for any motion-related artifact. Temporal stability is *trained on* real motion
vectors; feeding "no motion" every frame while the camera moves means the
temporal state cannot track the scene. Fix path: bevy's `MotionVectorPrepass`
when SR is active, then drop the stand-in. Until then, expect shimmer/ghosting
specifically during camera movement — that is the documented limitation, not a
new bug.

**Official controls vs our knobs:**

| Official (launch) | Semantics | Ours (v310.8 runtime) |
|---|---|---|
| **Structure Intensity** 0–1 | high-frequency uplift: AO, contact shadows, reflections, SSS | `DLSSNR.LocalStructureStrength` (`nr_structure_strength`, def 1.0) |
| **Tone Intensity** 0–1 | low-frequency uplift: broad lighting + color response; at 0 the output is identical to the rendered frame for that component | `DLSSNR.LocalToneStrength` (`nr_local_tone_strength`, def 1.0) |
| (global strength) | overall effect amount | `DLSSNR.Intensity` (`nr_intensity`, def 1.01 = addon default; parser default 1.0 ≈ no-op) |
| Models A/B/C | three weight sets, visibly different, no perf difference | not exposed in v310.8 params we know of — check for a model-select key on next disasm pass |
| Masking (auto + developer groups) | both feed ONE screen-space control buffer; zero cost to the model | no mask input seen in our param contract — likely driver-side only at this stage |

Note: our default `nr_local_tone_strength = 1.0` is the *maximum* of the
official 0–1 Tone Intensity range — i.e. full tone uplift by default. If output
reads dark, test Local Tone Strength → 0 first (cheap, no code).

**Performance:** ~50–60% of frame rate on average across RTX 50 GPUs at all
resolutions (NVIDIA's own answer, measured in NBA 2K27) — a design property, not
a bug. Community corollary: DLAA + NR fight over the same Tensor cores; DLSS
Quality mode + NR measured better than DLAA + NR. The model has been made ~5x
faster since development began — expect driver-side cost to keep dropping. RTX 50
only, confirmed by NVIDIA ("Yes, it") — Blackwell FP4 Tensor cores.

**Sources:** [research.nvidia.com/labs/adlr/DLSS5](https://research.nvidia.com/labs/adlr/DLSS5) ·
[TechPowerUp briefing coverage](https://www.techpowerup.com/review/nvidia-dlss-5-technical-preview/) ·
[launch date / hardware scope](https://pcmasterinsider.com/nvidia-dlss-5-release-date) ·
[Streamline releases](https://github.com/NVIDIA-RTX/Streamline/releases)
