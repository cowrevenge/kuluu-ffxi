# ffxi_disassembly

Static-analysis scanner suite for `FFXiMain.dll` — the retail oracle tooling named in
`AGENTS.md`. Python + capstone over a locally installed binary (PE32 i386, ImageBase
0x10000000). Read-only: nothing here patches or redistributes anything.

Full how-to (setup, run order, gotchas, result-recording conventions) lives in
`Cow_doc/disassmembly_docs/README.md`; the findings these tools back live in
`Cow_doc/disassmembly_docs/` (`mob_animation.md`, `event_vm.md`, ...). Every script
prints markdown meant to be pasted into those docs.

## Setup

```bash
pip install -r requirements.txt        # pefile, capstone
# Windows note: `python` may resolve to a project venv; use the system python.
```

The first script that needs disassembly runs a linear sweep of `.text` with capstone
(a few minutes) and caches it in `.cache/`. Every later run reuses the cache. Delete
`.cache/` when you swap in a different FFXiMain.dll build — RVAs are only valid against
one build (the `KNOWN_CLIENTS` rows in `ffxi-dat/src/client_profile.rs`: name the row,
e.g. `retail-2026-09`, beside anything verified on it).

## Pointing at a DLL

Every script takes the install directory or DLL path as its first argument. In this repo
the retail install is registered, not assumed:

```bash
kuluu install list                 # named installs
kuluu install path <NAME>          # prints the DAT root; FFXiMain.dll sits beside VTABLE.DAT
```

## Tools

| Tool | What it does |
|---|---|
| `common.py` | Shared plumbing: PE load, `pol1_decode` (the bit-packed LZSS that unpacks `.text` on packed builds), cached linear capstone sweep of `.text`, `KNOWN_RVAS` anchors. |
| `p0_modmap.py` | Module map: PE32 i386 confirmation for every exe/dll, image bases, sections, imports/exports, version strings. Records the ImageBase/TimeDateStamp/section table every RVA depends on. |
| `p1_anchors.py` | Maps known RVAs to sections, dumps vtable slot lists, finds constructors and `ini`/`init` literals, class-name and `ROM/` strings, ActionTimer2 sites. |
| `p2_handler.py` | Scores every heuristic function by ten signals; prints top candidates with hit sites, callers, full disassembly of the top three. |
| `p3_gates.py` | Gate/predicate pass (entity-type gates and similar branch scans). |
| `p7_event_vm.py` | Event-VM pass: pattern-searches every XiEvents byte pattern (wildcards `??`) against the cached `.text` sweep; name -> hit RVA -> func start. A miss is the fallback to immediate anchors. |
| `p8_jumptable.py` | Dumps the `ExecProg` switch jump table as opcode -> entry VA -> thunk RVA -> handler RVA. Source of `Cow_doc/disassmembly_docs/event_opcode_table.md`. |
| `p9_zone_scene.py` | Lists every DAT whose parsed scheduler routines reference movN / exNN stage names; file ids resolved through VTABLE/FTABLE. |
| `p10_combat_tags.py` | Combat tag census pass. |
| `p11_pkt_tables.py` | Packet table dumps. |
| `probe_tpc_files.py` | Tpc cross-check: resolves the four-band A/B file ids through VTABLE/FTABLE and lists which of tlk0 / thk1 / kka0 each mapped DAT carries. |
| `scene_dat_parse.py` | Parses zone scene DATs (scheduler routines, stage names) for the p9/probe workflows. |
| `dat_routines.py` | Shared DAT/scheduler-routine parsing used by the event-VM tools. |
| `assemble_event_evidence.py` | Rebuilds the raw evidence appendix (`Cow_doc/disassmembly_docs/event_evidence.md`) from raw dumps. Extend its section lists when adding new dumps. |
| `xref.py` | Cross-reference: `--to 0xRVA` (callers), `--from 0xRVA` (callees), `--imm 0x...` (who uses an immediate / on-disk VA / fourcc), `--disp 0x11E --size 2` (who touches a displacement), `--tree 0xRVA --depth N`. |
| `disasm.py` | Disassembly: `--func 0xRVA` (whole heuristic function, annotated), `--rva/--len`, `--va 0x04DF0F40` (convert a runtime VA, base 0x04AC0000), `--bytes 0x32BB38 --len 0x100` (hex/dword dump of vtables/tables). Annotations: known-vtable VAs, fourcc immediates, plan offsets. |

## Examples

```bash
python disasm.py "<install>\FFXiMain.dll" --func 0xBC27C      # whole annotated function
python xref.py   "<install>\FFXiMain.dll" --to 0xD2120        # who calls it
python p8_jumptable.py "<install>" > out_p8.md                # ExecProg opcode table
```
