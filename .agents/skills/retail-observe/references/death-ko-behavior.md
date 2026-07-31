# Death / KO behavior (retail FFXI, observed on HorizonXI + LSB)

Observed 2026-06-12, dying and being returned to the home point (Windurst Woods,
zone 241) on a LandSandBoat server, referenced against a vanilla HorizonXI client.

Kuluu's death handling (corpse pose, homepoint countdown, `hud/death_prompt.rs`,
the `0x037` decode) implements this. This file is kept as the **observation
record**; open work is in beads.

## Retail behavior

- Death plays a **collapse motion once** and holds the final corpse frame — it is
  not a looping idle. (Kuluu resolves the corpse pose to `cor?` via
  `ffxi-actor` `idle_animation_id` under `dead && owner_is_none`, registered as a
  looping idle; whether `cor?` is itself a collapse motion or a static pose needs
  a live run to settle.)
- Retail shows the **homepoint menu**, not a visible numeric KO clock. A numeric
  countdown is therefore an Enhanced-flavored addition unless proven otherwise.
- Music changes on the homepoint warp; the death-music slot must not survive it.
- The faithful server signal for the dead pose is `animation == ANIMATION_DEATH (3)`.

## Homepoint timer wire facts (LSB)

`0x037` char_status (`GP_SERV_SERVERSTATUS`):
- `dead_counter1` at body offset **0x38** (u32 LE).
- `hpp` is bits 16..24 of `Flags0` (body **0x24**).
- `seconds_until_homepoint = dead_counter1 / 60 - 360`. LSB pads `dead_counter1`
  with a fixed 6 min; the server's `CDeathState` force-warps at death + 60 min.
- Gate on the self packet **and `hpp == 0`**: `GetHPP()` clamps living HP to
  `max(1, …)`, so `hpp == 0` is a true KO sentinel. `dead_counter1` alone is
  identical for alive and fresh-dead, so it cannot be used on its own.
- The server only re-sends `0x037` on status changes, so a displayed countdown has
  to tick locally between packets and re-anchor on each fresh value.
- `0x00A LOGIN` carries a `DeadCounter` at body offset **0xA0** with the same
  encoding — relevant only when zoning in while still KO'd.

Offsets/formula confirmed against `vendor/server/.../char_status.cpp`,
`charentity.cpp::GetTimeUntilDeathHomepoint`, `ai/states/death_state.cpp`.

## Lifecycle gotcha

A homepoint warp **is a zone change**, and a zone change does not cycle
`AppPhase::InGame` — so `OnExit(InGame)` cleanup never runs on a warp. Pose, self
Y, and music slots all have to be cleared per-zone-change explicitly. See the
`zone-change-not-a-clean-lifecycle` memory and the `bevy-lifecycle-symmetry` skill.
