# Wormwatch runtime observations, 2026-09-09

Runtime observation of retail mob animation: four Ashita Lua probe sessions (wormwatch v0.4 and
v0.5) driving Carrion Worms and Forest Hares in the retail client, plus in-tree cross-checks of
the action-packet driver against pinned vendor/server and research/xim. Companion to
[2026-09-08-ffximain-animation-dispatch.md](2026-09-08-ffximain-animation-dispatch.md) (static
dispatch) and [2026-09-11-locomotion.md](2026-09-11-locomotion.md) (the Kuluu chase model).

## Build and method

Sessions 1-3 ran 2026-09-08 on the retail client (HorizonXI install, FFXiMain.dll
SHA-256 `f4f90fbd080c05448aab3f866b127d7c1675b3cc15c8beaa57bfc584064b7e7c`); session 4 ran
2026-09-09. The probe locked named actors, decoded the 0x0E/0x28 packets and scheduler nodes in
memory, and sampled at roughly 34.5 ms per frame (client ~29 fps), so "frames" below are probe
frames unless noted; "ticks" are 60/s scheduler ticks as the DAT viewer decodes them.

## Dig cycle RF decode

Across a full Carrion Worm dig cycle: dig sub=1 -> subA=1 (RF1 bits 1-3), RF1 bit 0x800 cleared,
RF1/RF2 bit 4 set, RF4 bit 7 set. INVISIBLE: RF1 |= 0x100, subA still 1. Pop packet (sub=1): new
actor and subA reset to 0; RF1 returns to 0x800 at lock start two frames later. Sub=0 packet:
only RF4 bit 7 clears; subA already 0; the lock runs its natural length. RF3 never changed during
the cycle (bit 0 / bit 23 not observable at frame granularity: set and consumed within one
flush). SubB (RF1 bits 13-15) never changed for this mob.

## Mid-routine sub=0 is a no-op

Sub=0 arriving mid-routine did nothing in every observed case where the create path had already
zeroed subA: across three sessions, 12 of 13 mid-pop sub=0 packets were no-ops and every lock ran
to its full length. The one early cut coincided with another worm's dig lock starting in the same
frame; scheduler nodes are a shared pool (the same node address was used by both worms). Conclusion:
sub=0 mid-routine is a no-op, and the rare early release is cross-entity contention when another
mob starts a routine in the same frame. Retail quirk with no Kuluu analog: Kuluu's driver has no
shared scheduler pool to contend over.

## Death and despawn flags

Death path: 0x0E status byte -> 3, HP -> 0, RF3 bit 28 set, a short lock (19-22 probe frames for
the worm; 53 for the hare, so death length is per model as expected for a DAT routine). Post-death
despawn shows RF0 `0x00406000` (status=3, actor=0) without the `0x00C16000` bits that INVISIBLE
sets: the extra bits (`0x800000 | 0x10000`) distinguish hide-from-INVISIBLE from
hide-from-despawn. Despawn packet: 0x0E status=2, mask 0x30; UpdateMask 0x0F -> 0x00, actor
destroyed.

## Routine record format

The probe caught both burrow routine records at the moment of dispatch through the first scheduler
node's +0x114/+0x118 pointers. A parsed routine record is a flat stage stream: each stage begins
with a dword whose low byte is the stage type and whose high byte is the stage length in dwords,
header included (held on all 12 consecutive stage boundaries observed). Types seen:

| Type | Meaning |
| --- | --- |
| 0x01 | record header (2 dw) |
| 0x5F | sibling reference (4 dw; payload +8 = fourcc of the other routine). The xim parser names this op StopRoutine, so dig stops `init` and pop stops `ini1` |
| 0x1F, 0x07 | 3 dw each, unknown at the time; 0x07 is an AnimationLock in the SE/xim op tables |
| 0x05 | motion (10 dw: +4 timing word, +8 clip fourcc, two 1.0f values, tail word) |
| 0x0A | sound (8 dw: +4 = 0 or 5, +8 = sound id as four ASCII digits) |
| 0x02 | VFX generator (4 dw: timing, instance fourcc, pointer to the generator object) |
| 0x29 | unknown; appears mid-stream in the pop routine, so not a terminator |

Decoded: dig = {xref `init`, motion sp1?, sound 7025, kak0/mok0/mok1/dis0 generators} and pop =
{..., sound 7024, mok1, motion sp0?, kak1, VFX}. Both match the on-disk DAT byte for byte: dig is
routine `ini1` (sound 7025), pop is routine `init` (sound 7024). An earlier reading had taken the
0x5F payload for the record's own name and concluded the contents were swapped; that claim is
withdrawn. The routine carries the sound SE id as ASCII digits, resolved by the `se%3.3u/se%6.6u.spw`
path template.

## Spawn flag rides unmasked

A worm already underground when locked carried RF1 with subA = 5: the raw wire value 1|0x04 stored
unmasked in bits 1-3 (an unwatched spawn packet had mask 0x57, sub=5). With the mod-4 table, sub 5
resolves to `ini1` and sub 4 to `init`: the wrap is what absorbs the spawn flag, and the
normalizer's steady states are "zero with / without the spawn flag". Indexing an 8-entry table
with the raw 3-bit value matches retail exactly.

## Lock statistics

