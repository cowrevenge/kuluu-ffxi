# research/

Read-only third-party material used while re-implementing FFXI client
behavior. **Nothing under here is redistributed by this repo** — it is either
a submodule pointer (URL + commit, no upstream bytes committed) or fetched on
demand and gitignored.

Treat everything here as **reference only**: study behavior and re-express it
in our own code. Do not copy, paste, or link third-party source into the
workspace crates.

## Contents

- `Phoenix/`, `AltanaViewer/`, `XiEvents/`, `XiPackets/` — submodule pointers
  to upstream repos cited in source comments. Deinitialized by default;
  populate on demand with `git submodule update --init research/<name>`.
  See *Which reference for what* below before trusting any of them for
  bit-level format details.
- `cexi-viewer/` — [cexi-viewer](https://github.com/CatsAndBoats/cexi-viewer),
  a Tauri/WebGL2 FFXI asset browser (zones, NPCs, PCs, textures, audio) with
  GPU skinning. GPL-3. Reference for DAT parsing, skeleton posing, and
  zone/weather rendering.
- `cexi-docs/` — [cexi-docs](https://github.com/CatsAndBoats/cexi-docs),
  community docs of FFXI's internal formats (DAT, animation, zone mesh, event
  bytecode, audio, VFX). GPL-3. Format cross-reference for `ffxi-dat` /
  `ffxi-audio`.
- `XIClient/` — [XIClient](https://gitlab.com/Aenge/XIClient), a from-scratch
  playable C++ FFXI client (no license — all rights reserved). Reference for
  client architecture and vanilla behavior only.
- `xim/` — the XIM browser FFXI client (**gitignored**, fetched locally). See
  below.

## Which reference for what

These sources are **not equally authoritative**. When they disagree, prefer
the higher tier:

1. **Retail itself** — the disassembled FFXiMain/POL `.text` and live
   observation (the `retail-observe` skill) are the oracle. Bit-level
   questions (field widths, masks, flags) are settled here, nowhere else.
2. **`XIClient/`** — disassembly-grounded; the best community reference for
   **bit-level format accuracy** (field widths, in-memory-only bits). No
   license: read-only.
3. **`Phoenix/`** — server-side divergence signal for wire-protocol
   questions (LSB under `vendor/` stays authoritative for runtime).
4. **`cexi-docs/`** — community format docs (DAT, animation, zone mesh,
   event bytecode, audio, VFX). Useful cross-reference for `ffxi-dat` /
   `ffxi-audio` work, but AI-assisted: treat claims as hypotheses and
   verify against tier 1–2 before baking values into the crates.
5. **`cexi-viewer/`** — rendering and asset-pipeline reference: WebGL2 GPU
   skinning, zone time-of-day/weather, BGW/SPW playback. Most useful for
   `ffxi-viewer-core` materials and `ffxi-actor` posing.
6. **`xim/`** — broad behavioral/architecture reference (actor handling,
   packet flow, DAT pipeline), but the author rarely consulted the
   disassembly and states XIM is unaware of in-memory-only bits/fields.
   **Do not trust XIM for bit-level format details** — it carries latent
   bugs there (e.g. it walks DAT chunks with a 20-bit size field where
   retail uses 19; harmless on retail data only because bit 26
   (`is_shadow`) is always clear). Confirm any XIM-derived mask or width
   against XIClient or the disassembly.

## XIM

[XIM](https://xim.pages.dev/) is Aamace's from-scratch browser FFXI client
PoC (unrelated to atom0s's Xi* repos). It's a
useful reference for vanilla feature behavior — actor/animation handling,
packet flow, DAT parsing — when filling in the parity scoreboard.

- Live app:   <https://xim.pages.dev/>
- Source zip: <https://xim.pages.dev/source.zip>
- Docker:     <https://github.com/Masin-M/xim-docker>

**License: GPL-3.** Fetch a local copy with:

```bash
research/fetch-xim.sh
```

This downloads and extracts the source to `research/xim/`, which is gitignored
so the GPL-3 source never enters our history. Re-run the script to refresh.
