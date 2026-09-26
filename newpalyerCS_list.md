# BASTOK NEW-PLAYER CS — WHAT AM I ASKING (zone 235, events 0 + 7)

Source of truth: zone master block `0x7FFFFFF0` and narration block `0x010EB001`,
zone-235 event DAT (ROM/25/45.DAT family, `ffxi_dat::event_locate::event_dat_file_id(235)`).
Every line below is one or more instructions in execution order; offsets are absolute into
the owning block's data region (traceable via `cargo run -p ffxi-event --example zz-disasm -- 235 <event> [block]`).

**Related docs:** per-opcode semantics — `research/XiEvents/OpCodes/0xNNNN.md`.
This file is the per-ask breakdown + coded/verified status for the Bastok new-player CS specifically.
The Southern San d'Oria twin (event 503) is the format reference.

## Trigger (server-side, not a VM ask)

Hidden quest `newCharacterCS`
(`vendor/server/scripts/quests/hiddenQuests/New_Character_Cutscenes.lua`, BASTOK_MARKETS):
- `onZoneIn` → `player:setPos(-280, -12, -90, 0)` + `return { 0, -1, flags }` → **event 0**.
- `onEventFinish[0]` → `player:startEvent(7, {isHidden=true})` → **event 7**.
- `onEventFinish[7]` → Adventurer's Coupon + marker tutorial + `notSeen=0`.

The server spawn `(-280,-12,-90)` is the walk's **END**; the authored scene snaps the player to the
walk's **START** (NW) and walks them SE into the market. `unique_no` for both events is the player
server id (s2c 0x032, no target NPC).

## Cast (server ids)

| id | who |
|---|---|
| `0x7FFFFFF0` | ZONE_PLAYER_ACTOR — the local player / zone block (owns the shared player position) |
| `0x010EB001` | the narrator — drives event 0's narration + event 7's scene |
| `0x010EB043` | Reet (event 7) |
| `0x010EB005/06/07/0A/0B/3C` | galka kids + background (event 7 owner blocks; event 0 NPC REQSETs) |
| `0x010EB025/29/2A` | background NPCs (event 0 NPC REQSETs) |

## DATs the event reads on demand

| DAT id | file | contents |
|---|---|---|
| **30839** | `ROM/62/87.DAT` | camera routines `sNNN` (operand 130 + base 30704) |
| **30840** | `ROM/62/88.DAT` | camera routines (continuation) |
| **30904** | `ROM/62/110.DAT` | fades `fdi/fdo` + overlay `ovl1` (operand 200) |

## Hold legend

- `[W n]` = WAIT n frames (n/60 s), VM-internal clock
- `[IN]` = player input gate (MESWAIT) — event 0 narration is auto-advance, event 7 is manual
- `[R tag]` = routine hold: host arms it from the routine's authored length in its DAT
- `[REQ]` = REQSET/REQWAIT on a scene actor stack

---

# EVENT 0 — narration + walk + NPC beats

## A. SCENE SETUP (zone master `0x7FFFFFF0` event 0 @1) — once, before anything is visible

| # | Ask | Instruction |
|---|---|---|
| A1 | Clear the event-hide flag on the player | `EVENT_HIDE_SELF(0)` @+00001 |
| A2 | **SNAP the player to the walk's START (NW)** | `SET_EVENT_POS` @+00003 → refs[0..3] = (-365250, -184858, -10501, 3584) = wire (-365.25, ground -184.858, height -10.501), heading 3584 (wire 224) |
| A3 | End the master program | `END` @+0000C |

The master's program is just these three opcodes; everything else runs as owner children + REQSET tags.

## B. NARRATION BEATS (narration block `0x010EB001` event 0 @1, 467 opcodes to +00D51)

Pattern per beat: camera routine + fade → NARRATION (auto-advance) → camera holds to full length.
All 9 narration lines are auto-advance (`7F 34 NN` continue-prompt codes), seconds `[5,7,7,5,7,7,6,5,5]`.