Six dig locks all ran 56 probe frames (1.94 s); seven pop locks all ran 94 frames (3.24 s). RF2 on
a fresh actor: `0xA0020001` idle -> bit 4 set on dig; destroy -> create drops it to `0x00020001`
(bits 29/31 cleared, matching the static record's `and 0x5FFFFFFF`) and it returns two frames later
when `init` dispatches. RF1 on a fresh actor: low bits 0x...112 -> 0x...180 (create) -> 0x...800
(dispatch).

## First non-burrow routines

Engaging a worm: 0x0E mask 0x06 anim=1 -> StatusServer 0->1, HP drops, RF3 bit 28 set; the status
byte syncs seven frames later. Every hit taken and every swing is a short scheduler routine on the
worm's actor (locks of 15-42 probe frames), with the actor type field stepping 4 -> 2 -> 3 -> 0xD.
ActionTimer1 is a count, not a bool: it read 2 when two routines overlapped. The records crawled
from those nodes carry fourcc families never seen during burrow: `dam2`/`dam3`/`dam4` (damage
reaction, right after the first player hit), `atk2`/`atk3`/`atk4`, `at2?` with `skaz`/`dada`,
`hit1`, `main`. So melee reactions and swings use the same mechanism as burrow: named DAT routines
resolved against the model DAT and pushed as scheduler nodes, triggered from the action packet
rather than from 0x0E.

## Melee is atk0 by name

Ninety-eight decoded 0x28 packets across two hares fought to death plus the player's own rounds:
every melee round (category 1) carries `0x306B7461` in the 32 bits at bit 86, i.e. the fields read
as actionid=29793 / recast=12395 are one field that spells the fourcc `atk0`. WS/mobskill start
(category 7) carries `0x65746163` = `cate`. These are LSB constants (the pinned vendor/server names
them, see in-tree cross-checks). One routine per melee round: lock 24-26 probe frames (~0.85 s),
starting the frame after the packet, hit or miss makes no difference to the attacker. Crawls during
those locks show the hare's attack motion clips `at00`, `at10`, `at20`, `at21` (the at?0/at2?
wildcard family), never the same one in sequence: the variant is picked client-side. The packet's
result `anim` field is 0 for mobs (players: 1 = second swing of a double attack). Per-target result
fields as decoded: react 8 hit / 9 miss / 24 WS or TP-move hit; eff 32 damage, 64/96 crit-ish
variants, 34 with msg 67 = critical; msg 1 hit, 15 miss, 43 "readies", 185 WS/TP damage.

## TP move packets

The mob TP move is two packets ~15 frames apart: category 7 (`cate`, target = the mob itself,
result param = mob skill id 259 = Foot Kick, msg 43) then category 11 (param 259, result react 24,
anim 3, msg 185). The category-7 "readies" packet produces no lock on the mob: it runs the DAT
routine `cate` (= call nerm + cast), which has no AnimationLock stage; the node was not visible in
the probe because the watched head node was occupied by the hit reaction. The category-11 packet
starts two routines at once (ActionTimer1 jumps by 2, a third joins two frames later), total lock
40-44 probe frames (~1.4-1.5 s); a motion-task node carried `sp10` and a crawl showed `wz60`. So the
`sp` clip family is "special" per model (worm: sp0?/sp1? = dig/pop, hare: sp1? = Foot Kick), and the
mob skill's `anim` value is what the client turns into an effect DAT whose routine names the clip.

## Target-side reactions

Being hit runs a damage-reaction routine of 15 probe frames (~0.5 s) on the target, overlapping
freely with the target's own swing (ActionTimer1 counts both). Clips seen around those frames:
`btl0`/`btl1` (battle stance), `swy1`/`swy2`/`swy3` (sway/knockback family, after a two-hit round),
`hit6`, `dfi6`/`dbi6` (recurring near reactions). Engage: 0x0E mask 0x06 anim=1 -> StatusServer
0->1, status synced seven frames later; ~9 frames after that a 14-frame lock with no packet behind
it (tentatively the idle-to-battle-stance transition, btl clips).

## View-range despawn and respawn

Hares leaving view went through UpdateMask 0x0F -> 0x00, RF0 -> `0x00406000`, actor destroyed;
re-entering view: UpdateMask 0x00 -> 0x0F, RF0 -> `0x00402200`, new actor, RF2
`0xA0020001` -> `0x00020001` -> back two frames later. Identical to the pop-up create path: leaving
and re-entering view is a destroy/create and replays `init`. Also seen unwatched: an actor with
sub=8 (does not fit RF1's 3 bits; the handler's shift-and-mask drops it to 0).

## In-tree cross-checks

The action-packet driver, checked against pinned sources in this tree:

- The 32-bit field at bit 86 is one scheduler name. Pinned vendor/server
  `src/map/enums/four_cc.h` (`enum class FourCC`) names it: `BasicAttack = 0x306B7461 // "atk0"`,
  `SkillUse = 0x65746163 // "cate"`, plus the cast/interrupt families. The old "actionid + recast"
  split reading of this field is withdrawn; the observed values stand.
- research/xim `poc/Actor.kt` (tracked submodule) shows the driver rules: melee auto-attack enqueues
  `atk0` as a non-blocking voice routine plus a separate swing picked from the model's attack ids
  (`onAttackMainHand`: random sub-attack id, else `bti0`; H2H kick picks `cti0`/`dti0`); ability
  ready enqueues `cate` with high priority; death runs the model's `dead` routine, falling back to
  `dea<displayAppearanceState>` when absent.
- research/xim `resource/table/MobAbilityTable.kt` (`getFileTableOffset`) gives the FTABLE index for
  a mob skill's animation id: +0x0F3C below 0x200, +0xC1EF below 0x600, +0xE739 below 0x800, else
  +0x14B07. Foot Kick (anim 3) lands at 0xF3F; the client loads that effect DAT and runs its `main`
  routine, whose motion stage names the caster's sp?? clip.
- research/xim `resource/EffectRoutineParser.kt` treats scheduler ops 0x07 and 0x59 as AnimationLock
  (delay accumulates into the start tick; duration in 60/s ticks) and op 0x5F as StopRoutine(name),
  which is what makes the burrow records' 0x5F stage a "stop the sibling" rather than a plain
  cross-reference.
