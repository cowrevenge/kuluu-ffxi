# Treasure pool chat lines (retail FFXI, from player screenshots)

Observed 2026-08-03 from two retail chat-log screenshots supplied by the
project owner (one HorizonXI capture of a Rock Lizard camp, one of a Leaping
Lizzy claim with the log split into two windows).

Kuluu's implementation composes these lines from the retail DAT rather than
from English literals — see `ffxi-dat/src/sysmes.rs` and
`ffxi-dat/tests/sysmes_treasure.rs`. This file is the **observation record**;
open work is in beads (epic `kuluu-k3gm`).

## What the screenshots show

Drop and obtain lines are **multicoloured**: the item name is a distinct green
span inside a line that is otherwise the log-mode colour (white in both
captures). Bracketing the coloured run:

```
You find a [lizard tail] on the Rock Lizard.
...obtains a [lizard tail].
You find a [pair of bounding boots] on Leaping Lizzy.
Macnugget obtains a [pair of bounding boots].
```

The colour boundary is exactly the item-name substitution slot — it starts at
the first character of the item name and ends at its last, excluding the
preceding article and the following preposition or period.

"on Leaping Lizzy" has no "the" while "on the Rock Lizard" does: that is the
`NamedFlag` bit of s2c 0x0D2 selecting the `[the /]` alternative in the entry.

The two captures use different palettes for the surrounding text (one shows
battle messages in cyan, the other in white), which is consistent with these
colours being the player's configurable Config → Font Colors and not fixed.

## Where the wording and colour selection live

`ROM/27/76.DAT` is the client's system-message DialogTable — 326 entries in the
NA install, self-identifying because entry 262 is the untranslated placeholder
`sysmes262` and entry 40 is `mes40`. Treasure entries:

| Entry | Message |
|---    |---      |
| 15  | `<name> does not meet the necessary requirements to obtain the <item>.` + `<item> lost.` |
| 16  | `You find a <item> on [the ]<mob>.` |
| 17  | `<name>'s lot for the <item>: <n> points.` |
| 18  | `<name> obtains a <item>.` |
| 19  | `<name> obtains <n> gil.` |
| 31  | `You do not meet the requirements to obtain the <item>.` + `<item> lost.` |
| 130 | `You cast lots for the <item>.` |
| 131 | `You obtain a <item>.` |
| 164 | `A <item> was lost.` |
| 218 | `You find a <item> in the <container>.` |

Each entry leads with `0x1F <mode>` — the chat-log message type that picks the
line's colour out of the player's font-colour config. Across the whole table
the modes are 9/10/13/14 (chat-channel echoes), 121 (normal system), 123
(error), 127 (item obtained), 136 (countdown), 138 (trade), 141 (moogle), 200
(debug), 208 (examine), 214 (unity). The treasure lines use 121 and 127.

**There is no `0x1E` (`CC_SET_COLOR`) anywhere in this table.** The item name's
green therefore does not come from an inline colour code — the client colours
the substitution itself, which is why the span boundary lands exactly on the
substituted text.

## Still unpinned

Actual RGB values. The captures are photographs of a display at unknown
settings, so sampling them would pin the camera, not the client. The renderer
currently maps each span kind onto the HUD theme. Pinning retail's default
font-colour palette wants a clean in-VM capture via this skill.
