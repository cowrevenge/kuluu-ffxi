# PC crit-hit effect DATs (eflg/selg family) — 2026-09-27

Question: which DAT carries a PC's crit-hit flash, and how does retail select it?
(No white/red hit flash in kuluu live; user's viewer read of ROM/0/0.DAT pointed at the
hitN family.)

## Chain (byte-dumped from the installed client via ffxi_disassembly/dat_routines.py)

- Normal melee hit: victim `damg` → ref09 **chit** on the attacker → `call57 se h` +
  `call57 ef h`. For a HumeM sword, `ef h` is defined in the weapon DAT (id 8392 =
  ROM/29/20.DAT) as `call hit1`; `hit1` (ROM/0/0.DAT) fires g010–g013 with zero timing.
- Crit: victim `ldam` → ref09 **lhit** on the attacker. `lhit` is defined in the race
  skeleton base (HumeM id 7072 = ROM/27/82.DAT) as `call57 eflg` + `call57 selg`.
- `eflg` = `call sho1` (+ `selg` sound). `sho1` (ROM/0/0.DAT) fires gs10–gs12 with mixed
  timing — the visible crit burst.

## Where eflg/selg live, and how retail loads them

- `eflg` is NOT in ROM/0/0.DAT and NOT in the specific weapon DAT (8392 ships only
  skaz/sinr/sotr/ef h/se h). For HumeM it lives in **ROM/32/0.DAT … ROM/32/12.DAT**
  (ids 9485–9497) — one file per weapon-type slot, all defining the same routine set
  `selg eflg lgin lgot kalg` (mesh wep2), differing in content. The amot battle base is
  id 9672 = ROM/32/13.DAT: the effect siblings sit at directory indices 0–12, directly
  below the base at index 13. No ROM/32 file defines `ef h`, so normal-hit selection is
  unaffected by these files.
- The retail loader for this family is `ReadAtkMotionRes` in research/XIClient/src/
  XIClient/source/World/Actor/SkeletalMeshActor.cpp (dispatched from ReadMotionPackPolling,
  read_reqno=2) — marked XICLIENT_CODE_MISSING, so the exact slot arithmetic is not
  decompiled. The sibling loader that IS decompiled, `ReadDefMotExtRes`
  (`m_raceMotionIndex + 0x7F68`, waist variants +0x7F88/+0x7FA8), resolves for HumeM to
  ROM/71/99.DAT et al., which do NOT carry eflg — a different per-race family.
- Kuluu's load set before this fix: [amot base + weapon_anim_type file, amot base,
  skeleton base, upper/waist, equipment DATs] — none of the effect siblings, so `eflg`
  never resolved and the crit flash silently no-opped (run_routine_on drops unresolvable
  names).

## Decision

load_pc adds one effect sibling: same directory as the amot base, file index = the
weapon CIB's motion_index (clamped to 0–12), guarded on the file defining `eflg` so races
whose family is laid out differently load nothing extra. Verified for HumeM; other races
degrade to today's behavior until verified per race.
