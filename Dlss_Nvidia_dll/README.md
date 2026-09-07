# Dlss_Nvidia_dll — where NVIDIA's DLSS files go

This folder is the single home for **NVIDIA-supplied** DLSS material in this
repo. Everything in it except this README is git-ignored: these are NVIDIA's
files, not ours, and they never get committed. Anyone who wants to try a DLSS
build supplies their own copies (sources below) and drops them here — then the
rest of the repo works unchanged.

## Required at runtime (drop in next to this README)

| File | What it is | Where to get it |
|---|---|---|
| `nvngx_dlss.dll` (~54 MB, Windows; Linux: `libnvidia-ngx-dlss.so.*`) | NGX backend runtime for **Super Resolution** | Official NVIDIA DLSS SDK: `git clone --branch v310.5.3 https://github.com/NVIDIA/DLSS`, then take `lib/Windows_x86_64/rel/nvngx_dlss.dll` (or the matching Linux `.so`) |
| `nvngx_dlssnr.dll` (~166 MB, Windows only) | **Neural Uplift** (DLSSNR / DLSS 5 NR) backend runtime v310.8.0 | Not in any public SDK tag (public headers end at v310.7.0 with no NR). Obtain the v310.8 runtime from your own legitimate source |

The build (`build_cowland.bat`) and the client do not need these for a
non-DLSS build; without them the DLSS menu rows simply read `N/A`.

## Optional, compile-time only (when building with `--features dlss`)

Clone into this folder so `scripts/checks.sh` / the env vars find them:

| Path | What it is | Where to get it |
|---|---|---|
| `Dlss_Nvidia_dll/sdk/` | NVIDIA DLSS SDK v310.5.3 (headers + static libs, incl. `nvsdk_ngx_s.lib`) | same clone as above (`DLSS_SDK=<repo>\Dlss_Nvidia_dll\sdk`) |
| `Dlss_Nvidia_dll/vulkan-sdk/` | Vulkan SDK headers (`VULKAN_SDK=<repo>\Dlss_Nvidia_dll\vulkan-sdk`) | LunarG Vulkan SDK installer |
| `Dlss_Nvidia_dll/llvm/bin/libclang.dll` | libclang for bindgen (`LIBCLANG_PATH=<repo>\Dlss_Nvidia_dll\llvm\bin`) | any LLVM/clang distribution with the DLL in `bin/` |

## License note

All files here are NVIDIA proprietary. Redistribution is governed by the
NVIDIA DLSS SDK license (section 9.5 of the SDK programming guide) — keep that
text alongside any distributed binary. The Streamline layer DLLs
(`sl.common.dll`, `sl.dlss*.dll`, `sl.interposer.dll`, …) are **not** needed
by Kuluu: our two runtimes import only KERNEL32/USER32/ADVAPI32/VERSION and
are fully self-contained.
