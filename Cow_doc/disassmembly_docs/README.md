# FFXI retail client disassembly

Read-only analysis of the PhoenixXI install (`C:\PhoenixXI\SquareEnix\FINAL FANTASY XI\`, `FFXiMain.dll`
build TDS 0x6A7297F5) to learn how the client drives mob animation and cutscenes, so kuluu can
implement the same mechanisms generically. Nothing from the install is redistributed; Ashita SDK
headers (LGPL) and XiEvents (AGPL) are cited, not vendored.

## Files

| File | What |
|---|---|
| [mob_animation.md](mob_animation.md) | Mob animation driver: synthesis, reference tables (RVAs, entity/actor layout, RenderFlags bits, stage ops), findings F1-F58, open items, kuluu conclusions |
| [event_vm.md](event_vm.md) | Event VM and cutscenes: struct layouts, motion resource readers, request stack, wait predicates, GetActorIndex, zone scene DAT, camera, Type byte; findings E1-E20 |
| [event_opcode_table.md](event_opcode_table.md) | ExecProg jump table: opcode -> thunk -> handler RVA, with XiEvents names and width notes |
| [tpc_package_table.md](tpc_package_table.md) | Opcode 0x66 Tpc motion package -> A/B DAT file id rule (four bands) with worked examples |
| [mob_evidence_1_modmap_anchors.md](mob_evidence_1_modmap_anchors.md) | §A module map, POL1 entry stub; §B vtable slots, ctor sites, fourcc literals (F24-F27) |
| [mob_evidence_2_entity_update.md](mob_evidence_2_entity_update.md) | §C per-entity update routine: PopEffect, create dispatch, flush wrapper, xrefs (F28, F34, F35) |
| [mob_evidence_3_handler_destroy_live_dat.md](mob_evidence_3_handler_destroy_live_dat.md) | §D name resolvers / state switch; §E 0x0E handler; §F destroy path + xrefs; §G live sessions (routine records, combat); §H on-disk DAT dumps (F29-F34, F46-F55) |
| [event_evidence.md](event_evidence.md) | §A-§M raw dumps behind E1-E20 |

F36-F45 are backed by `.agents/skills/retail-observe/references/worm-burrow-routines.md` and the
`p3_gates.py` outputs rather than an evidence doc here. Pre-existing LSB/kuluu-side facts (wire
offsets, burrow FSM, LSB timeline) are in the kuluu repo's `ffxi_mob_animation.md`.

## Conventions

- **Addresses are RVAs** relative to `FFXiMain.dll` ImageBase **0x10000000** (on-disk VA = 0x10000000
  + RVA). Disassembly dumps print on-disk VAs (0x10xxxxxx). The DLL has no dynamic-base bit, but
  runtime bases differ per session: wormwatch sessions saw 0x04AC0000 and 0x03DF0000; convert with
  `RVA = VA - base` (each log records its base).
- **`.text` is POL1-packed on disk** (rawsize 0): bit-packed LZSS in the POL1 section, unpacked by the
  entry stub at load (F24). Every static scan runs on the decoded image via
  `cow_tools/ffxi_disasm/common.py:pol1_decode`; RVAs are unaffected. `.rdata`/`.data` are not packed.
- **Hot functions are entered mid-function** and reached through vtables and thunk tables, so
  `xref.py --to <func start>` often returns 0. Use `--to` on the real entry address, `--imm <vtable VA>`
  for constructors, and `--disp <offset>` for field consumers (F33, F34).
- **Packet offsets vs struct offsets.** s2c body offsets (status +0x1C, animationsub +0x26) are not
  entity-struct offsets; in the handler `esi` is header-inclusive (body = esi+4).
- **Evidence tiers:** `[local]` = verified against this install's binary or memory; `[web]` = from a
  public source (Ashita SDK, XiEvents, XiClient decompilation, cexi docs), unverified here;
  `[external]` = SE's own code via the PS2 decompile or a reimplementation (xim). A web fact promoted
  to local gets a new entry; tiers are never edited in place.
- **Numbering:** mob pass findings are F<n>, event pass E<n>. Evidence sections are cited as
  "mob_evidence_3 §G.1" / "evidence §K". Every finding carries RVA + evidence pointer; nothing
  lives only in chat.

## Tooling

- `cow_tools/ffxi_disasm/`: `common.py` (PE + cached capstone sweep, POL1 decode), `p0_modmap.py`,
  `p1_anchors.py`, `p2_handler.py --dump`, `p3_gates.py`, `p7_event_vm.py`, `p8_jumptable.py`,
  `p9_zone_scene.py`, `xref.py --to/--imm/--disp`, `disasm.py --func/--rva/--range`,
  `dat_routines.py` (DAT chunk walk + stage-stream parser, `--scan` for a census),
  `scene_dat_parse.py`. Outputs: `out/` (Phase 0), `out2/` (mob pass), `out3/` (event pass).
- `cow_tools/ffxi_disasm/ashita/wormwatch/wormwatch.lua` (Ashita v4 addon, v0.5): entity lock,
  0x0E/0x28 packet decode, XiAtelBuff diffs, actor and scheduler-node diffs, pointer crawl with
  fourcc scan. Logs under `ashita/wormwatch/logs/`.
- Python 3.13 with `pefile` + `capstone`; MinGW objdump as a bounded fallback. No Ghidra/r2 used.
- Reference implementations (read, not vendored): teschnei/lotus-ffxi, InoUno/xi-tinkerer,
  atom0s/XiEvents, AshitaXI/Ashita-v4beta `entity.h`, vekien/xi-model-viewer + xi-tools,
  sruon/FFXI-PS2, Aamace xim, LandSandBoat.

## Raw source map

| Source | Reproduced in |
|---|---|
| `out/p0.utf8.md`, `out/entrystub.md` | mob_evidence_1 §A |
| `out2/p1.md` | mob_evidence_1 §B, mob_evidence_2 §C.4 |
| `out2/d_906c0.md`, `d_90840.md`, `d_908ec.md`, `d_8f6e0.md`, `d_92680.md`, `d_8f7e0.md`, `d_8f900.md`, `d_95D9F.md`, `d_95A43.md`, `d_c56e0.md`, `d_c5880.md`, `d_a4110.md`; `x_8f750.md`, `x_c56f0.md`, `x_c5890.md`, `x_a4150.md`, `x_92910.md`, `x_disp_12c.md`, `x_2c3511.md` | mob_evidence_2 §C |
| `out2/d_ce241.md`, `d_d6b40.md`, `d_5cbf0.md` | mob_evidence_3 §D |
| `out2/p2.lf.md`, `d_9be88.md`, `d_9c0e6.md` | mob_evidence_3 §E |
| `out2/d_928a3.md`, `x_c525e.md`, `x_a4110.md` | mob_evidence_3 §F |
| `wormwatch_20260908_231759.log`, `wormwatch_20260909_000019.log` | mob_evidence_3 §G |
| `dat_routines.py` on 5.DAT / 11.DAT / 32.DAT; `out2/datdump.zip` | mob_evidence_3 §H; F55 |
| `out3/*` (`p7.md`, `p8.md`, `d_*.md`, `x_*.md`, `probe_*.md`, `p9_zone_scene_scan.md`, `d_scene23*.md`, `cib_b9_census.md`) | event_evidence §A-§M |
