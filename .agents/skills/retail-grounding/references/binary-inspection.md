# Inspecting the installed retail client

Use this route when the question needs the original computation or binary
layout. It is not a requirement to disassemble every vanilla change.

## Locate the relevant build and data

The usual install is `vendor/game-files/SquareEnix/FINAL FANTASY XI/`, or the
user's `FFXI_DAT_PATH`. Inspect only the needed paths; that installation can
also contain account configuration and logs unrelated to the investigation.
`FFXiMain.dll` contains client logic and lookup tables. Resolve model DAT IDs
through the installation's VTABLE/FTABLE and existing `ffxi-dat` readers rather
than guessing ROM paths. `rg` normally ignores this install, so use an explicit
path with `--no-ignore` when searching it.

Search the relevant policy in `research/XIClient/src/XIClient/source/` to find
candidate callers and data structures. XIM and cexi references can supply search
terms. Preserve the distinction between a community hypothesis and a rule
independently confirmed in the installed binary.

## Packed FFXiMain.dll

Some builds store compressed code in a `POL1` PE section while `.text` has no
raw bytes. Inspect the actual PE section table before disassembly. The local
`research/cexi-docs/reference/ffximain.md` and
`research/cexi-docs/dats/research/pol_decompress.py` describe an LZSS decoder;
read them as hypotheses/tools, not universal offsets or lengths.

- Inspect the installed unpacker stub and section sizes rather than assuming
  the example build matches. Bound decoding to the actual destination size and
  reject invalid back-references or truncated output.
- Prefer a raw decompressed `.text` dump for analysis. Its virtual start is
  **ImageBase + .text.VirtualAddress**, not ImageBase alone. Distinguish file
  offset, RVA and VA in every citation.
- Do not run or replace the game's DLL with an analysis artifact. A PE whose
  `.text` raw size was zero cannot safely be reconstructed by just writing the
  code at its old raw pointer: another section may occupy that file range.
  Preserve the installed binary; put dumps in an ignored artifact directory or
  temporary directory.
- Use an available disassembler (for example Capstone, Ghidra or IDA) and
  confirm instruction boundaries and cross-references. A matching byte pattern
  alone is not a function identification. Respect normal tool-install and
  execution permissions; absence of a preferred tool is not a reason to claim
  the binary was verified.

## Make the conclusion reproducible

Record the input path, SHA-256, image base and relevant section RVA; include the
function addresses and how they were identified. For a DAT measurement, record
the file ID, resolved path, resource/locator index, raw value and coordinate
conversion. Scope addresses and measurements to that build.

Follow the call chain far enough to establish the actual inputs and exceptions.
For example, the nameplate investigation on `kuluu-81r8` found that the name
caller defaults to locator 2 and the model accessor special-cases that ID using
static translation and model scale. Reading only the general animated locator
helper would have produced a plausible but wrong fix. Consult that bead for
its build-specific evidence; do not reuse its addresses on another build.

Keep binaries, decompressed code, raw assets and screenshots out of git.
Commit independently written implementation and concise evidence citations,
not copied third-party source. A binary-derived rule still needs comparison
with the resulting Kuluu behavior before claiming runtime parity.
