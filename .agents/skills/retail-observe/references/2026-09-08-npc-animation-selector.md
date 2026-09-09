# NPC animation routine selector, 2026-09-08

Binary observation for `kuluu-lpdn`; no live retail session was available.

## Build and method

Inspected the installed `vendor/game-files/SquareEnix/FINAL FANTASY XI/FFXiMain.dll`:
SHA-256 `f4f90fbd080c05448aab3f866b127d7c1675b3cc15c8beaa57bfc584064b7e7c`.
Image base is `0x10000000`; unpacked `.text` begins at VA `0x10001000`
and contains 3,289,278 bytes. Addresses below are VAs for this build.
The installed DLL was not changed or executed for this inspection.

## Observed computation

At `0x1009b280`, the packet byte at offset `0x2a` is doubled and
merged into the actor field at `+0x124` using mask `0x0e`.
Only the original byte's low three bits reach this selector.
The adjacent branch at `0x1009b25d` instead retains its low two bits
in actor bits 13-14.

Routine dispatch at `0x1008e594` reads actor `+0x124`, shifts right one,
masks with seven, and indexes the table at `0x10356df8`:

| Index | Routine |
| --- | --- |
| 0, 4 | `init` |
| 1, 5 | `ini1` |
| 2, 6 | `ini2` |
| 3, 7 | `ini3` |

Thus the effective routine choice is `raw_byte & 0x03`. Bit two aliases
the same routine; higher bits do not choose `ini1`. This does not establish
what every higher bit does, nor that every `ini1` routine means burrowing.
Motion and effects remain model-authored DAT content.

## Server and regression cross-check

LSB `vendor/server/sql/mob_pools.sql` supplies Damselfly sub=8,
Brutal Sheep sub=16, and Sand Hare sub=0. The spawn path in
`src/map/packets/entity_update.cpp` ORs four before ORing the raw value.
Values 8/12 and 16/20 therefore select `init`.

Kuluu's earlier worm implementation removed only bit two, treating the
remaining high bits as an active burrow. Its missing `sp1?` fallback then
played ordinary idle with one-shot loop parameters. The pre-fix live
Damselfly log held frame 2 for over 28 seconds. These are Kuluu regression
observations, separate from the retail computation above.

Local raw evidence: `artifacts/verify/animation-regression-2026-09-08/retail-selector-evidence.txt`
and `before/burrow-evidence.log` in that directory. The bead carries fix
validation and the user recording timestamps.
