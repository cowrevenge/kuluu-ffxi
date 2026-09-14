# FFXI MOB ANIMATION - State of the Port

What mob animation looks like in Kuluu today, with the special-pose mechanism (retail's
sub->routine table) as the worked example: the tunnel worm's dig/pop-up cycle is what that
mechanism does on a model that ships `ini1`/`init`. Written from the tree at
`bionic/entity-table`; wire facts verified against LSB (`vendor/server`) first, retail DAT second.

## 1. Status (playtest-confirmed unless noted)

| Piece | State |
|---|---|
| Idle / walk / run / battle stance from `animation` byte + movement | Working (pre-existing) |
| Skill/action clips (action DATs, layered over pose set) | Working (pre-existing) |
| Fishing clip resolution (`fsh<n>` routine → real motion clip) | Working (pre-existing) |
| Special-pose override (sub->routine table, FFXiMain.dll F37/F44): worm dig = `ini1` -> `sp1?` one-shot + hold buried end-frame; pop-up = resurface fires `init` -> `sp0?` | Committed (Phase 2); **first post-fix playtest pending** (§5 documents the earlier clip-mapping correction) |
| Effect routines (`ini1` dig / `init` pop: dirt particles + sounds 7025/7024) | Wired via `effects_only`; fires on trigger; **no early cancel** (retail lets it finish, §4.1) |
| Interruption handling (server clears sub mid-effect -> pose settles to locomotion) | Pose settles; dirt/sound routine runs out like retail (§5) |
| Unflushed-invisibility workaround | Removed earlier: the pop-up's explicit POS carries status=3 and drives the state correctly (§6.1) |
| Frozen-mob fix (fall-through when a selected clip does not resolve, §10.5) | Committed; **first post-fix playtest pending** |
| Target-chase off the LSB step model: step-relative snap bands, chase paced to dt_server, Y server-resolved (§10.2) + wire gait (§10.3) | Committed (A/B); MOTION_UPD `band=` distribution is the regression guard |
| Crit/heavy reaction falls back to damg when ldam does not resolve; no attacker crit swing on purpose (§10.6) | Committed (C); rabbit_tester S6c/S6d cover both victim kinds' matrix |
| 0x45 Info chunk interpreted: movement_type gates the stride scale, model scale byte applied to NPC spawns, named-but-unparsed chunk kinds (§10.7) | Committed (D); bat loads live at 0.85/Flying, 100-scale walker unchanged |

## 2. Architecture: how a mob's animation is chosen each frame

All of it lives in the pose pass (`advance_actor_pose`, [kuluu-render/src/ffxi_actor_render.rs](../kuluu-render/src/ffxi_actor_render.rs) ~L1950+), driven by `ActorAnimInputs`
([ffxi-actor/src/actor_state.rs](../ffxi-actor/src/actor_state.rs)). Clip selection precedence:

```
action_id (skill/action DAT clip, localDir wins over actor pose set)
  > engage_overlay (battle stance overlay clips)
    > fishing (fsh<n> routine's motion clip; loops or holds per phase)
      > special (the wire's animationsub names a routine; its first Motion stage clip, §3)
        > rest phase (sit/kneel in-loop-out, one-shot hold on ends)
          > selected_animation (idle/walk/run from movement + animation byte)
```

- **Clips come from the model DAT** (`ROM/5/<id>.DAT` → parsed routines + `action_assets`). No
  per-mob custom action code: we play whatever the DAT defines. This is a hard project rule:
  retail's effect-routine names (`init`, `ini1`, …) are data, not code paths.
- **One-shot clips** (special-pose motion, rest in/out) hold their last frame instead of looping
  or falling back to idle: they register with num_loops = 1 and the coordinator clamps at the end
  frame while the wire state stays up. Nothing hides a model on clip completion (§3.4).
- **Effect routines** (particles/sound, no motion) run on a separate track: an
  `ActiveScheduler` component on the entity's *wire* entity, flattened with
  `MotionStages::Suppress` (`effects_only`, [kuluu-render/src/scheduler_runtime.rs](../kuluu-render/src/scheduler_runtime.rs)).
  Stage events spawn dirt SEPs / play sounds. The slot is shared with melee-swing/cast/emote
  dispatches (all `try_insert`); a lingering scheduler silently blocks the next one, which is why
  cancellation on early exit matters (§4.3).

## 3. Special-pose mechanism (worked example: tunnel worm)

Retail plays named routines from the model DAT off two wire triggers; kuluu ports both and lets
the DAT decide what any of them does on a given model. The tunnel worm's dig/pop-up cycle is the
canonical case: its ROM/5/64.DAT ships `ini1` (Motion sp1? + dirt generators + sound 7025) and
`init` (Motion sp0? + dirt generators + sound 7024).

### 3.1 Wire facts (verified against LSB + live packets)

