# Death / KO behavior (retail FFXI, observed on HorizonXI + LSB)

Observed 2026-06-12, dying and being returned to the home point (Windurst Woods,
zone 241) on a LandSandBoat server, referenced against a vanilla HorizonXI client.

Kuluu's death handling (corpse pose, homepoint countdown, `hud/death_prompt.rs`,
the `0x037` decode) implements this. This file is kept as the **observation
record**; open work is in beads.

## Retail behavior

- Death plays a **collapse motion once** and holds the final corpse frame — it is
  not a looping idle. Settled from the installed DATs, no live run needed: the PC
  skeletons' `dead` routine is two Motion stages, `ded?` (116 half-frames = 58
  frames on Hume M; 68..156 half-frames across the seven PC skeletons) followed by
  `cor?`, and `cor?` is a **static** pose whose first and last keyframes place
  every bone in the same spot (worst measured gap over the seven skeletons:
  2.4e-7 on a translation, 2.4e-7 on `|q1.q2| - 1`, with one bone's identity
  rotation stored sign-flipped). So the collapse is the one-shot and the corpse
  frame is what persists — `dat-routine-stages 7072 dead`, pinned by
  `retail_dead_routine_is_a_one_shot_collapse_into_a_static_corpse_pose`.
- **Inference, not observed:** a death the client never watched (zoning in still
  KO'd, or a corpse already dead when it streams into view) starts on the held
  corpse frame rather than replaying `ded?`, which would pop the corpse upright to
  fall over again. Wants a retail capture of walking up to an already-dead player.
- Retail shows the **homepoint menu**, not a visible numeric KO clock. A numeric
  countdown is therefore an Enhanced-flavored addition unless proven otherwise.
  **Settled 2026-09-08 (kuluu-8t5h): Enhanced**, on the strength of the dated
  observation above. The XIClient decompile is consistent with that call but does
  not settle it: `GC_ZONE::field_40D6C` is written by exactly two packet handlers
  — 0x00A (`Payload.field_A4 / 60 + ntGameTimeGet()`) and 0x037 (`gameTime +
  dead_counter1 / 60`, or `dead_counter2` when that is already in the future) —
  and the sole read site found (grep `field_40D6C`) is the zone-in ResState
  branch in `GameManager.cpp`, which is an **undecompiled stub**:
  `SPDLOG_ERROR("ResState not implemented")` guarding two empty `<= 360`
  branches. What retail draws from that value is therefore unknown, and "no HUD
  reader exists in a partial decompile" is absence of evidence, not evidence of
  absence — XIClient reconstructions are community evidence until corroborated
  by the retail binary or observation (research/AGENTS.md). Kuluu's "Home Point
  in M:SS" line now lives behind the `enhanced-death-countdown` cargo feature,
  off in default and release builds.
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
  encoding — relevant only when zoning in while still KO'd. Decoded as of
  kuluu-8t5h (`ffxi-proto` `ServerLogin::dead_counter`), gated on the same KO
  sentinel: `PosHead.HpMax` is `GetHPP()`, so `hpp == 0` works there too. Retail
  agrees on the offset — its `GP_SERV_LOGIN.field_A4` (the struct's names run +4
  ahead of the payload offsets, pinned by
  `static_assert(offsetof(GP_SERV_LOGIN, field_A8) == 0xA4)`) is the u32 it
  divides by 60.

Offsets/formula confirmed against `vendor/server/.../char_status.cpp`,
`charentity.cpp::GetTimeUntilDeathHomepoint`, `ai/states/death_state.cpp`.

## Lifecycle gotcha

A homepoint warp **is a zone change**, and a zone change does not cycle
`AppPhase::InGame` — so `OnExit(InGame)` cleanup never runs on a warp. Pose, self
Y, and music slots all have to be cleared per-zone-change explicitly. See the
`zone-change-not-a-clean-lifecycle` memory and the `bevy-lifecycle-symmetry` skill.
