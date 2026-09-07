# DLSS — History

Part of the DLSS doc set: [How to Use](How_to_Use.md) · [History](History.md) · [Architecture](Architecture.md). Forensic dumps: [`logs/`](logs/).

---

## Doc lineage

| Version | Path @ commit | Content |
|---|---|---|
| v1 | `docs/DLSS.md` @ `208e59b` (149 lines) | Original how-to-use/build doc for SR. |
| v2 | `ffxi_dlss5.md` @ `b146e45` (968 lines) | NR research & implementation log, pre-merge (§-numbered). |
| v3 | `dlss_docs/DLSS.md` @ `c9a1c73` (1,166 lines) | Merged doc: Part 1 current system + Part 2 NR log + Build history appendix. **Authoritative source for this folder's content.** |

v3 was then consolidated into README §"Optional DLSS builds" (~line 110,
machine-independent build instructions) and the 1,166-line file deleted at
commit `4855997`. This folder restores the full content as per-topic files
(`How_to_Use.md`, `History.md`, `Architecture.md`). Recovery commands (all
commits are ancestors of current HEAD):

```bash
git show c9a1c73:dlss_docs/DLSS.md   # v3 merged doc
git show 208e59b:docs/DLSS.md        # v1
git show b146e45:ffxi_dlss5.md       # v2
```

## Build history (NR integration, builds 1–13)

- **Builds 1–5** — compile-error rounds against the pinned sources
  (bevy/bevy_render/bevy_core_pipeline/bevy_camera 0.19.1, wgpu/wgpu-hal/
  wgpu-types 29.0.4, ash 0.38.0+1.3.281). Recurring traps: ash handle tuple
  fields are private (raw extraction goes through the public `ash::vk::Handle`
  trait `.as_raw()`); bevy 0.19 wraps wgpu types in `WgpuWrapper` structs that
  only Deref where a target type is known; `RenderDevice` is not an alias for
  `wgpu::Device` (use `.wgpu_device()`); kuluu-render has no direct `wgpu`/
  `tracing` deps and forbids unsafe code, so every raw Vulkan touch lives in the
  FFI crate.
- **Build 6** — first full binary, clean compile + link; game works with SR at
  DLAA, but the NR DLL load failed with a misleading "not found" (LoadLibraryW
  returns NULL for any failure).
- **Build 7** — absolute-path `LoadLibraryW` next to current_exe() +
  GetLastError diagnostics. Still "export missing".
- **Build 8** — root cause of the export miss: the `resolve!` macro passed a
  Rust string literal's `str::as_ptr()` to GetProcAddress, which has **no NUL
  terminator** (Rust literals are not C strings) — it searched for the symbol
  name plus adjacent .rodata garbage. CString-based lookup fixed it; all five
  symbols then resolved clean and exposed the real blocker: `Init_Ext` returns
  PlatformError 0xBAD0_0002 — the calling-module gate ([Architecture](Architecture.md)).
- **Build 9** — forwarder crate `kuluu-ngx-fwd` defeats the gate for Init_Ext;
  first run passed init, then `CreateFeature(0x12)` returned PlatformError —
  disasm proved the gate is NOT init-only: every export except EvaluateFeature
  carries it.
- **Build 10** — extended forwarder (ABI v2): trampolines for CreateFeature +
  ReleaseFeature; recreate attempts throttled to 1/s after a failed create.
- **Build 11** — handle ABI mismatch found and fixed: the v310.8 runtime uses an
  **opaque 64-bit object-pointer handle**, not the public header's `{ u32 Id }`
  ([Architecture](Architecture.md), "Binary ground truth"). Build 10 passed a pointer to its own 4-byte struct, so the runtime's
  FNV table lookup missed every frame (per-frame InvalidParameter). After the
  fix evaluate succeeds every frame; two new problems surfaced — visual
  artifacts and an `ERROR_DEVICE_LOST` crash on mode change.
- **Build 12** — four fixes: (1) write every parameter unconditionally each
  frame, with subrects/Reset/Enabled/DepthInverted through `set_i` (the parser's
  GetI slot — SetUI values are invisible to those lookups); (2)
  `wait_device_idle` before every ReleaseFeature (kills the device-lost UAF);
  (3) `DLSSNR.Reset=1` when the depth signature changes or on first frame after
  create; (4) zero-filled Rg16Float MVec stand-in instead of NULL. Plus:
  ping-pong `preserve_main_after_flip()` copy if a flip happened but evaluate
  failed to encode, and `.before(tonemapping)` so NR enhances HDR data before
  the LDR conversion.
- **Build 13** — two-encoder fix for wgpu-core's EncodingApi mixing panic: the
  one-time MVec clear ran on its own high-level-only encoder (submitted first),
  while feature creation uses a raw-only encoder. General rule: **one
  CommandEncoder, one API style**. Clean release build of all fixes; evaluate
  succeeds every frame, device-lost crash gone.

**2026-09:** `build_cowland.bat` was de-DLSS'd — the `dlss` feature dropped from
the default local build (whitepaper hold; also during bisection of an
exclusive-fullscreen stale-screenshot bug on the NVIDIA driver path). The repo
itself was not changed by that edit. Re-enable recipe:
[How to Use — Building with DLSS](How_to_Use.md#building-with-dlss).
NVIDIA-supplied material (runtime DLLs, SDK/toolchain checkouts) moved from
`streamline/` to [`Dlss_Nvidia_dll/`](../Dlss_Nvidia_dll/README.md); the scratch folder was
deleted. Forensic .txt dumps survive under [`logs/`](logs/).
