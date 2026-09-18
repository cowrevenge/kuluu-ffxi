# Zone-line crossing and the zone-change request, 2026-09-18

How the retail client detects that the player crossed a zone line and turns
that into the c2s `0x05E` MapRect request. Kuluu code cites sections of this
record by heading.

## Method and naming

Static, community source: XIClient (research/XIClient, pin 2026-07-28) carries
the reconstructed `RidManager`, `ActorTelemetry::CheckZoneLine`,
`GameManager::CliLocalTask` and `KO_RectData::HitCheck`. No retail
disassembly or live observation was taken for this record, so every claim
below is tier-2 community evidence unless marked otherwise.

Retail-DAT evidence: `ffxi-dat/examples/dat-rid-zoneline-probe.rs` dumped the
`RID` zone-line rects from the `retail-2026-09` install (patch `30260904_1`),
zone 230 (Southern San d'Oria, file 330). Those numbers are tier-1 facts about
the shipped data.

## Where the trigger volumes come from

Zone-line trigger volumes are **client data**: the `RID` section of the zone
DAT, one 64-byte entry per oriented box. `RidManager::Add` keeps an entry
unless its source fourcc starts `m`/`M` *and* its rect-class word is non-zero;
for each kept entry it

- takes `|size|` componentwise (negative extents are stored),
- builds the inverse transform `T(-position) . RotateY(-orientation.y) . S(1/size)`,
- caches the eight forward-transformed unit-cube corners,
- stores the source fourcc, the dest fourcc and the param word.

So a rect is a box of **full** extent `size`, yawed by `orientation.y`, centered
on `position`, and a point is inside iff the inverse transform maps it into
`[-0.5, 0.5]` on **all three** axes. `ffxi-dat`'s `ZoneInteraction::contains`
already implements exactly this.

`RidManager::InitZonelines` additionally collects the `z`/`Z` entries with a
non-zero dest fourcc into a flat `Zonelines` array; `InitSubModels` does the
same for `m`/`M`.

## The hit check is a swept segment, not a point test

`RidManager::ZoneLineHitCheck(start, end)` walks every kept rect whose source
fourcc begins `z`/`Z` **and** whose dest fourcc is non-zero, transforms both
`start` and `end` into the rect's unit-cube space, and returns the rect's
fourcc when any of the following holds:

1. `end` is inside `[-0.5, 0.5]^3`, or
2. `start` is inside `[-0.5, 0.5]^3`, or
3. the segment `start -> (end - start)` intersects any of the unit cube's six
   faces with an intersection factor in `[0, 1)`.

Case 3 is `KO_RectData::HitCheck` over the six-entry `KO_RectData::DataTbl`
face table: a plane intersection against the face normal followed by four
inside-edge winding tests with a `-0.0001` tolerance. It is what makes a thin
zone line untunnelable at any frame rate, and it has no counterpart in a
point-in-box test.

### The third winding test as XIClient writes it cannot be retail's

Each winding test is `cross(edge, hit - vertex)` scaled componentwise by the
face normal, required to clear `-0.0001` on every component. Tests one, two and
four take the offset from the edge's own start vertex (`v0`, `v1`, `v3`). Test
three takes the edge `v3 - v2` but reuses `offsetFromFirstVertex` — the offset
from `v0`.

Taken literally that is not a slack test, it is a degenerate one. On a
rectangle `v3 - v2 = -(v1 - v0)`, so test three becomes
`-cross(v1 - v0, hit - v0) . n >= -0.0001` while test one is the same quantity
`>= +0.0001`. Together they pin the hit point onto the line through `v0` along
`v0 -> v1`, to within a ten-thousandth. Every face crossing that is not exactly
along one edge would be rejected, and zone lines could only ever fire through
the two endpoint-containment cases — which is the behavior the sweep exists to
avoid.

So this is a decompilation artifact (the kind of variable reuse a decompiler
folds), not a retail policy. `ffxi-dat`'s `face_crossed` tests each edge
against its own start vertex. Worth re-deriving from the retail binary if the
`KO_RectData` table is ever probed directly.

`ZoneRectHitCheck` is the same sweep without the non-zero-dest condition, so it
also accepts the `z` rects that carry a zero dest fourcc (zone *entrances*).
Its only caller in XIClient is the unused `ZoneRenderer::ZoneRectHitCk2`
wrapper, so what retail uses it for is an open question.

`LiftRectHitCheck` is the same shape again for `@`-prefixed elevator rects, and
is called from `CollidableActor` after collision resolution.

## The per-frame gate and the request sequence

`ActorTelemetry::CheckZoneLine` runs once per frame on the local player, from
`GameManager` immediately before `SomePosUpdater`. It returns early unless

- the actor's active bit (`RF0` bit 9) is set,
- `EventExecFlag` is false (no event or cutscene running), and
- `ZoneChgReqFlag` is zero.

The swept segment is `AUDIT_004.Position -> AUDIT_0A0->Position.Position`:
the previous frame's committed telemetry position (which `SomePosUpdater`
writes *after* the check, so the check always sees last frame's value) to this
frame's actor position. A frame's worth of movement is the sweep.

On a hit the client, in order:

1. cancels autorun (`ControllableActor::IsAutoRunning(false)`),
2. sets `CliZoneFadeOutFlag = 1`,
3. sets `CliEventUcFlag = true`,
4. sets `ZoneChgReqFlag = 1`,
5. sets the global phase to `GamePhase::Seven`.

`ZoneChgReqFlag` is a **one-shot latch**: nothing clears it until the
zone-change reset path (`GameManager` reset sets it and `ZoneChgMode` back to
zero). A refused request does not re-fire on a later line.

## The packet waits for the fade-out to finish

`GameManager::CliLocalTask` drives the fade as a small state machine, all
counts in frames (retail runs at 30 fps, so 60 frames is about two seconds):

- `CliZoneFadeOutFlag == 1`: zone volume snapped to 1.0, effect and system
  volume ramped to 0.0 over 30, the `'0odf'` scheduler started from resource
  file `0x78B8` with a 60-frame life, count set to 60, flag -> 2.
- `== 2`: count decremented by the frame tick until negative, flag -> 3.
- `== 3`: `'0odf'` stopped, flag -> 0.

Only in the `default` arm -- that is, once `CliZoneFadeOutFlag` is back to 0 --
and with `CliZoneFadeInFlag == 0` and `ZoneChgReqFlag == 1`, does the client
call `SendMapRect(ZoneChgId, GetExitMyroomType(), ZoneChgMode)`. On a
successful enqueue `ZoneChgReqFlag` becomes 2 and the camera is re-anchored to
the local player.

**The screen fades to black first and the request goes out afterwards.** The
fade is not a reaction to the server's reply.

Fade-in mirrors it: `CliZoneFadeInFlag == 1` restores the volumes over 30,
starts `'0idf'` for 60 frames and goes to 2/3 the same way. If the player is on
a lift, state 1 waits for `LiftReadFlag` and routes through state 4, which
zeroes the camera offset and re-aims it at the player before the fade-in.

While `CliEventUcFlag` is set, `SendCharPos` returns without sending: the
client **stops publishing position** for the whole fade-out window. (Same guard
also rejects an all-zero position.)

## The 0x05E payload

`GP_CLI_MAPRECT` is 0x14 bytes: `RectID` u32, `x`/`y`/`z` f32, `ActIndex` u16,
`MyRoomExitBit` u8, `MyRoomExitMode` u8. `SendMapRect` sets only `RectID`,
`MyRoomExitBit` and `MyRoomExitMode`; it never writes `x`/`y`/`z` or
`ActIndex`. LSB ignores them too -- its 40-unit sanity check uses the
server-tracked `PChar->loc.p`, not the packet
(`vendor/server/src/map/packets/c2s/0x05e_maprect.cpp`
`GP_CLI_COMMAND_MAPRECT::process`).

`RectID` is the rect's source fourcc as a little-endian u32, which is also the
primary key LSB looks the zone line up by.

`GetExitMyroomType` derives an expected exit bit from a word on the current
zone (`NowZone` +0x40D10) through a fixed switch, compares it against another
word on the same zone (+0x40D44, which is where the `0x00A` `MyRoomExitBit`
lands), and returns the bit only when the two agree -- otherwise 0. The switch
keys (199, 214, 219, 256, 257, 258, 288, 289, 290, 291, 292, 745) map onto the
`MYROOMEXITBIT` enum values LSB names, but what field +0x40D10 actually holds
is not identified here.

## What LSB's zonelines table is worth

LSB's `data/zones/<zone>/zone.yaml` `zonelines` is *not* a copy of the RID
data. Zone 230, LSB against the `retail-2026-09` DAT:

| tag | LSB `from` | DAT `position` | LSB `at[3]` | DAT yaw | LSB `scale` | DAT `size` (x,y,z) |
| --- | --- | --- | --- | --- | --- | --- |
| z6e0 | 113.458, -4.079, -57.351 | same | 0.785398 | 2.356194 | 2, 12 | 15, 10, 2 |
| z6e2 | -113.372, -4.075, -57.418 | same | 2.356194 | 0.785398 | 3, 10 | 15, 10, 2 |
| z6e4 | 0.013, -8.469, 57.965 | same | 4.712389 | 3.141593 | 1, 6 | 12, 15, 2 |
| zmr0 | 164.933, -5.547, 164.792 | same | 2.356194 | 3.926991 | 1, 4 | 12, 8, 2 |

Three things follow:

- `from` is exact. The rect centers are trustworthy.
- `at[3]` is the **arrival facing at the destination**, not the trigger's yaw.
  It differs from the DAT yaw by exactly +/- pi/2 on all four lines, with the
  sign varying per line, so it cannot be corrected by a constant.
- `scale` is a 2-vector with no vertical component and does not match the DAT
  extents: the real gates are 12-15 units wide and 8-15 tall, LSB records
  widths of 4-12 and no height at all. Nothing on the server validates these
  numbers -- LSB only ever looks a zone line up by RectID -- so they are
  unconstrained approximations.

An LSB-scraped trigger box is therefore the wrong volume, rotated up to 90
degrees off, systematically undersized horizontally and unbounded vertically.
The DAT `RID` rects are the only faithful source.
