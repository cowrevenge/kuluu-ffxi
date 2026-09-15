# Lower Jeuno persistent effects and point lights (2026-09-14)

Tracked by `kuluu-8wb2` and `kuluu-9j8i`.

## Observation

The user supplied `Screen Recording 2026-09-14 at 4.49.50 PM.mov`.
At about 7–15 seconds, the room beside Caruvinda and the Guide Stone has
hanging amber lamp halos, a white ceiling glow, and a shaft above the stone.
The displayed game time is about 04:50. The video does not identify its DLL,
injected graphics settings, or whether graphics modifications are disabled.
It proves that these effects appear in this recorded original-client session;
it does not by itself establish exact vanilla photometry.

A pre-fix Kuluu release session in the same room used Dynamic Lights Vanilla,
Low preset, bloom 0, and FfxiFaithful characters. At 09:40–09:47 the hanging
lamp halos rendered but the ceiling glow and shaft did not. Different times
prevent an exact brightness comparison. Local evidence is under
`artifacts/verify/lower-jeuno-lighting/`: `retail-ceiling.png`,
`retail-monument.png`, `room-initial.png`, `room-vanilla.png`.

## DAT identities and resources

Both local horizonxi-2023 and retail-2026-09 installations have identical
Lower Jeuno file 345, `ROM/1/41.DAT`, SHA-256
`eda5020a9d10f1850dfb2493cc5ceddb9b0009cd84a18431566c309a891812e2`.
Coordinates below are the authored DAT basis (negative Y is up).

| Generator | Chunk offset | Resource | Position | Lifetime |
| --- | ---: | --- | --- | ---: |
| myl7 | 1009888 | SpriteSheet ligh | (19.905813,-9.682327,47.062130) | 0 |
| SPLT | 1010208 | MMB ligh | (19.930250,-0.000001,46.994091) | 0 |
| c14 | 8802336 | Point light | (19.940557,-6.597886,47.022369) | 0 |

Both visual generators are auto-run. `SPLT` has no UV scroll. Its MMB is
`tshimonolightstp`, with local Y bounds [-10.2755,-5.69514], matching the shaft
above the stone. `c14` is a separate illumination source: range 18, theta 20,
RGB bytes (99,74,42), and no clock updater. A point light alone does not draw
its visible glow or shaft.

The outdoor `pl00` family links the `pttm` theta track through initializer
0x6C and updater 0x49. Its (day fraction, theta) points are
(0,3.5), (.25,3.5), (.275,0), (.725,0), (.758333,3.5), (1,3.5).
Indoor `c1`–`c14` have no such updater. Applying one sun-altitude gate to the
entire zone cannot represent both populations.

## Binary corroboration

Build: **retail-2026-09**. Raw unpacked `.text` SHA-256:
`b55f8b4c730c00229e2febd1ea6a5efba29920e094fa565a3816b763c3d9cdc9`.
Image base 0x10000000; `.text` RVA 0x1000. Capstone inspection of the raw
text followed the calls from the point-light draw function into both helpers.
Local disassembly: `artifacts/verify/lower-jeuno-lighting/retail-light-disasm.txt`.

- RVA **0x56400**, point-light OnDraw: range is field_19C times field_1A8;
  theta is nonnegative field_1A0 times field_1A4 times field_138, clamped
  nonnegative. Calls InitLight at RVA 0x178610 and UpdateLight at RVA 0x178530.
- RVA **0x178610**, InitLight: clears the D3DLIGHT8 record before populating
  it. Constant and linear attenuation remain zero. Range is passed through.
- RVA **0x178530**, UpdateLight: writes range to light+0x4C and reciprocal
  theta to Attenuation2 at light+0x5C. Nonpositive theta sets diffuse RGB to
  zero. RGB bytes multiply the float at RVA 0x32A778, verified as **1/128**.

These independently confirm the relevant arithmetic in
`research/XIClient/src/XIClient/source/World/Generator/Effects/CMoPointLightProgElem.cpp`
(InitLight, UpdateLight, OnDraw).

The following remain tier-2 reconstructions, not independently identified
branches in this binary inspection:

- `research/XIClient/src/XIClient/source/World/Generator/CYyGenerator.cpp`
  HandleOne opcode 0x58 initializes range/theta multipliers as authored value
  plus one; opcode 0x49 samples the clock track into theta.
