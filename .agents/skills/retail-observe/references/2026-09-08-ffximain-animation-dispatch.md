# FFXiMain.dll animation dispatch, 2026-09-08

Static observation of the retail client's routine dispatch: how a wire `animationsub` byte and an
actor state byte become named DAT routines. Companion to
[2026-09-08-npc-animation-selector.md](2026-09-08-npc-animation-selector.md), which records the
packet-side selector; this record covers the dispatch machinery it feeds.

## Build and method

Inspected the installed `vendor/game-files/SquareEnix/FINAL FANTASY XI/FFXiMain.dll`:
SHA-256 `f4f90fbd080c05448aab3f866b127d7c1675b3cc15c8beaa57bfc584064b7e7c`, the same build as
the sibling record. Image base `0x10000000`; the `.text` section is POL1-packed, so addresses are
VAs of the unpacked image and `.data` reads were verified byte-for-byte against the on-disk file
(packing only affects `.text`). The installed DLL was not changed or executed for this inspection.

## Sub-to-routine table

An inline fourcc array at RVA `0x35AF60` (`.data`, not pointers) maps the wire sub to a routine:

| Index | Routine |
| --- | --- |
| 0, 4 | `init` |
| 1, 5 | `ini1` |
| 2, 6 | `ini2` |
| 3, 7 | `ini3` |

The index is the actor field at `+0x124` (RF1) bits 1-3: the dispatch sites shift right one and
mask with seven (`shr eax,1; and al,7`). Sub values 4..7 therefore wrap mod-4 onto the same four
routines. This is what absorbs the spawn flag bit two that LSB ORs into `animationsub` on spawn:
the raw 3-bit value indexes the table directly.

## Normalizer and change detector

Two functions sit between the packet handler and the play slots:

- The normalizer at `0x8EF70(ent)` reads live sub = RF1 bits 1-3, keeps a mirror in RF1 bits 4-6
  (`RF1 := low_nibble | (sub<<3)`), syncs actor+`0x8A9` via the setSub vtable slot (+0xa8), and
  routes to the change detector when not in steady state. Steady states: sub=4 with nibble=0, or
  sub=0 with nibble=4 (zero with / without the spawn flag).
- The change detector at `0x8EFD5` is the real dispatch. It compares RF1 bits 1-3 against
  actor+`0x8A9` (getSub, +0xa4) and the mirror nibble; on a change it calls setSub(new) and then
  plays `table[sub]` via vtable slots +0x29C and/or +0x298. The play is gated by RF0 bit 9 (plus
  RF4 bit 2 when RF0 bit 13 is clear), preceded by a check call to slot +0x2A0(actor, fourcc).
  The sub>=4 path additionally requires RF2 bit 29 set.

## State dispatch switch

The state dispatcher at `0x8C490(ent)` jumps on `[esi+0x170] - 6` (byte map at RVA `0x8C680`,
jump table at `0x8C670`):

| Raw states | Action |
| --- | --- |
| 6, 50, 56 | fishing helper: plays `fsh0`..`fsh3` via +0x29c |
| 34 | plays `init` then `inte`, both via +0x298; clears actor+0x7D8 to spaces. The literal fourcc bytes i-n-t-e were verified in the instruction stream; their meaning is unexplained and recorded as observed |
| 64..83 | loops nine times building `wep` + digit (`wep0`..`wep8`), the weapon-attack clips |
| all others | sub-dispatch block: getSub/save, setSub(new from RF1 bits 1-3), play `init` via +0x298, run the post-init hook, clear actor+0x7D8 to spaces, then setSub(old) again. The transient re-set is unexplained; best hypothesis is a temporary sub for the duration of init/post-init processing while the official update happens in the change detector |

Prologue: when RF2 bit 31 (initialized latch) is clear and status byte +0xEE is not in {3,4,5}
and a predicate holds, it calls an init helper and sets bit 31. Entry skips when RF2 bit 29
("dispatched this cycle") is set; that bit is cleared together with bit 30 by
`and [esi+0x128],0x5FFFFFFF` in a destroy/despawn-ish path.

