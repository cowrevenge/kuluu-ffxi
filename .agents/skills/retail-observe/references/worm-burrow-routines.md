# Worm burrow routines + animation dispatch (retail FFXI, observed on HorizonXI + static)

Runtime: 2026-09-08, Carrion Worms in the retail client driven by `wormwatch.lua`
v0.4 (`cow_tools/ffxi_disasm/ashita/wormwatch/`), logs
`wormwatch_20260908_182632.log` and `wormwatch_20260908_182935.log`.
Static: 2026-09-08, `p3_gates.py` (G1–G4) + full-function dumps against retail
`FFXiMain.dll` via `cow_tools/ffxi_disasm` (POL1-packed `.text`; the tooling decodes
it — see that README).

Kuluu's worm implementation (`8fe705f`, `ffxi-actor/src/actor_state.rs`
`BurrowPhase`/`burrow_clip`) implements this. This file is kept as the
**observation record**; open work (generalizing beyond worms) is in the
consolidated animation plan, to be recorded as a bead where `bd` is available. Findings F36–F45 below are **paste-ready for §9 of the out-of-repo
disassembly doc** (`docs/ffxi_disassembly.md`, "your plan + my edits").

## Retail runtime findings (Phase L, closed 2026-09-08)

- **F39 [local] 2026-09-08 (wormwatch_20260908_182632.log, v0.4, Carrion Worm idx
  102):** RF1 decode across a full cycle: dig sub=1 -> subA=1 (bits 1-3), RF1 bit
  0x800 cleared, RF1/RF2 bit 4 set, RF4 bit 7 set. INVISIBLE: RF1 |= 0x100, subA
  still 1. Pop packet (sub=1): new actor and **subA reset to 0**; RF1 returns to
  0x800 at lock start 2 frames later. sub=0 packet: only RF4 bit 7 clears; subA
  already 0; lock runs natural 94 frames. **RF3 never changed** (bit 0 / bit 23 not
  observable at frame granularity: set and consumed within one flush). **subB
  (bits 13-15) never changed**: F30's "second copy" is not a sub copy for this mob;
  re-read 0x9BE6D.