| # | Ask | Instruction |
|---|---|---|
| B0 | Prologue | `HIDE_HUD` (0x67), `CANCEL_DISARM` (0x42), `DEFCAMERA case1` (0x46), `LOCAL_MODE` (0x38), `LOADEVENTSCHEDULER2 fdo2+s000` (0x45), `STOP_CLOCK` (0x77) |
| B1 | **WALK 1 starts** (before the first line) | `REQSET tag 3` on the zone block @+00036 |
| B2 | Line 1 (auto 5 s) | *"In the Gustaberg Mountains of Southern Quon lies the industr…"* camera s001, `MESWAIT` |
| B3 | Line 2 (auto 7 s) | *"Compared to San d'Oria, with its legendary orders of knights…"* camera s002, `MESWAIT` |
| B4 | Line 3 (auto 7 s) | *"But advanced technology and near limitless underground resou…"* camera s003, `MESWAIT` |
| B5 | Line 4 (auto 5 s) | *"Innovative and adaptable Humes make up more than half the po…"* `MESWAIT` |
| B6 | Line 5 (auto 7 s) | *"With their flair for industry, economy, and politics, they w…"* `MESWAIT` |
| B7 | Line 6 (auto 7 s) | *"The Galka, a race of hulking humanoids, comprise a third of…"* `MESWAIT` |
| B8 | Line 7 (auto 6 s) | *"Without their incomparable strength, the countless ore mines…"* `MESWAIT` |
| B9 | Line 8 (auto 5 s) | *"Adventurers seeking fame and fortune flock to rising nations…"* `MESWAIT` |
| B10 | Line 9 (auto 5 s) | *"Your story begins here, amid the smoke and bustle of the ind…"* `MESWAIT` |
| B11 | **WALK 2** | `REQSET tag 4` @+000A8 |
| B12 | **WALK 3** | `REQSET tag 5` @+00154 |
| B13 | NPC beats | many `REQSET` on 0x010EB033/25/29/2A (tag2), 0x010EB006/07 (tags 3–8), 0x010EB005, 0x010EB00B/0A, 0x010EB016; NPC dialogue `0x2B` + gestures `0x5B`/`0x66` (tlk0/thk1/thk2/dis0/tlb0); s138–s141 cameras; Reet (0x010EB043) lines |
| B14 | **WALK 4** | `REQSET tag 6` @+0059D |
| B15 | Release | `fdo1`, s141, `DEFCAMERA case0` (release camera), `fdi2`, `EXECEND` (0x21) |

## C. WALK SEGMENTS (zone master `0x7FFFFFF0` tags, REQSET'd by the narration block)

The walk path: character starts NW at (-365.25, ground -184.86) and walks SE to (-281.55, ground -98.24).
Speed 13 (1.3 yalms/s) throughout. Walk 1 has **no** snap — it starts from the current (snapped) position.
Walks 2–4 carry explicit `SET_EVENT_POS` snaps.