- Entity updates are s2c `0x0E` CHAR_NPC; every update carries, unconditionally:
  - `status`: LSB offset 0x20 -> body[0x1C] (`NpcState::decode_char_npc_status`,
    [ffxi-proto/src/decode/entity.rs](../ffxi-proto/src/decode/entity.rs) ~L657). Valid on every
    update, including POS-only ticks. **Mobs serialize their raw status value**: a visible mob's
    resting state is `UPDATE=1` (constructor, baseentity.cpp L73), so visible mob ticks carry 1;
    only *players* get UPDATE rewritten to NORMAL(0) on the wire (entity_update.cpp ~L326).
  - `animationsub`: LSB 0x2A -> body[0x26], General-block only (UPDATE_HP send-flag 0x04). On
    spawn LSB ORs in a **spawn flag 0x04**. The client does not mask it: retail's sub->routine
    table wraps mod-4, so the raw byte indexes it directly and the flag is absorbed (F47).
    POS-only ticks don't carry the byte; the last known value persists.
- `STATUS_TYPE` ([vendor/server/src/map/entities/baseentity.h](../vendor/server/src/map/entities/baseentity.h) L47):
  NORMAL=0, UPDATE=1 (visible), DISAPPEAR=2, INVISIBLE=3.

### 3.2 What the server does to a worm (LSB `ROAMFLAG_WORM`, [mob_controller.cpp](../vendor/server/src/map/ai/controllers/mob_controller.cpp))