- **F40 [local] (H4, explains F13 vs F38):** sub=0 mid-routine is a no-op when the
  create path has already zeroed subA (this run, F13). It cancels the lock only
  when subA is still 1 at arrival (worm 100 in F38), i.e. a real 1->0 transition on
  a live actor dispatches a routine change (candidate: F29's init->ini0 rewrite).
  Race between create-path reset and packet order. Test: Lock ALL, several cycles,
  correlate `subA=1 -> subA=0` ENT lines with lock cuts.
- **F41 [local]:** RF4 (+0x130) bit 7 = "sub != 0" latch: set on the sub=1 packet,
  survives destroy/create, cleared on the sub=0 packet.
- **F42 [local] 2026-09-08 (wormwatch_20260908_182935.log, 3 Carrion Worms, 7 pop
  locks, 8 sub=0 packets; F38/F40 resolved, H4 dead):** subA is 0 at every sub=0
  arrival (create path resets it, F39); 5 of 6 sub=0 packets during an active pop
  lock did nothing (locks ran 94 ticks). The one cut (idx 100, 71 ticks, f8165)
  landed in the same frame worm 102's dig lock started; the F38 cut (f3362)
  likewise coincided with worm 102's dig start. Scheduler nodes are a shared pool
  (same node address used by both worms). Conclusion: sub=0 mid-routine is a
  no-op; the rare early lock release is cross-entity contention when another mob
  starts a routine in the same frame. Retail quirk, **no Kuluu action** — Kuluu's
  driver has no shared scheduler pool to contend over (validates `8fe705f`'s
  "sub->0 does not cancel" semantics). Not pursued further.

Also in the F42 log: death path = StatusServer/Status -> 3 from the animation byte,
HP -> 0, RF3 bit 28 set, short 19-22 tick lock; post-death disappear shows RF0
0x00406000 (status=3, actor=0) **without** the 0x00C16000 bits INVISIBLE sets, so
those extra bits (0x800000|0x10000) distinguish hide-from-INVISIBLE from
hide-from-despawn.

## Static verification (p3 gates + full-function dumps, 2026-09-08)

- **F36 [local] 2026-09-08 (G1+G2):** RF3 (+0x12C) bit 0 = "rebuild pending". No
  status test inside the update routine 0x8F750..0x926C7; the gate is caller func
  @0x95DB0(ent): `test byte [esi+0x12c],1 / je skip / call 0x92910 / call 0x8F750
  / and al,0xFE` — i.e. `if (RF3 & 1) { rebuild; update; RF3 &= ~1 }`. Three call
  sites, all in per-entity loops (@0x95A43 loop @0x95A58; @0x95A81 path via
  @0x95AEA which first writes `RF3 |= 0x20000000`; @0x95B9A loop @0x95BAB). G1 RMW
  list: bit-0 SETTER = `or [eax+0x12c],1` @0xB9121 (func 0xB90F0, actor from global
  table @0x10480B30 by index; reached only via the stdcall thunk table ~0xBC6AF —
  callback registration); clear bit0+bit23 helper @0x87DC0 (`and [ecx+0x12c],
  0xFFF7FFFF`); clear bit23 only @0xBD378 (`and 0xFFFEFFFF`); two `and
  [esi+0x12c],ebx` in the caller loops @0x95A81/0x95C97.
- **F37 [local] 2026-09-08 (G3 + full dumps):** THE sub->clip dispatch, fully
  decoded:
  - Sub->fourcc table @RVA 0x35AF60 (.data): **inline fourcc array** (not
    pointers), index = RF1 bits 1-3 (`shr eax,1; and al,7` @0x8C5C8 / @0x8F091):
    `[init, ini1, ini2, ini3, init, ini1, ini2, ini3]` — sub 4..7 wraps mod-4.
    Verified byte-for-byte against on-disk .data (POL1 packing only affects .text).
  - Normalizer @0x8EF70(ent): live sub = RF1 bits 1-3; keeps a mirror in RF1 bits
    4-6 (`RF1 := low_nibble | (sub<<3)`); syncs actor+0x8A9 via setSub (+0xa8);
    routes to the change detector when not in steady state (steady: sub=4 &
    nibble=0, or sub=0 & nibble=4).
  - Change detector/applier @0x8EFD5 — the real dispatch: compares RF1 bits 1-3 vs
    actor+0x8A9 (getSub, +0xa4) and the mirror nibble; on change: `setSub(new)`
    then plays `table[sub]` via vtable **+0x29C** and/or **+0x298**, gated by RF0
    bit 9 (+RF4 bit 2 if RF0 bit 13 clear, @0x8F0B2..D8), a check call to
    +0x2A0(actor, fourcc) (@0x8F097-0x8F0A5), and (sub>=4 path) requires RF2 bit
    29 set.
  - State dispatch func @0x8C490(ent): prologue — when RF2 bit 31 is clear: status
    byte +0xEE not in {3,4,5} && predicate 0xD12E0(actor) -> call 0xCF110(ent,0),
    set RF2 bit 31 ("initialized" latch). Entry skip when RF2 bit 29 "dispatched
    this cycle" (set @0x8C61A after processing; cleared with bit 30 by `and
    [esi+0x128],0x5FFFFFFF` @0x90CAD in a destroy/despawn-ish path). Jump table on
    **[esi+0x170] - 6** (byte map @RVA 0x8C680, jump table @0x8C670):
    - raw states {6, 50, 56} (idx 0/44/50) -> fishing helper 0x8C6D0(ent, state):
      plays **'fsh0'..'fsh3'** via +0x29c.
    - raw state 34 (idx 28) -> plays 'init' then **'inte'** (`push 0x65746e69`,
      bytes i-n-t-e — verified literal, unexplained; record as observed), both via
      +0x298; clears actor+0x7D8 to spaces.
    - raw states 64..83 (idx 58-77) -> 0xD60D0: loops 9x calling 0xD60F0 which
      builds fourcc **'wep'+digit** ('wep0'..'wep8') = weapon-attack clips.
    - all other states (<6, >=84, and the rest) -> sub-dispatch block @0x8C5A8:
      getSub->save; setSub(new from RF1 bits 1-3); play 'init' via +0x298;
      post-init hook 0xCF070(ent, actor); clear actor+0x7D8 to spaces; **then
      setSub(old) again** — unexplained (best hypothesis: transient sub for the
      duration of 'init'/post-init processing while the official update happens via
      @0x8EFD5). Open question.
  - RF2 (+0x128): bit 29 = "dispatched this cycle"; bit 30 = one-shot **'hen0'**
    pending — armed @0x95F9C (in tick SM func @0x95EA0) when a routine completes,
    which also sets status byte +0xEE := 2 and RF0 bit 2; consumed+cleared
    @0x8C628..0x8C662 after playing 'hen0' via +0x298 (skipped if status in
    {3,4,5}); bit 31 = initialized latch.
  - Second state machine @0x95EA0(ent) ("tick/advance"): gated by RF0 bit 5; keyed
    on [esi+0x170]-2, byte map @RVA 0x95FF0 (82 entries), jump table @0x95FE8 =
    [0x95ED5, 0x95EDE]; both blocks converge to the routine-end logic above.
- **F43 [local] 2026-09-08 (G4; resolves F30's open question — where the previous
  sub lives):** handler @0x9BCF7 maintains both RF1 copies itself via convergent
  XOR-diff writes (like the status bits in RF0). Branch condition @0x9BE55-0x9BE63:
  `if ((status_byte & 1) || ([pkt+0x28] dword & 0x4000000))` -> **variant B
  @0x9BE88**: `sub<<1; xor RF1; and 0xE` (full 3-bit write into bits 1-3 = the copy
  the dispatcher reads). Else **variant A @0x9BE6D**: `sub<<13; xor RF1; and
  0x6000` (2-bit write into bits 13-14, consumed elsewhere e.g. funcs 0x7A4C1 /
  0x7BDAD which test bit 13). Common tail @0x9BEA5: `RF1 ^= diff`. So: odd status
  or the +0x2B flag -> low copy; even status (2,4) without it -> high copy. Note:
  the tested bit is bit 26 of the dword at pkt+0x28 = **bit 2 of byte pkt+0x2B**
  (the byte after animationsub), not a bit of the sub byte itself.
- **F44 [local] 2026-09-08:** vtable slots on CXiSkeletonActor (vtable @RVA
  0x330F40): +0xa4 (slot 41) @0xA4B20 = getter of byte actor+**0x8A9**; +0xa8
  (slot 42) @0xA4B30 = setter of the same byte (`mov [ecx+0x8a9],al; ret 4`) — so
  "set animationsub" is just a plain byte store. +0x298 (slot 163) @0xCEE50 and
  +0x29c (slot 164) @0x84CD0 = named-animation play slots: both resolve
  fourcc->entry via shared helper 0xCE490 / vtable slot +0x1EC, then start it
  (0x72FC0 global). +0x2A0 = a check method taking (actor, fourcc). Resolver
  0xCE490 walks the actor's loaded ANI clip linked list ([esi+8]=next) and
  special-cases 'init' at its tail.
- **F45 [local] 2026-09-08:** post-init hook family: 0xCF070(actor,actor): if word
  actor+0xB2 != 0 -> 0xCF0C0: if bit 7 of byte actor+**0x840** set -> clear it, try
  playing **'!kl1','!kl2','!kl3'** via 0xCEF10. Else (path B): latch bit 7 of
  +0x840 and call 0xCEF10 with sentinel **'!in?'** (`push 0x3f6e6921`), which in
  0xCEF10 expands to trying the candidate list @RVA 0x330F28 = ['!in1','!in2',
  '!in3']. Weapon path ~0xD608x tries '!f01'/'!f02' gated on bit 13 of +0x840. The
  **'!' prefix is a systematic convention** (only these literals in the whole
  binary: !f01, !f02, !in?, !kl1-3). Best inference: literal clip names with '!'
  marking internal/secondary clips. Open question — cannot verify against game data
  (PhoenixXI install ships FFXiMain.dll + FTABLE/VTABLE.DAT only, no ANI/DAT).

## Kuluu relevance

- `8fe705f` (BurrowPhase FSM, sp1?/sp0? clips, ini1/init effect routines,
  no early-cancel) matches the validated retail model: dig = sub set on a live
  actor running `ini1`; pop-up = fresh actor running `init`; mid-routine sub=0 is
  a no-op (F39/F42).
- **No Kuluu action for F42** — the scheduler-pool contention quirk has no analog
  in Kuluu's driver.
- Remaining work: generalize mob animation beyond worms via the [esi+0x170] state
  machine + sub->clip table (F37). Tracked as open work in beads; see the
  consolidated animation plan produced alongside this record.