| # | Ask | Tag @ offset | Instruction |
|---|---|---|---|
| C1 | **WALK 1** (NW → mid) | tag 3 @14 | `SPEED=13` (ref4) + `MOVE case0` goal refs[5,6,7]=(-356.714, -10.0, -176.372) + `MOVE case1` wait + END |
| C2 | **WALK 2** (mid → mid) | tag 4 @28 | `SPEED` + `SET_EVENT_POS` snap refs[8..11]=(-336.059, -10.003, -156.09) + `MOVE case0` goal refs[12,13,10]=(-319.076, -10.0, -139.082) + `MOVE case1` wait + END |
| C3 | **WALK 3** (mid → scene) | tag 5 @51 | `SPEED` + `SET_EVENT_POS` snap refs[14..17]=(-285.931, -10.0, -109.684) + `MOVE case0` goal refs[18,19,20]=(-281.546, -12.0, -98.238) + `MOVE case1` wait + END |
| C4 | **WALK 4 / arrival** | tag 6 @74 | `SET_EVENT_POS` snap refs[21..24]=(-279.816, -12.136, -91.989) heading 245 + `LOOK_AND_TALK` + … |
| C5 | Arrival sequence | tags 0x32–0x35 (50–53) | later arrival-sequence programs (REQSET'd at +009FE/+00AED/+00AF7/+00C3B with priority 5) |

## D. TAIL (end of event 0, after the last narration)

| # | Ask | Instruction |
|---|---|---|
| D1 | Settle | `WAIT 0.5 s` |
| D2 | Hold the final camera | s004 camera (`LOADEVENTSCHEDULER2` + `WAITLOADSCHEDULER`) |
| D3 | Fade out | fdo2 fade (`LOADEVENTSCHEDULER2` + `WAITLOADSCHEDULER`) |
| D4 | Hold on black | `WAIT 2 s` |
| D5 | End event 0 | `EXECEND` → c2s 0x05B EVENT_END; server chains event 7 |

---

# EVENT 7 — NPC cutscene (Reet + galka kids)

The zone master has only a wildcard (0xFFFE) entry @13 = END. The scene is driven by
`0x010EB001` event 7 @420; the other owner blocks (0x010EB005/06/07/0A/0B/3C) are small programs
(hide/show, `SET_EVENT_POS`).

| # | Ask | Instruction |
|---|---|---|
| E1 | Prologue | `DEFCAMERA case1`, fdo0, `CLOCK_HOLD`, `LOCAL_MODE`, `HIDE_HUD` show, `ACTOR_HIDE(player, show)` |
| E2 | **PLACE the player at the walk's END** | `SET_ACTOR_POS` (0xBA) on the local player → (-281.546, -12.0, -98.238) heading 3086 |
| E3 | Camera + fade | s003, fdo1 |
| E4 | Dialogue (manual `7F 31` prompts) | *"Hey, Gwill! Is it true? You really saw Captain Volker!?"* / *"Are you really, really sure? We're talking about the captain…"* / *"I'm telling you, I'm sure! I saw him walking toward the Mine…"* / *"Yeah, right! Like you'd ever know!"* — s004–s006 on player; ExtScheduler gestures on 0x010EB00A/0B (tlk0/ten0/tlk1/ten1) |
| E5 | Release | `DEFCAMERA case0`, `END` |

---

# CODED / VERIFIED STATUS

Legend: **DONE** = coded + verified (unit test and/or live) · **CODED** = coded, not yet live-verified ·
**KNOWN-ISSUE** = observed defect, not yet root-caused/fixed · **NOT-STARTED** = no code yet.

## Fixes landed this round

| Fix | What it does | Coded in | Verified | Status |
|---|---|---|---|---|
| **Liveness: walk is not a stall** | The liveness tuple gains a move-units term, so a player walk parked on its `MOVE case1` hold drops its remaining units every tick and runs to its goal instead of stall-cancelling after the 5 s hold grace. | `EventVm::move_units_remaining` + `Scene::motion_units_remaining` + `scene_move_units` (ffxi-event vm.rs / vm/scene.rs); `DialogRunner::move_units_remaining` (runner.rs); `check_liveness` tuple `(park, ep, wait, move, since)` (kuluu-session event_dialog.rs) | unit test `a_walk_longer_than_the_hold_grace_runs_to_its_goal_instead_of_stalling` (a 10 s walk past the 5 s grace finishes, no cancel) | **DONE** |
| **Walk starts from the snap** | The local player's position is one shared cell across every VM in the event. The zone block's `SET_EVENT_POS` snap writes it; a walk child REQSET'd after the snap reads it, so the walk starts from the snapped NW position instead of a sibling's zeroed origin. | `shared_player: Option<Arc<Mutex<EventPosition>>>` on `EventVm`; seeded in `attach_scene` (zone block), shared to children in `spawn_owner`/`push_request`, written by the 0x37/0x39 player arms | unit tests `set_event_pos_on_the_player_moves_the_shared_cell_a_walk_child_reads` + `a_reqset_walk_child_starts_from_the_snapped_shared_position`; **live: user confirmed "the walk direction is correct"** | **DONE** |
| **CS feet ground on the MZB floor** | The dialog-driven (CS) local player is pinned to the scripted position, which can sit slightly below the authored MZB floor; the feet now lift to the floor when it is above them (wire z grows down), never snapping down, so they do not clip into the floor. | `ground_dialog_feet_wire_z` + `DIALOG_FLOOR_SNAP_EPSILON` (kuluu input.rs); the dialog-driven branch of `dispatch_movement_system` grounds the lerped feet on the MZB collision when the zone's MZB is ready | unit test `dialog_feet_lift_to_the_floor_when_the_scripted_height_sits_below_it` (lifts when below, holds when on, refuses to snap down when above) | **CODED** |

## Per-ask status (event 0)

| Ask | How coded | How verified | Status |
|---|---|---|---|
| A1 clear event-hide | `OP_EVENT_HIDE_SELF` arm emits `ActorHide{EVENT_ENTITY, hide:false}` | existing cue test | DONE |
| A2 snap to NW start | `OP_SET_EVENT_POS` player arm sets `scene.player` + shared cell, publishes `PlayerPosition` (renderer snap + c2s POS) | unit test `set_event_pos_on_the_player_snaps_the_tracked_position` + live position trace (snap at t+2.9 s) | DONE |
| B0 prologue (HUD/camera/clock) | 0x67/0x42/0x46/0x38/0x45/0x77 arms | event 503 parity (same opcodes) | CODED |
| B1–B10 narration auto-advance | `7F 34 NN` continue-prompt codes → `DialogSession` countdown on the session clock | live: all 9 frames auto-advanced with zero input (gaps ≥ governing seconds) | DONE |
| B11–B14 walk REQSETs | `OP_REQSET` pushes the tag program onto the zone-block stack; the walk child runs the player-branch `MOVE` lerp | live: walks 1–3 now start from the snapped position (shared-cell fix) | DONE |
| C1–C4 walk segments | `OP_SPEED` + `OP_MOVE` player arm (scene.motion lerp + `PlayerPosition` actions) | live: walk direction correct; unit test pins the lerp start | DONE |
| C5 arrival tags 50–53 | REQSET'd arrival-sequence programs | not traced | NOT-STARTED |
| B13 NPC beats (REQSET + dialogue + gestures) | REQSET/REQWAIT + 0x2B/0x5B/0x66 arms | not traced | NOT-STARTED |
| D1–D5 tail (WAIT/camera/fade/EXECEND) | WAIT + scheduler holds + EXECEND | **stalls after the fade-out in the live run (see KNOWN-ISSUE K2)** | KNOWN-ISSUE |

## Per-ask status (event 7)

| Ask | How coded | How verified | Status |
|---|---|---|---|
| E1 prologue | 0x46/0x77/0x38/0x67/0x4E arms | not traced | NOT-STARTED |
| E2 place player at walk end | `SET_ACTOR_POS` (0xBA) player arm | not traced | NOT-STARTED |
| E3 camera + fade | 0x45/0x55 scheduler holds | not traced | NOT-STARTED |
| E4 dialogue (manual) | 0x2B + MESWAIT (manual `7F 31`) | live: event 7 held manual, advanced by hand (prior run) | CODED |
| E5 release | 0x46 case0 + END | not traced | NOT-STARTED |

## KNOWN-ISSUES (observed in the live run, not yet root-caused/fixed)

| # | Issue | Observation | Proposed direction |
|---|---|---|---|
| **K1** | **Feet in the floor** | The snap/walk height (wire z ≈ -10.5) sits slightly below the actual ground, so the character clips into the floor. | **FIXED (code)** — the dialog-driven feet now ground on the MZB floor (`ground_dialog_feet_wire_z`); see the "CS feet ground on the MZB floor" row above. Awaiting live confirmation. |
| **K2** | **Stall after the fade-out** | Event 0 plays to the fdo2 fade, then parks and the liveness watchdog cancels it with an error line. | Likely the D2 s004 camera / D3 fdo2 scheduler hold not releasing (renderer finish report) or the D4 WAIT 2 s; trace the tail's holds. |
| **K3** | **`???` speaker names + missing NPCs** | After the K2 stall, dialogue shows `???` speakers and NPCs are missing. | Consequence of the stall leaving the event in a broken state; resolve K2 first, then re-check speaker attribution + NPC spawn. |
| **K4** | **ESC not locked out** | After the K2 stall, ESC cancels the event (should be disarmed by the B0 `CANCEL_DISARM`). | Consequence of the stall; re-check `cancel_armed` after K2 is resolved. |

## Notes

- The uncommitted general CS-reader timer fix (VM reads motion DATs itself via
  `set_dat_root`/`routine_length`; session `arm_motion_holds` uses `runner.routine_length`) is still in the
  tree and is **separate** from the two fixes above; it touches the same files (vm.rs / runner.rs /
  vm/scene.rs / event_dialog.rs).
- Live evidence (prior green run, pre-fix): `artifacts/verify/bastok_intro_events.jsonl` +
  `bastok_intro_summary.txt` — event 0 auto-advanced all 9 frames; the position trace showed the walk
  starting from the world origin (the bug the shared-cell fix removes).
- No live testing was done this round beyond the user's manual run; K1–K4 are from that run and are
  not yet reproduced/root-caused in code.