| t | Server action | Wire effect |
|---|---|---|
| dig start (~L1182) | `animationsub = 1`, `HideName(true)`, `SetUntargetable(true)`; both set updatemask | UPDATE_HP: visible, sub=1 -> **sub-change trigger fires `ini1`** |
| ~3s later (QueueAction, L1195) | `status = INVISIBLE` (**raw assignment, no updatemask**, §6.1) | *nothing, until some other update carries the byte* |
| while buried | may roam a new path underground (`RoamAround`, navmesh zones only); POS updates carry status=3 | hidden (if packets flow); nothing triggers while hidden |
| pop start (~L1278) | explicit `UPDATE_POS` (**carries the still-INVISIBLE status byte**; this is what lands us in the hidden state even when §6.1's flush never happened), then `status = UPDATE`, `SetUntargetable(false)` -> UPDATE_HP within 250ms | visible, sub still 1 -> **resurface trigger fires `init`** |
| +2s (QueueAction, L1287) | `animationsub = 0`, `HideName(false)`; fires even mid-battle (`ActionQueue.checkAction` runs every tick) | visible, sub=0 -> pose settles to locomotion; the dirt/sound routine keeps running out (retail semantics, §4.1) |

Mid-dig interruption is impossible in core C++ (untargetable rejects all attack/spell/ability
paths); the only reachable interruption is engaging during the 2s pop-up window.

### 3.3 The two triggers and the wire-state transition (`ffxi-actor/src/actor_state.rs`)

Retail's dispatch (FFXiMain.dll, [ffxi_disassembly.md](disassmembly_docs/ffxi_disassembly.md) §9):

1. **Sub change while the actor is visible:** a change detector plays `table[sub]` on the model,
   where table = [init, ini1, ini2, ini3] with a mod-4 wrap over the raw 3-bit sub (F37 @RVA
   0x35AF60). Sub 0 and spawn-flagged 0 name no active special. A model that does not ship the
   named routine gets nothing: the name is data, the DAT decides (F44's named-play slots).
2. **Hidden -> visible transition:** retail destroys the actor on `status = INVISIBLE` and
   constructs a fresh model when it becomes visible again; the create path runs the load routine
   `init` (F53: leaving and re-entering view range is the same destroy/create).

kuluu keeps one hidden actor instead of destroying/rebuilding, so `hidden` stands in for "actor
destroyed". The state is `SpecialPose { sub, hidden, active_routine }`, advanced by pure
`next_special_pose(prev, status, animationsub)` on every snapshot change; it returns the new
state plus the routine to fire this frame:

```
prev hidden -> visible now                          : fire 'init' (the create path wins over any same-frame sub change)
visible + sub changed                               : fire table[sub] (ini1/ini2/ini3), or settle when the byte names no routine
visible + active_routine set + sub settled to zero  : settle back to locomotion
otherwise                                           : hold, nothing fires
```

While hidden nothing triggers: retail has no live actor to run it on; the resurface replays
'init' instead of re-firing the old special. A sub-clear arriving mid-routine does nothing in
retail (the routine finishes), so there is no early-cancel path here either.

Tests in `cargo test -p ffxi-actor`: `special_routine_table_matches_retail` (F37's table verbatim,
including the mod-4 wrap that absorbs the spawn flag) and `special_pose_full_worm_cycle`
(dig -> buried -> resurface fires init, not ini1 -> settle).

### 3.4 Render-side driving (`tick_live_ffxi_actors`)

- Per-entity state memory: `Local<HashMap<u32, SpecialPose>>`. An entity absent from last frame's
  snapshot is modeled as hidden (retail had no live actor for it), so its first visible observation
  takes the resurface path and fires 'init' (F53). Only ~16 model DATs game-wide ship an `init`
  with a Motion stage, so zone-in init firing is quiet in practice; worm-family models pop up on
  spawn, faithful to retail.
- Triggers queue effect routines on the wire entity (`ini1` on dig start, `init` on resurface;
  `effects_only`, motion stages suppressed because the pose pass owns the routine's motion clip).
  **No early-cancel path**: retail ignores a sub-clear arriving mid-routine and lets it finish
  (F19-F22). The shared-slot consequence (a lingering dirt/sound scheduler can delay the worm's
  first combat swing sound until the routine completes) is retail behavior, not a bug.
- Model-root Visibility is written here **only** when `status == INVISIBLE` hides the entity;
  nothing else writes it. A completed one-shot holds its end frame pinned in the coordinator,
  and retail never hides a model on clip completion (a worm's buried dig pose is occluded by
  terrain exactly as there). Every other root stays owned by `scene::apply_invis_flag_system`.

## 4. Known-good decisions (don't re-litigate)

1. **Retail semantics (FFXiMain.dll findings F19-F22 + F37/F44/F53; see [ffxi_disassembly.md](disassmembly_docs/ffxi_disassembly.md) §9):**
   - Dig = server sets sub=1 on a *live* actor → client runs the DAT's `ini1` routine: Motion
     `sp1?` (sinks, ends underground and **holds**; no status byte needed to hide), dirt
     generators, sound. The clip-hold *is* the burial.
   - `status = INVISIBLE(3)` → client **destroys the actor**. Optional as far as visuals go;
     nothing ever hides a model on clip completion.
   - Pop-up = visible status on an entity with **no actor** → client constructs a fresh model and
     runs the load routine `init`: Motion `sp0?` (rises), dirt generators, sound. The pop is a
     fresh model load, every time (F53: view-range re-entry is the same destroy/create).
   - A sub-clear arriving mid-routine does nothing; the routine finishes.
2. Our port keeps **one hidden actor** instead of destroy/rebuild: `hidden` stands in for "actor
   destroyed", and the one-shot's pinned end frame stands in for retail's clip-hold. Both effect
   routines fire on the same entity (retail fires them on different model instances).
3. One-shot hold both directions (dig holds buried; pop holds emerged) until the wire says otherwise.
4. No per-mob code: DAT defines the clips/routines, packets drive the state. The special tier
   applies only when the model actually ships a usable chunk for the named routine's motion clip.

## 5. The back-to-back bug: real root cause (corrected this round)

**Symptom:** worm digs down, then immediately pops straight back up with no buried gap; model
never actually invisible during burial.

**Root cause: the clip mapping was reversed.** We had DigDown→`sp0?`, PopUp→`sp1?`. But in the
DAT (verified by re-running `dat-burrow-dump.rs` against retail ROM/5/64.DAT, §8) **`sp00` is the
pop-up clip** (joints rise from buried to surface pose; 75 frames ≈ 3s) and **`sp10` is the
dig-down clip** (joints sink; 50 frames ≈ 2s). So our "dig" was playing the *rising* motion;
the worm visibly came back up a second after spawn/dig start, then status=3 hid it, and our
"pop-up" played the *sinking* motion. The whole cycle looked inverted/back-to-back.

The earlier §5 theory (LSB's unflushed INVISIBLE causing an instant Underground→PopUp) was a red
herring for the visual symptom: even when status=3 never flushes on its own, holding `sp10`'s
buried end frame *is* visually underground; no timer needed. The state machine still reaches the
hidden state
correctly because the pop-up's explicit POS carries the still-INVISIBLE byte (§3.2), ~250ms before
the status flip.

**Fix (this round):**
- `burrow_clip`: DigDown→`sp1?`, PopUp→`sp0?` (matches retail F19-F22 and the DAT).
- Routine firing swapped to match: dig start fires `ini1` (the DAT's dig routine), pop-up fires
  `init` (the DAT's load/pop routine); previously both were backwards.
- Removed the round-9 early-cancel on `(DigDown|PopUp) → None`: retail ignores a mid-routine
  sub-clear and lets the routine finish (§4.1).
- Removed `BURROW_DIG_INVIS_TIMEOUT_SECS` (the 4s timeout-hide): it emulated no client behavior.

**Note:** the "down then up for one frame" observed on *retail* is an LSB artifact, not a retail
client quirk; confirmed and noted in F19-F22.

## 6. LSB server quirks (documented, not fixed on the server)

1. **Unflushed INVISIBLE**: raw `status` assignment in the dig QueueAction lambda has no
   updatemask; PostTick only flushes when updatemask≠0. No client workaround needed: during the
   gap we hold the buried end frame (visually underground), and the pop-up's explicit POS carries
   status=3, which drives DigDown→Underground before the status flip arrives.
2. **No-navmesh zones:** `RoamAround` returns false for worms ("no point worm roaming cause it'll
   move one inch"); buried worms there never start a path, so they sit still (no packets) until
   whatever triggers pop-up; covered by the clip-hold above.

## 7. Diagnostics: `KULUU_SPECIAL_LOG=1`

Same switch gates both layers (`$env:KULUU_SPECIAL_LOG="1"` before launch):

| Line | Layer | Meaning |
|---|---|---|
| `wire id=… send_flag=0x.. status=.. sub=..` | session ([session/mod.rs](../kuluu-session/src/session/mod.rs)) | raw CHAR_NPC update that could drive a transition (status==3 or masked-sub≠0 on Mob/Pet). **This is the ground truth for "what did the server send".** |
| `fsm id=… prev=… new=… triggered=… status=… sub=…` | render | an actual wire-state change, with the routine that fired (if any) |
| pose-selection / hold-probe lines | render | clip chosen + pinned-frame sampling while a special state is up (`done` reports whether the routine's motion clip has played to its end frame) |

Reading one worm cycle: dig start = `wire … status=1 sub=5` (spawn flag) or `sub=1` (mobs carry
raw status, so visible ticks are 1); buried window = silence (no packets; the buried pose holds
pinned and the model hides on status when it arrives); pop-up = a POS carrying `status=3`, then
~250ms later `status=1 sub=1` (resurface: sp0? + `init` dirt/sound), and ~2s after that `sub=0`
(settle; pose returns to locomotion, routine runs out).

CLIP_WARN is on by default regardless of this switch (§10.4); KULUU_CLIP_LOG=1 adds CLIP_OK, and
KULUU_MOTION_LOG=1 logs the chase model's per-POS-update probe (§10.2).

## 8. Key files

| What | Where |
|---|---|
| Special-pose state machine + F37 table + tests | [ffxi-actor/src/actor_state.rs](../ffxi-actor/src/actor_state.rs) (`SpecialPose`, `SPECIAL_ROUTINE_TABLE`, `next_special_pose` ~L40, tests near the bottom of the file) |
| Pose pass (precedence, fall-through, one-shot hold, visibility write) | [kuluu-render/src/ffxi_actor_render.rs](../kuluu-render/src/ffxi_actor_render.rs) (`advance_actor_pose` ~L2140+, `tick_live_ffxi_actors` ~L3200+) |
| Target-chase prediction + gait probe | [kuluu-render/src/combat_stance.rs](../kuluu-render/src/combat_stance.rs) (`EntityPrediction`, `advance_prediction`, `track_entity_motion_system`) |
| 0x45 Info chunk parse + MovementType/RangeType/WeaponAnimStyle + scale_factor | [ffxi-dat/src/cib.rs](../ffxi-dat/src/cib.rs) (viewer-cited; bat raw-bytes test) |
| Speed decode constants (SPEED_TO_YPS, mount doubling, clamp, AUTHORED_ANIM_RATE, anim_rate_scale) | [kuluu-snapshot/src/lib.rs](../kuluu-snapshot/src/lib.rs) (`pub mod speed`) |
| Effect scheduler (ActiveScheduler, effects_only, natural completion) | [kuluu-render/src/scheduler_runtime.rs](../kuluu-render/src/scheduler_runtime.rs) |
| status/sub decode + spawn flag docs | [ffxi-proto/src/decode/entity.rs](../ffxi-proto/src/decode/entity.rs) (`NpcState` ~L603+) |
| Wire probe log | [kuluu-session/src/session/mod.rs](../kuluu-session/src/session/mod.rs) (`special_wire_log_enabled`, in `handle_sub_packet`) |
| LSB worm roam/dig/pop | [vendor/server/src/map/ai/controllers/mob_controller.cpp](../vendor/server/src/map/ai/controllers/mob_controller.cpp) (dig ~L1182, pop ~L1278) |
| Any-model DAT dump | `cow_tools/ffxi_disasm/dat_routines.py` (standalone Python): chunk table + every scheduler routine as stages (motion/sound/vfx/call) for one DAT or a whole install (`--scan`, `--csv`). Verified on 3 mob/PC DATs; every model ships the same ~36 standard routines (init, pop0, dead, corp, atk0, ati0-2, atf0, ldam/damg/sdam, gurd/pary/shld, sway, cate, cast*, shot, sh??, v???) plus its specials. See [ffxi_disassembly.md](disassmembly_docs/ffxi_disassembly.md) §9 F54 |
| Worm DAT probe | `ffxi-dat/examples/dat-burrow-dump.rs`: re-run this round against retail ROM/5/64.DAT (PhoenixXI): clips **`sp10` = dig-down** (joints sink, 50 frames ≈ 2s), **`sp00` = pop-up** (joints rise, 75 frames ≈ 3s); routine **`ini1` (dig)** = Motion sp1? + kak0/mok0/mok1/dis0 + sound **7025**, ~112 stage-frames; routine **`init` (pop)** = Motion sp0? + dis0/mok1/kak1/kak0/mok0 + sound **7024**, ~188 stage-frames. Both carry an `Unknown(0x5F)` cross-reference to the other; **now understood (F46): the 0x5F stage holds the sibling routine's name, and the client's parsed record keeps it as a 4-dword stage**. Runtime confirmation of this whole row: [ffxi_disassembly.md](disassmembly_docs/ffxi_disassembly.md) §9 F46 |

## 9. Open items / housekeeping

- [ ] **First post-fix playtest (West Sarutabaruta):** every roaming mob animates walk/run instead
      of sliding frozen in its spawn pose; stops land exactly with no slide-back; engaged mobs flip to
      run the tick the chase starts; worm cycle = dig sp1? + `ini1` dirt/sound, hold buried through the
      silent window, resurface sp0? + `init` dirt/sound. Log with KULUU_SPECIAL_LOG=1
      KULUU_MOTION_LOG=1 KULUU_CLIP_LOG=1; CLIP_WARN lines are the regression guard (§10.4).
- [x] **Sound attribution check: resolved (F46, 2026-09-08).** The DAT was right: dig = `ini1` =
      7025, pop = `init` = 7024. F22 had read the routine record's 0x5F cross-reference (the
      *sibling's* name) as the record's own name, so its "contents are swapped" claim is withdrawn.
      Both runtime records were dumped at dispatch and match this file's §8 byte for byte.
- [ ] **Library resolution (F56/F58):** `run_routine(name)` resolves routine dir -> model DAT root ->
      target's DAT (0x09 calls) -> ROM/0/0.DAT -> warn once and no-op. No built-ins: `hwat aloc mloc
      dada wash waso proc dcnt` are dead references (xim and PS2 both no-op them).
- [ ] **Driver rules (F58, from xim `Actor.kt`):** melee = `atk0` (voice, non-blocking) + swing
      (random `ati0..N` standing, `atf0/atl0/atr0/atb0` moving); TP move = load effect DAT by
      `anim + 0x0F3C` FTABLE index, run its `main`; `cate` on start; `in<n>`/`out<n>` engage; `dead`
      or `dea<state>`; target reaction from `resolution`/`hitDistortion`.
- [x] **Generalize clip selection (F46/F47/F49): done in Phase 2.** The pose pass takes the motion
      clip from the routine record's first Motion stage (`routine_motion_clip`) instead of a hard-coded
      mapping; the sub->routine table is indexed with the raw byte and its mod-4 wrap absorbs the spawn
      flag (§3.3). Remaining: plan the action-packet path for `dam`/`atk`/`hit` routines. Details in
      [ffxi_disassembly.md](disassmembly_docs/ffxi_disassembly.md) §9 F46-F49.
- [ ] Decide: keep `ffxi-dat/examples/dat-burrow-dump.rs` (proven useful this round; suggest keep);
      delete `rustc-ice-*.txt` junk.
- [x] Rounds 6-9 + Phase 1/2 committed locally on bionic/entity-table: A=CLIP_WARN instrumentation,
      B=frozen-mob fall-through, C=target-chase, D=wire gait, E=special-pose mechanism. Not pushed;
      push only when asked.

## 10. Locomotion (target-chase + wire gait)

How a mob's position and walk/run animation are driven off the `0x0E` POS block, replacing the old
dead-reckoning velocity model. Wire facts first, then the two rules that fall out of them.

### 10.1 The wire (what retail decodes)

- `0x0E` CHAR_NPC POS block ([vendor/server/src/map/packets/entity_update.cpp](../vendor/server/src/map/packets/entity_update.cpp)):
  `moving` u16 at 0x18, `speed` u8 at 0x1C (the MovementSpeed2 source), `animationSpeed` u8 at 0x1D.
- Retail's decode (research/XiPackets world/server/0x000E): the client walks `LocalPosition`
  toward the POS target at `MovementSpeed2 = Speed * 0.1` yps and **stops on arrival**; there is no
  velocity extrapolation. `AnimationSpeed = SpeedBase * 0.1` scales walk/run clip playback only; it
  never feeds movement.
- The two bytes are independent by construction: LSB's `UpdateSpeed(run)` multiplies `speed` only
  ([vendor/server/src/map/entities/battleentity.cpp](../vendor/server/src/map/entities/battleentity.cpp),
  `CBattleEntity::UpdateSpeed`: roam calls `updateSpeed(false)`, chase calls `updateSpeed(true)` with
  the run multiplier from `map.MOB_RUN_SPEED_MULTIPLIER` or a per-mob `MobMod`). `animationSpeed`
  is never multiplied.

### 10.2 Target-chase off the LSB step model (A + B)

The wire is a discrete step stream, not a velocity stream: per AI tick a moving mob advances
exactly one step and emits exactly one POS update.

- `CPathFind::StepTo` ([vendor/server/src/map/ai/helpers/pathfind.cpp](../vendor/server/src/map/ai/helpers/pathfind.cpp):383):
  `stepDistance = speed / (run ? 50 : 40)` yalms (:399), advanced once per AI tick; the entity emits one POS
  update per tick, which is what MOTION_UPD's dt_server measures (~400 ms cadence).
- The run flag comes from `PATHFLAG_RUN`, which [mob_controller.cpp](../vendor/server/src/map/ai/controllers/mob_controller.cpp)
  sets for chase (:872/:909), follow (:1055) and the return-home fallback (:1117); roaming's
  `RoamAround` (:1199) passes no run flag: roam = walk (/40), engaged = run (/50).
- `CBattleEntity::UpdateSpeed(run)` multiplies only the movement speed when running (the
  `map.MOB_RUN_SPEED_MULTIPLIER` block, [battleentity.cpp](../vendor/server/src/map/entities/battleentity.cpp):322+); it never touches
  `animationSpeed`. So the run/walk split is read straight off the wire as `speed > speed_base`,
  the same comparison the gait rule uses.
- Y is fully server-resolved inside StepTo: on arrival the position snaps exactly to the target
  (:412-417), otherwise XZ advances by the remainder and Y walks toward target.y along the slope,
  clamped between start and end vertical positions (:426-435). The client assigns
  `rendered.y = server_pos.y` directly on every update (combat_stance.rs:1058): no smoothing, no ground probe,
  no gravity. Y is excluded from the jump metric so a floor-height change cannot inflate it.

On each POS update the prediction stores target position, heading, speed byte and speed_base byte;
it keeps no velocity state. (MovTime = Flags0 & 0x1FFF is decoded by `PosHead::mov_time()` as a
reserved accessor for the follow-up that phases the walk/run cycle off its delta; it is
deliberately not threaded through the snapshot yet.)

**The step.** `expected_step_yalms(speed, speed_base)` = `speed / (run ? 50.0 : 40.0)` with
`run := speed > speed_base` (combat_stance.rs:992). The two divisors are LSB constants named in the
code as `LSB_RUN_STEP_DIVISOR` / `LSB_WALK_STEP_DIVISOR` (combat_stance.rs:905/907, cited to
StepToInternal); they are the only literals this step model may use.

**The three snap bands.** On an update, `jump` = XZ distance from rendered position to the new server
position; `step` = expected_step_yalms for the incoming bytes. The band is a RATIO TO THE STEP,
never a flat distance (combat_stance.rs:1037-1058):

| Band | Condition | Action |
|---|---|---|
| Normal | `jump <= 1.0 * step` (`SNAP_NORMAL_RATIO`, :912) | chase |
| Stretch | `1.0*step < jump <= 2.0*step` (`SNAP_STRETCH_RATIO`, :914): a path re-eval or a late tick | chase, no snap |
| Pop | `jump > 2.0 * step`, OR the sample is older than `DT_SERVER_CEIL` (1.0 s, :920) | pop rendered XZ onto the server position |

Why step-relative: a running mob legitimately moves ~speed/50 per tick, and a late tick can exceed
a flat threshold on an ordinary update. The old flat constant (`SNAP_DIST_SQ = 4.0` yalms^2)
therefore read fast mobs' normal ticks as teleports and snapped them (the position pop). Measured
against what LSB actually moved, the ratio is invariant to the mob's speed: only a genuinely
anomalous jump pops.

**Pacing the chase to dt_server.** The chase rate is not a yps constant; it is paced to the packet
cadence so the rendered position arrives at the target on the frame the next update is expected,
never before. Per update (combat_stance.rs:1016-1049):

- `dt_server_smoothed`: running mean of the measured inter-update interval, EMA with weight 0.5
  (`DT_SMOOTH_ALPHA`, :931) so one late/early packet moves it at most halfway toward itself; clamped
  to `[DT_SERVER_FLOOR = 0.1 s (:926), DT_SERVER_CEIL = 1.0 s]`. The floor is a guard against
  pathological timing (packets faster than the ~400 ms AI tick cadence can justify), not a movement constant.
- `chase_rate = expected_step_yalms / dt_server_smoothed`: one smoothed interval covers exactly one LSB step.
- `hold_until = clock + dt_server_smoothed` (:1049): while the entity is inside this grace window of
  one expected interval past the last update, `is_chasing()` stays true even after reaching the target,
  so a late packet holds the gait instead of dropping to idle for a few frames (the stop-and-go stutter).
- The step clamps exactly on arrival (remaining distance caps it), so a stop can never overshoot and
  slide back; the old dr_velocity/VEL_BLEND_TAU/CORRECT_TAU/DECEL_TAU/STALE_VEL_SECS/MAX_DR_SPEED
  extrapolation is gone. Heading keeps HEADING_TAU smoothing.
- A late packet then finds the rendered position already at the target and holds it until the next POS:
  retail behavior (walk to target, stop on arrival), not a stall.

### 10.3 Gait from the wire bytes (no thresholds)

- `moving` = rendered position has not reached the target yet, OR the entity is inside its hold-until
  grace window (§10.2). No hysteresis, no velocity smoothing.
- **run = speed > speed_base**, else walk. One rule for Mob/Pet/Npc/Pc: LSB lifts only `speed` when
  it runs an entity (§10.1), so the comparison is the whole signal; it also picks the step divisor
  (`/50` run, `/40` walk) in §10.2. Self keeps its input-driven gait
  (`self_move_moving`/`self_walking`, driven by `move_speed_yps` in kuluu-session: that yps decode is
  the self-movement path only); entities without wire bytes keep the transform-delta fallback.
  No kind-specific branch, no base-speed constant anywhere in kuluu.
- Walk/run clip playback scales by `anim_rate_scale(speed_base)` =
  `speed_base * SPEED_TO_YPS / AUTHORED_ANIM_RATE` (retail's AnimationSpeed against the authored
  reference rate; both constants live in `kuluu-snapshot::speed`, cited to research/XiPackets
  world/server/0x000E). Applied on the locomotion tier only: idle and override tiers play at their
  authored pace. The scale is gated by the model's 0x45 movement byte (§10.7): Walking/Large (and
  Unset, which keeps today's behavior) take the wire scale; Flying/Sliding have no ground stride to
  match and play their locomotion clips at the authored rate (ffxi_actor_render.rs:3700-3712).
  CLIP_OK prints the resolved movement type (`move=`).

### 10.4 CLIP_WARN (regression guard, on by default)

Printed once per (world_id, clip, reason) when a selected clip resolves to no usable chunk in the
model DAT; `KULUU_CLIP_LOG=1` adds CLIP_OK for successful resolutions.

| reason | Meaning |
|---|---|
| `not_found` | No chunk in the model DAT matches the parameterized id (wlk?, run?, idl?, btl?, mvl?, mvr?, mvb?, sp0?/sp1?). Verify against `cow_tools/ffxi_disasm/dat_routines.py` output for that model before concluding the DAT lacks it. |
| `seq_load_error` | Motion chunk present in the DAT walk but rejected by parse (zero key frame sets, joint mismatch); the loader keeps a per-model list of seen-but-rejected chunks so this is distinguishable from not_found. Fix the parser/loader for that chunk shape; do not skip the model. |
| `no_match_kept_previous` | Selected id had zero matches and current_clip was left untouched: the frozen-mob signature. Unreachable since the fall-through fix (§10.5); kept as a warning one round, then debug_assert. |
| `not_found_override_skipped` | A higher-priority tier (special/fishing) asked for a clip the model does not ship; the override is skipped and selection falls through to the next tier. Expected once per entity on zone-in for models without the named routine. |

### 10.5 Fall-through rule (the frozen-mob fix)

`advance_actor_pose` walks the precedence tiers in order: action > engage overlay > fishing >
special > rest > locomotion. A tier claims the pose only when its clip resolves to at least one
usable chunk; otherwise it warns once and selection falls through to the next tier. Overrides apply
only when the model actually ships the clip (DAT-driven, no per-mob/pool/family gating). The
terminal fallback is idl? registered as a looping idle.

Before this fix a miss fell back to idl? registered as a one-shot keyed on the burrow/special
override, which pinned its end frame: every mob with a nonzero animationsub slid frozen in its spawn pose
while walking. In West Sarutabaruta that is every mob but the Rarab (the only one with
animationsub=0), which is why only it animated.

### 10.6 Crits are victim-side; there is no attacker crit swing (C)

LSB flags a crit in exactly one place: on `attack.IsCritical()` the server sets
`actionResult.info |= ActionInfo::CriticalHit`, and the same bool is passed to
`recordDamage(isCritical=true)` (public GIT read for C: entities/battle_entity.cpp ~L3736, where
recordDamage writes hitDistortion = Heavy(3) directly). In this tree's vendored revision the same
two facts sit at [battleentity.cpp](../vendor/server/src/map/entities/battleentity.cpp):3147-3150 (the CriticalHit bit)
and [action/action.cpp](../vendor/server/src/map/action/action.cpp):42 recordDamage (:59-63 sets the same bit from
isCritical; :68-85 derives distortion from HPP% thresholds in this revision). The enum itself
documents the level: `Heavy = 3, // 1.0 distortion (critical hits)`
([hit_distortion.h](../vendor/server/src/map/enums/action/hit_distortion.h):34). Either way the wire carries
`hitDistortion`, and routing keys on that byte: dist 3 is the heavy/crit case. It rides the VICTIM's
result block; wire order per [packets/s2c/0x028_battle2.cpp](../vendor/server/src/map/packets/s2c/0x028_battle2.cpp)
~L70-80: resolution(3) kind(2) animation(12) info(5) hitDistortion(2) knockback(3) param(17)
messageID(10) modifier(31).

Routing ([scheduler_runtime.rs](../kuluu-render/src/scheduler_runtime.rs):1493-1494, `hit_reaction_routine`):

| Hit result | Reaction |
|---|---|
| hitDistortion 3 (Heavy/crit) and the model ships `ldam` | `ldam` |
| hitDistortion 3 on a model without `ldam` | `damg` (same fallback pattern as Block -> shld/gur1; lookup is victim-own-first, then global dir) |
| hitDistortion 0/1/2 (None/Light/Medium) | `damg` per retail's dam0 branch table - never sdam, which flinches nothing on its own |

The attacker's `animation` field is the swing and is limb-selected, never crit-selected: LSB does not
flag an attacker-side crit anywhere. `swing_routine` (scheduler_runtime.rs:1518) carries a one-line
cite saying so; do not re-add a crit variant there. The `ActionInfo::CriticalHit` bit stays decoded in
kuluu-session's BATTLE2 outcome bits; nothing extra is gated on it today. If a bigger 0x21 impact
effect on crit is wanted later, it keys on that bit, not on a new packet field.

Unchanged by C: flinch still does not interrupt an in-progress swing (`dispatch_flinch_stages`
is_locked_now guard), and knockback > 0 still appends `sway` alongside the damage reaction
(scheduler_runtime.rs:1506).

### 10.7 The 0x45 Info chunk (D)

The model DAT's 0x45 Info record is a 16-byte body after the section header; ffxi-dat parses the
first 15 (`CIB_LEN`, [ffxi-dat/src/cib.rs](../ffxi-dat/src/cib.rs)) and ignores the uninterpreted sixteenth.
Field names follow vekien/xi-model-viewer `ui/js/dat/inspect.js` (parseInspectInfo :1378; tables at
:1344-1354), read as reference only:

| Offset | Cib field | Viewer interpretation |
|---|---|---|
| b[0] 0x00 | `movement_type` | MOVEMENT_TYPE (:1348-1350): 0 Walking, 1 Sliding, 2 Large, 3 Flying, 0xFF Unset. Out-of-table bytes read as Unset (this install's ROMs ship only 0/1/2/3/0xFF). Gates the stride scale in §10.3 |
| b[1] 0x01 | `footstep_material` | "Movement char" (base36) |
| b[2] 0x02 | `footstep_size` | "Shake factor" |
| b[3] 0x03 | `motion_index` | WEAPON_ANIM_STYLE (:1344-1347): battle idle pack index (0 Club/Staff .. 8 Polearm). Kept raw: the loader uses it as a DAT offset |
| b[4] 0x04 | `motion_option` | weapon sub byte |
| b[5] 0x05 | `is_shield` | selects the upper-body motion DAT (`base + is_shield + 1`) |
| b[6] 0x06 | `weapon_constrain` | - |
| b[7] 0x07 | `unknown2` | - |
| b[8] 0x08 | `weapon_unknown3` | - |
| b[9] 0x09 | `body_armour_waist` | selects the waist/skirt motion DAT (`base + max(waist_type,1) + 2`) |
| b[10] 0x0A | `scale` | model scale in percent; 0xFF = default. `Cib::scale_factor()` divides by 100 (research/xim poc/Model.kt:532-536 NpcModel.getScale) |
| b[11] 0x0B | `static_npc_scale` | scale for static NPCs not in a chair; retail swaps it in there (poc/Actor.kt:974-980). Named and available; the swap-in is a follow-up, not implemented |
| b[12] 0x0C | `unknown7` | - |
| b[13] 0x0D | `unknown8` | - |
| b[14] 0x0E | `motion_range_index` | RANGE_TYPE (:1351-1354): None/Wind/String/Marksmanship/ThrowingWeapon/ThrowingAmmo/Archery/HandbellIndi(0x0a)/HandbellGeo(0x0b)/Unset. xim documents the 0x07/0x08/0x09 gaps and reads out-of-table bytes as Unset; this install carries seven 0x08 CIBs |

Consumption:

- **Scale** applies to NPC subjects only, resolved once at load time in `kick_load_actor_tasks`
  (ffxi_actor_render.rs:2849+): it bakes the bind pose/bounds and rides along to spawn_live_actor's
  per-frame RootTransform, so both see one number. PCs stay at 1.0: retail copies the race CIB's
  movementType but drops its scale byte (poc/Model.kt:293-298 PcModel.getMovementInfo). Mounts are
  None on purpose: their Info layout is the mount variant, whose +0x0A byte is a pose type, not a
  scale (viewer :1395; xim readMountDefinition). Verified live in rabbit_tester S8: bat
  (ROM/4/106.DAT) loads at exactly 0.85 with Flying; the 100-scale walker is unchanged at 1.0.
- **Movement type** mirrors onto `FfxiRenderActor` and gates the playback-rate match in §10.3.
- **Chunk kinds**: ffxi-dat's ChunkKind now names Route (0x06), WeightedMesh (0x25), UiMenu (0x30),
  UiElementGroup (0x31), PointList (0x3E), SpellList (0x49), Path (0x4A), AbilityList (0x53),
  WeaponTrace (0x54), BumpMap (0x5D) and Blur (0x5E) after the viewer's SECTION_TYPE_NAMES
  (:17-26). They are named-but-unparsed: CLIP_WARN and the loader's rejected-chunk lists can name a
  chunk instead of printing an unknown code.

### 10.8 Speeds and distances are data or cited constants

LSB's speeds are per species (`mobutils.cpp ApplySpecies`), per NPC entry (`zoneutils.cpp`), and
mutable at runtime by Lua (`setBaseSpeed`, `setAnimationSpeed`). Nothing in kuluu may assume a
value: the chase rate, the snap bands, the gait rule and the playback scale all read the wire bytes
of the moment. Every threshold in this system derives from a wire byte or a cited LSB constant:
the step divisors 50/40 (StepToInternal), the band multipliers as ratios to that step (not
distances), and the dt_server clamp as a timing guard, not a movement constant. The only other
named constants are retail's own decode factors for the self-movement path and clip playback
(SPEED_TO_YPS = 0.1, mount doubling, clamp) and the authored reference rate they derive from.