RF2 (+0x128) bits observed: 29 = dispatched this cycle, 30 = one-shot `hen0` pending (armed when a
routine completes, which also sets status byte +0xEE := 2 and RF0 bit 2; consumed after playing
`hen0`, skipped if status is in {3,4,5}), 31 = initialized latch.

A second state machine at `0x95EA0(ent)` ("tick/advance") is gated by RF0 bit 5, keyed on
`[esi+0x170]-2` (byte map at RVA `0x95FF0`, jump table at `0x95FE8`); both of its blocks converge
on the same routine-end logic.

## Named-play vtable slots

On CXiSkeletonActor (vtable at RVA `0x330F40`):

| Slot | Address | Role |
| --- | --- | --- |
| +0xa4 (41) | 0xA4B20 | getter of byte actor+0x8A9 (getSub) |
| +0xa8 (42) | 0xA4B30 | setter of the same byte (`mov [ecx+0x8a9],al; ret 4`): "set animationsub" is a plain byte store |
| +0x298 (163) | 0xCEE50 | named-animation play slot |
| +0x29c (164) | 0x84CD0 | named-animation play slot |
| +0x2A0 | - | check method taking (actor, fourcc) |

Both play slots resolve fourcc to entry via the shared helper at `0xCE490` / vtable slot +0x1EC,
then start it. The resolver walks the actor's loaded ANI clip linked list (`[esi+8]` = next) and
special-cases `init` at its tail; names that resolve nowhere fall through to shared/common DATs
and finally to client built-ins (see the runtime record's in-tree cross-checks).

## Rebuild-pending bit

RF3 (+0x12C) bit 0 is a rebuild-pending flag. The update routine itself contains no status test;
the gate lives in the caller at `0x95DB0(ent)`: `test byte [esi+0x12c],1 / je skip / call
rebuild / call update / and al,0xFE`, i.e. if (RF3 & 1) { rebuild; update; RF3 &= ~1 }. Three
call sites, all in per-entity loops. The bit-0 setter is `or [eax+0x12c],1` at `0xB9121` (reached
only via a stdcall thunk table, i.e. callback registration); a helper at `0x87DC0` clears bits 0
and 23 together (`and [ecx+0x12c],0xFFF7FFFF`).

## Sub latch and dual copies

RF4 (+0x130) bit 7 is a "sub != 0" latch: set on the sub=1 packet, survives destroy/create,
cleared on the sub=0 packet.

The packet handler at `0x9BCF7` maintains both RF1 copies of the sub via convergent XOR-diff
writes (the same pattern as the status bits in RF0). Branch condition: if ((status_byte & 1) or
(pkt+0x28 dword & 0x4000000)) then variant B writes `sub<<1; xor RF1; and 0xE` (full 3-bit write
into bits 1-3, the copy the dispatcher reads); else variant A writes `sub<<13; xor RF1; and
0x6000` (2-bit write into bits 13-14, consumed elsewhere). The tested bit is bit 26 of the dword
at pkt+0x28, i.e. bit 2 of byte pkt+0x2B (the byte after animationsub), not a bit of the sub byte
itself. So: odd status or that flag -> low copy; even status without it -> high copy.

## Post-init hook family

The post-init hook at `0xCF070(actor,actor)`: if word actor+0xB2 != 0 it calls `0xCF0C0`, which
on bit 7 of byte actor+`0x840` set clears it and tries playing `!kl1`, `!kl2`, `!kl3`; otherwise
it latches that bit and calls the same helper with sentinel `!in?`, which expands to trying the
candidate list at RVA `0x330F28` = [`!in1`, `!in2`, `!in3`]. The weapon path tries `!f01`/`!f02`
gated on bit 13 of +0x840.

The `!` prefix is a systematic convention: these literals are the only ones in the whole binary
(`!f01`, `!f02`, `!in?`, `!kl1-3`). Best inference: literal clip names with `!` marking
internal/secondary clips. Not verifiable against game data from this install (it ships FFXiMain.dll
plus FTABLE/VTABLE.DAT only, no ANI/DAT).