- `research/XIClient/src/XIClient/source/Rendering/ZoneRenderer.cpp`
  UpdateBlockLightSettings selects the positioned block's four authored
  references, skips missing lights, and excludes lowercase-c light IDs from
  terrain. Actor light selection is a separate policy.

Screenshots cannot uniquely establish these algorithms. General generator
animation, spline interpolation, and exact actor selection beyond the existing
binding path are not established by this observation.

## Persistent effect alpha

The first live implementation drew the shaft, but the user identified excessive
opacity. SPLT's initializer 0x16 is RGBA `a8 80 44 32`: alpha **50/255**,
not full opacity. Its generator has no alpha animation track. Kuluu's
trackless fallback replaced this value with 1.0.

Tier-2 source chain: CYyGenerator HandleOne opcode 0x16 writes field_F8;
`research/XIClient/src/XIClient/source/World/Generator/Effects/CMoElem.cpp`
VirtOt1 calls GetWithScaledAlpha on that color; CMoD3mElem DoMMBDraw supplies
the resulting SomeColor as TEXTUREFACTOR. The persistent fallback now keeps
the initializer alpha. This also restores the authored 128/255 initial alpha
of DAT 210's moon sprite and kasa halo; their prior tests assumed full alpha.

The original exclusion and 2.4 range multiplier were each temporarily restored
in an isolated checkout. The Lower Jeuno effect-ownership and authored-light
regressions failed, then passed after restoration. Live captures and logs are
under `artifacts/verify/lower-jeuno-lighting-fixed/`.

Final release GPU verification used the explicit retail-2026-09 install and
Vanilla configuration (`graphics-alpha-vanilla.json`). At 05:02 and 05:27,
`shaft-alpha-corrected.png` and `shaft-alpha-wide.png` show the ceiling source
and tapered shaft, with masonry visible through its lower portion. The
comparison establishes restored geometry and authored transparency, not exact
pixel photometry. Broader point-light animation coverage is `kuluu-boxh`.


## Ground selection and actor light conversion follow-up

The user reported visible indoor/outdoor transitions, missing outdoor sources,
unlit actors indoors, and concentrated outdoor light on actors. The following
binary inspection uses the same **retail-2026-09** build and unpacked text hash
identified above. Disassembly is retained in
`artifacts/verify/lower-jeuno-lighting-regressions/` as
`collision-light-disasm.txt` and `actor-light-adjust-disasm.txt`.

- RVA **0x167E1E** remaps the collision object's four byte light references at
  offsets **0xAC..0xAF** through a one-based light table. RVA **0x169A47**
  copies its area at **0xB0** into collision-query output; **0x169A53** copies
  the four light references.
- The actor ground-light resolver at RVA **0x181AA0** reads those references.
  Its masked comparison at **0x181ADE** excludes IDs with the suffix `lgb`.
  Render-mesh bounding boxes do not supply this actor selection.
- Actor AdjustLighting at RVA **0xCB4E0** evaluates point-light attenuation at
  the actor origin. **0xCB698** limits the selected point lights to one;
  **0xCB845** converts it to a directional light with the sampled intensity.
  This agrees with `SkeletalMeshActor::AdjustLighting` in XIClient. Computing
  inverse-square distance separately at each body fragment is not this rule.
  Temporal smoothing also exists in the binary; this change does not establish
  or implement its complete state-dependent timing.

The Lower Jeuno floor probe found area `ev01` and actor lights c3/c4/c14 at
native (15,-1,45), and area zero with pl09/pl13 at (-15,-6,-45).
The pl09 street light is near (-18.936,-9.5,-48.269), range 6. Its visible
lt14/RP34 effects are near (-18.95,-11.12,-48.33), separately controlled by
night alpha tracks. These coordinates are the authored DAT basis.

The Ashenbubs Lower Jeuno DAT has SHA-256
`be38b8de5c6916a70df2e211b57dc30d4398d7e8e827f82686d25c3debcd15f7`.
A directory-aware comparison against retail found 1934 chunks in each and
98 differing chunks, all texture type 0x20. Collision, geometry, generator,
and keyframe chunks are identical. This rules out different generator
placements in that pack; it does not prove identical texture appearance.

Kuluu now resolves actor lighting from collision-floor metadata and terrain
lighting from each positioned block's area. Per-area GPU buffers preserve
indoor and outdoor terrain lighting simultaneously. Vanilla actor point
lighting uses the strongest assigned point sampled once at the actor origin.
Enhanced point shadows retain spatial sampling. Distance fog still uses the
player's area, and exact retail transition smoothing remains unverified.
