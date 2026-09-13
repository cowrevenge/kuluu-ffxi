# FFXI cutscenes: how retail runs them, and how kuluu is coded

Sections 1 to 9 are the retail mechanism, checked against retail DATs from a current install (ROM/21/39.DAT, ROM/62/82.DAT, ROM/62/110.DAT, ROM/94/123.DAT, ROM/0/23.DAT), the in-tree research (research/XiEvents, research/XIClient, research/cexi-docs), the pinned LandSandBoat source (vendor/server), and where noted the FFXiMain.dll disassembly pass. Section 10 is the kuluu state as of 2026-09-13, section 11 the quick reference, section 12 the TODO list. Where something is inferred rather than read, it says so.

The disassembly pass lives in the `disassmembly_docs` set: `event_vm.md` (findings E1 to E20 with the decoded handlers), `event_opcode_table.md` (the full ExecProg dispatch table), `tpc_package_table.md` (the 0x66 file-id rule), `event_evidence.md` (raw dumps). E-numbers below refer to it.

Conventions: "DAT id" is the number FTABLE/VTABLE map to a `ROMn/dir/file.DAT` path (the resolver in `ffxi_dat::DatRoot::resolve`, or `ffxi_dat_find.py resolve <id>`). "4cc" is a four-character tag stored in file byte order. Event VM opcodes are written `0xNN` and packet opcodes `0x0NNN`; the two namespaces collide (VM 0x5B is LOADEXTSCHEDULER, packet 0x005B is EVENT_END) and must never be mixed up.

---

## 1. The one-paragraph version

A cutscene is a program. The server does not send the scene; it sends a trigger naming an actor and an event id, plus up to eight integers. The client finds that event id in the zone's event DAT, which holds compiled bytecode for every NPC in the zone plus a master block for the player, and starts a small virtual machine on it. That VM prints text out of the zone's string DAT, locks the camera, fades the screen, and delegates: it pushes requests onto other actors' request stacks so their own bytecode runs concurrently, loads gesture packages onto skeletons and plays named routines, and starts scheduler routines out of standalone scheduler DATs, most of which are camera moves along spline paths stored as camera resources. It pauses on timers, on the player dismissing text, on routines finishing, and on the server acknowledging mid-scene round trips. When the master script ends, the client tells the server which menu option the player picked, and the server's Lua decides what happens next.

---

## 2. Server side (LandSandBoat)

### 2.1 What starts a scene

A zone or NPC Lua script calls `player:startEvent(csid, params...)` (or returns a cs id from `onZoneIn`). LSB builds the EVENT packet:

| Packet | Direction | Meaning |
|---|---|---|
| s2c `0x0032` | server to client | EVENT with no params |
| s2c `0x0034` | server to client | EVENT with `num[8]` params |
| s2c `0x0033` | server to client | EVENT with string params |

Fields that matter: the actor's UniqueNo (server id) and ActIndex (target index), the event id (csid), and the params. The params land in the VM's Work_Zone slots the script reads as `param N`. Nothing else is sent. The bytecode is on the client.

### 2.2 The new character opening scene specifically

`vendor/server/src/login/login_helpers.cpp` inserts the charvar `HQuest[newCharacterCS]notSeen = 1` at character creation when `main.NEW_CHARACTER_CUTSCENE` is on (`settings/default/main.lua`). `scripts/quests/hiddenQuests/New_Character_Cutscenes.lua` fires the scene on zone-in while that var is 1, then gives the Adventurer's Coupon, prints the map tutorial line, sets the home point and clears the var in `onEventFinish`.

| Starting zone | Zone id | Event id | Event DAT |
|---|---|---|---|
| Bastok Markets | 235 | 0, then 7 from onEventFinish | ROM/21/44.DAT |
| Bastok Mines | 234 | 1 | ROM/21/43.DAT |
| Port Bastok | 236 | 1 | ROM/21/45.DAT |
| Southern San d'Oria | 230 | 503 | ROM/21/39.DAT |
| Northern San d'Oria | 231 | 535 | ROM/21/40.DAT |
| Port San d'Oria | 232 | 500 | ROM/21/41.DAT |
| Windurst Waters | 238 | 531 (has an onEventUpdate) | ROM/21/47.DAT |
| Windurst Woods | 241 | 367 | ROM/21/50.DAT |
| Port Windurst | 240 | 305 | ROM/21/49.DAT |

The cutscene flags passed with these (`UNKNOWN_1 | NO_PCS | UNKNOWN_2`, some add `NO_NPCS`) tell the client to hide other players (and NPCs) for the duration.

Skipping it needs server cooperation; see 10.4.

### 2.3 Mid-scene and end-of-scene packets

| Packet | Direction | Meaning |
|---|---|---|
| c2s `0x005B` EVENT_END | client to server | Scene ended (mode END) or an update request (mode UPDATE_PENDING) with `EndPara` = the value in Work_Zone[1], normally the menu option picked. Handler `vendor/server/src/map/packets/c2s/0x05b_*.cpp`. Lua: `onEventUpdate` for pending, `onEventFinish` for end. |
| c2s `0x005C` EVENT_END_XZY | client to server | Same, carrying a position the script authored for the player (VM opcode 0x47). |
| s2c `0x005C` PENDINGNUM | server to client | `player:updateEvent(...)` answer: eight ints written into Work_Zone from index 2 (research/XiPackets/world/server/0x005C). |
| s2c `0x005D` PENDINGSTR | server to client | String form of the same. |
| s2c EVENTUCOFF, `EVENT_RECV_PENDING = 1` | server to client | Releases the client's "waiting for server" hold after a pending tag. |
| s2c `0x0052` RELEASE / cancel | server to client | Server-side cancel of the running event. |

The client sends 0x005B/0x005C from VM opcodes 0x43 SENDTAG (case 0 sends, case 1 polls) and 0x47 EVENTPOSSET (same pair with a position), and holds the VM on the case-1 poll until PENDINGNUM/PENDINGSTR or EVENTUCOFF arrives. Windurst Waters' opening scene needs this round trip (its `onEventUpdate` sets param 7).

---

## 3. The event DAT (per zone bytecode)

Location: `ffxi_dat::event_locate::EVENT_DAT_LOCATIONS`, an irregular table (ROM/21/39..50 for the starter cities, ROM3/0/66+ for early zones, ROM4/0/51+ for others). Format per research/XiEvents `Event DAT Structures.md`, parser `ffxi_dat::event_dat`:

```
eventheader_t { u32 BlockCount; u32 BlockSizes[BlockCount]; }
eventblock_t  { u32 Actornumber; u32 TagCount;
                u16 TagOffset[TagCount]; u16 EvectExecNum[TagCount];
                u32 ImedCount; u32 ImidData[ImedCount];
                u32 EventDataSize; u8 EventData[align4(EventDataSize)]; }
```

- One block per actor. `Actornumber` is the NPC's server id (0x010E6001 style: high byte 01, zone 0x0E6, index). The zone/player master block is `0x7FFFFFF0` (`ZONE_PLAYER_ACTOR`). ROM/21/39.DAT has 502 blocks.
- The tag table is two parallel arrays: event id and byte offset into `EventData`. An event id of `0xFFFF` is not requestable by id, but its offset is real: those entries are the entry points other scripts jump to with REQSET by tag INDEX (section 5.3). `0xFFFE` is a wildcard matched for any requested id.
- `ImidData` is the reference table. Operands with bit 15 set (`0x8000 | i`) read `ImidData[i]`. Operands below 2048 are per-VM local work slots, 4096..4191 are the shared zone Work_Zone (params live at 4096+2 onward), and the 0x7F00/0x7F80 bands are live entity/player position reads.
- Many blocks own the same event id. In ROM/21/39.DAT event 503 is owned by the master block (offset 14321, 4211 bytes) and 35 NPC blocks. Each NPC's offset-1 handler is a one-liner (set position, set speed, end); the real per-NPC beats are the placeholder entries.

---

## 4. The string DAT (per zone text)

`ffxi_dat::zone_dat::zone_id_to_string_file_id` maps a zone to its dialog string file (the table is scraped at build time from POLUtils ROMFileMappings). Parser `ffxi_dat::dmsg::StringDat`. Message opcodes (0x1D, 0x2B, 0x48, 0x49, 0xB0) carry a message index; the client prints `strings[index]` with `{Num:N}` / `{Choice:N}` substitutions from the params. Menus (0x24 QUERY) index the same file. The VM blocks on 0x23 MESWAIT until the player dismisses, and on 0x25 QUERYWAIT until a choice is made; the choice lands in Work_Zone[0] (and [1] for the end para).

---

## 5. The event VM (XiEvent)

Reference: research/XiEvents `Event VM Structures.md`, `Event VM Functions.md`, `OpCodes/0xNNNN.md` (one file per opcode, with pseudo code); the local decode of ExecProg, the request stack, the waits and GetActorIndex is `event_vm.md` sections 2 to 7 (E1 to E13). kuluu port: `ffxi-event/src/vm.rs`, `vm/scene.rs`, `opcode_meta.rs` (widths, jump/yield flags).

### 5.1 Execution model

One XiEvent per actor (the entity's +0xD4 pointer). Each has an `ExecPointer`, a jump stack, local work slots, a `RetFlag` (yield this frame), and a 16-slot `ReqStack` (32-byte entries: Priority at +0, saved exec pointer at +0x26, TagNum at +0x3A, ReqFlag at +0x3B; E8). Each frame EventIdle picks the ReqStack entry with the lowest priority number (0xFF = empty slot, ties go to the later index), restores its saved exec pointer, and loops ExecProg until an opcode sets RetFlag (E9). Yielding opcodes: timed waits (0x1C WAIT, 0x6F SLEEP), input waits (0x23, 0x25), server waits (0x43/0x47 case 1), scheduler waits (0x53, 0x54, 0x55), request waits (0x28, 0x29, 0x2A), and the resource-load yields inside 0x5B/0x66. Timers count 1/60 s units.

### 5.2 Opcode families that matter for a scene

Housekeeping: 0x22 hide/show the event entity, 0x38 CliEventModeLocal (what the scene may alter), 0x42 clear cancel flag, 0x46 DEFCAMERA case 1 take the camera / case 0 give it back, 0x67/0x68 hide/unhide HUD, 0x77/0x78 stop/restore the game clock and weather, 0x69 sound volume, 0x5C music, 0x5D music volume, 0x2F render flags on an actor, 0x4E event-hide an actor, 0x79 lookat, 0x4A turn actor toward actor, 0x39 set facing, 0x32 set walk speed, 0x1F MOVE case 0 (start walking to x,z,y) / case 1 (wait for arrival), 0x37 set event position (teleport, also fed to the server for the player), 0x1E look and talk, 0x76/0x70 turn checks, 0x5E stop the event entity's action (idl0), 0x6B stop a named action on an actor, 0xC8/0x8B/0x8A map window and marker (the map tutorial), 0x21 set exec-end, 0x00 end the current request.

Actor motion, see section 6: 0x2C SCHEDULOR, 0x5B/0x66 LOADEXTSCHEDULER, 0x53 WAITSCHEDULOR.
Scheduler routines and camera, see sections 7 and 8: 0x45 LOADEVENTSCHEDULER2 (and 0x62, 0x9F, 0xBB, 0xC5, 0xCD, 0xD0, 0xD5 with other bases), 0x55 WAITLOADSCHEDULER, 0x52 ENDLOADSCHEDULER, 0x2D MAPSCHEDULOR, 0x54 WAITMAPSCHEDULOR.
Fan-out, see 5.3: 0x27, 0x28, 0x29, 0x2A.
Control flow: 0x01 GOTO and 0x02 IF take absolute offsets into EventData (verified against the corpus), 0x1A/0x1B call/return via the jump stack.

### 5.3 Fan-out: REQSET and the request stack

`0x27 REQSET prio(u8 @1) actor(u32 @2) tag_index(u8 @6)` (width 7): push (prio, tag) onto the TARGET actor's ReqStack, i.e. ReqSet is called on the target entity's own XiEvent at ent+0xD4 (E11); the entry's exec pointer is `TagOffset[tag_index]` in the target's block. Yields only if the stack is full (helper returns 2). `0x28` waits for a requested tag to start. `0x29 REQEW` pushes and then yields while that tag is still queued or running on the target (GetReqStatus). `0x2A REQWAIT prio actor` yields while the target has anything queued or running at priority <= prio (GetReqLevel returns 1 only when the running priority and all 16 slots are numerically above it). All four also require RenderFlags0 bit 7 on both entities.

This is how one scene drives dozens of actors. Southern San d'Oria's opening master script issues roughly forty REQSETs: background walkers at priority 99..110, the sequenced talk beats at 3..5, then REQWAITs before the next line. Work_Zone is shared by every actor's VM, which is how a master script and its children pass values.

### 5.4 Southern San d'Oria event 503, master block, condensed

```
0x22 hide self; 0x42; 0x38 mode; 0x46 camera lock; 0x69/0x5C volumes; 0x67 hide HUD; 0x77 stop clock; WAIT 30
repeat 8x:  0x45 file 130 tag s0NN (camera route, 600 frames)   [30834 = ROM/62/82.DAT]
            0x45 file 200 tag fdi1 (fade in)                       [30904 = ROM/62/110.DAT]
            0x55 wait fdi1; message; MESWAIT; 0x45 fdo1 (fade out); 0x55 wait s0NN; 0x55 wait fdo1; WAIT 60
            (0x27 REQSETs to background NPCs interleaved; 0x2F/0x4E show and hide groups of eight NPCs)
0x37 place player; 0x22 show self; WAIT
0x45 file 200 tag ovl1 on the event entity; 0x45 file 208 tag v000 (camera cut)  [30912 = ROM/94/123.DAT]
0x1F MOVE player to authored spot; MOVE wait; 0x6F; 0x7B
loop over guard beats:
   0x27 REQSET prio 3 guard tag N       (guard's part: 0x66 package 20 key tlk0 / thk1, or MOVE, or stop idl0)
   0x2B message from guard; 0x23 MESWAIT; 0x2A REQWAIT prio 3 guard
   0x45 file 208 tag v0NN (camera); 0x55 wait; 0x4A turns; 0x79 lookats; WAITs
0x24 QUERY menu; 0x25; 0x02 IF branches -> 0x66 package 20 key tlk0 on the guard + reply lines
0xC8 open map; 0x8B set marker; 0x8A close map
0x45 s004; 0x45 v016; 0x66 tlk0; message; 0x5E stop idl0
0x45 fdo2 (fade out); 0x55 wait; 0x78 restore clock; WAIT 120; 0x46 camera unlock; WAIT 120; 0x45 fdi2 (fade in)
0x21; 0x00  -> client sends c2s 0x005B EVENT_END with EndPara = the menu choice
```

Guard 0x010E6001's block (21 tag entries): index 15 = `0x66 file 20 key tlk0; WAIT 60; END`, 16 = `0x5E idl0; WAIT 60; END`, 17 = `0x66 file 20 key thk1; WAIT 60; END`, 11/13/14 = `MOVE to (x,z,y); MOVE wait; SLEEP; [0x39 face]; END`.

---

## 6. Actor motion: where an animation comes from

Three sources, three opcodes.

### 6.1 The actor's own routines: 0x2C SCHEDULOR

`0x2C actor1(@1) actor2(@5) key(@9)` (width 13) calls `SetAction(actor1, key, actor2)`. The routine named `key` must already exist among actor1's resident resources, which are the Scheduler chunks (kind 0x07) in its own model DAT plus the shared ROM/0/0.DAT library. Mobs and fixed-model NPCs have idl0/wlk0/run0/atk0/dead and so on; a player race model has the race's motion set. Nothing is loaded. A miss is a silent no-op returning 0 (SetAction @0xCEE50, E7).

### 6.2 Gesture banks: 0x5B LOADEXTSCHEDULER

`0x5B bank(@1) actor1(@3) actor2(@7) key(@11)` (width 15, always: no caller of the shared helper takes the +2 form, E3). Step one loads an event motion resource DAT onto actor1's skeleton, but only when actor1's entity Type is 1, 2, 7 or 8; doors, lifts and boats (Types 3 to 6) are a no-op (E4). The bank operand maps to a file through fixed bands (E4, matching research/XiEvents/OpCodes/0x005B.md):

| Operand | File id |
|---|---|
| 0..511 | 32104 + operand |
| 512..1023 | 49135 + operand |
| 1024..2047 | 56345 + operand |
| 2048..3071 | 59739 + operand |
| 3072.. | 66339 + operand |

Base 32104 is ROM/68/76.DAT; the banks run through ROM/68..73. Step two yields until the read completes. Step three, unless `key` is 0 or the bytes `xxxx`, kills the actor's last action and plays `key` with actor2 as partner. research/cexi-docs/cutscene_authoring.md: bank 60 (32164) holds exactly `ann0 ann1 han0 han1 ika0 ika1 pas0 ski0 thk1 thk2 tlb0 tlb1 tlk0 tlk1 yor0`; bank motion binds by joint index, so it distorts on fixed-model rigs (Maat, Byakko), and an actor-owned routine of the same name should win over a bank gesture.

### 6.3 Per-actor packages: 0x66 LOADEXTSCHEDULER2

Same layout and behavior as 0x5B, but the load goes through `ReadTpcEventMotionRes(actor, package)` @0xD2230, which is decoded in full in `tpc_package_table.md` (E5, E6, E17). The package number is not a global file id and not a flat offset. The reader gates on entity Type in {0, 1, 6}, rejects packages >= 0x118, and computes **two** file ids from four bands of the package number: A (base 32712 for packages 0 to 0x45, then 61241, 87825, 102239) attached with resource tag 1, and B attached with tag 2. Which B column is used, or whether B loads at all, is decided by the model's CIB (chunk 0x45) waist_type byte: 1 selects one column, 2 to 0x7F the other, 0 or >= 0x80 loads A only. DAT cross-check: package 20 (every Sandy opening beat on the guard) -> A = 32732 = ROM/72/87.DAT, which holds tlk0 and thk1; package 12 (Cornelia in cexi) -> 32724 = ROM/72/79.DAT, which holds kka0. kuluu still uses the flat `32104 + package` guess (section 10, TODO 1).

### 6.4 Waiting for motion

`0x53 WAITSCHEDULOR actor1(@1) actor2(@5) key(@9)` (width 13) yields while `IsMovingAction(key, actor1, actor2)` is true on actor1. `0x55` is the same for routines started from a file (section 7). Both first require both actors to resolve, to sit at their authored positions (f32 ent+0x74/+0x78 match) and to have RenderFlags0 bit 9 set, and fall through otherwise, which is why a client with no actor model resolves nothing and never waits. Retail then **polls** each tick: 0x53 via an actor vcall, 0x54 via a zone-object vcall, 0x55/0x52 via the scheduler is-moving/start pair keyed on (file id, tag). Nothing in retail times a hold from the routine's authored length (E13).

---

## 7. Scheduler routines and standalone scheduler DATs

A scheduler routine (chunk kind 0x07, parser `ffxi_dat::scheduler`) is a timeline of stages, each with a delay, a duration and a type: motion (0x05), model translate/rotate (0x0C/0x0D), sound (0x0A/0x0B/0x4A/0x60), particle (0x03..), screen colour drive (the fades), sub-routine links, animation lock, flinch, follow points, and camera (0x04, section 8). Combat actions, emotes, zone doors and cutscenes all use the same format; kuluu-render/src/scheduler_runtime.rs plays them on skeletons and is the machinery a cutscene reuses.

### 7.1 0x45 LOADEVENTSCHEDULER2 and its siblings

`0x45 file actor1 actor2 tag duration` (width 17). File id = base + `FUNC_DatIdHelper(file)`, where the helper folds two bands (`>= 600 -> +39643`, `>= 300 -> +25937`, else unchanged) and the base is 30704 for 0x45. The other opcodes in the family are the same call with a different base: 0x62 base 5012, 0x9F 51183, 0xBB 56685, 0xC5 67355, 0xCD 70435, 0xD0 70691, 0xD5 102449 (research/XiEvents/OpCodes/0x00NN.md, one line each). 0x7D plays a rank-up routine from `5112 + operand` on the player with tag `main`.

`duration` 0 means play the authored timing; 1 means loop; any other value is a total frame count the routine is stretched or squeezed to (research/XIClient CMoSchedulerTask, `speed_ratio = operand / total_frame`).

`0x55 WAITLOADSCHEDULER file actor1 actor2 tag` (width 15) yields while that (file, tag) routine is still running on actor1. `0x52 ENDLOADSCHEDULER` stops one.

### 7.2 The three scheduler DATs Southern San d'Oria's opening uses

| DAT id | Path | Contents | Used for |
|---|---|---|---|
| 30904 | ROM/62/110.DAT | 56 Scheduler, 28 Camera, 25 Sep, 3 Generator | Screen fades `fdi0/fdo0/fdi1/fdo1/fdi2/fdo2`, overlays `ovl1/ovl2`, atomos/blackout effects. Referenced as `file 200` (30704 + 200). kuluu-render/src/cutscene.rs already plays the fade stages out of it. |
| 30834 | ROM/62/82.DAT | 103 Scheduler, 104 Camera | `s000..s080` and `cm00..cm05`, `ca05/ca06`, `flt0`: each is one camera stage over a matching `cNNN` camera resource. `file 130`. |
| 30912 | ROM/94/123.DAT | 126 Scheduler, 132 Camera, 2 Sep | `v000..v019`, `t000..`, `w0..`, `x0..`, `y0..`, `z0..`, `r000..`, `se00/se01`, `u002`, `s004`: camera stages over `fNNN` and friends. `file 208`. |

Every scene routine in the last two decodes as three stages: `0x01` (0 frames), `0x04 camera <name>` for N frames, `0x00` end. N is 600 for the s-series, 0 (a hard cut) for v000/v003/v010, 700 for v001, 360 for v004, 120 for v016, 473 for s004.

### 7.3 The zone scene DAT and 0x2D MAPSCHEDULOR

`0x2D MAPSCHEDULOR actor1 actor2 key` (width 13) calls `XiZone::SetAction(zone, key, actor1, actor2)` to run a scheduler routine on the zone object itself, and `0x54 WAITMAPSCHEDULOR` waits on it (E14). The routines live in one **global** scene file, ROM/0/23.DAT (file id 23): 24 routines (mov1..mov8, ex1a..ex1c, ex2a..ex2f, ex3a..ex3e, main, loop) over 317 camera routes, all referenced by fixed name from inside the file; `main` just calls `loop`, and `loop` is a pick-one-per-group candidate list. 184 of the route names encode a zone id as two hex digits plus an index (8c01, c414, ...), covering 17 real zones; the rest are letter families (cgn0, crz1, ...). The routes use the attached form (AttachmentInfo nonzero, positions relative to caster/target). Of 52869 DATs only file 23 carries both movN and exNN routines; files 24 to 26 and a few ROM/62 files carry movN only (E14, E16, E18). How a zone object picks its scene file at runtime is not statically readable (BSS vtable), so treat "zone -> file 23 (or 24..26)" as the observed rule, not proof. Southern San d'Oria's opening scene does not use 0x2D at all; it plays everything through 0x45 out of the standalone files above.

---

## 8. Camera resources (chunk kind 0x06)

Reference: research/XIClient `include/World/Camera/CameraFormat.h`, `source/World/Camera/CameraFormat.cpp`, `CameraResource.cpp`, `CameraTask.cpp`, `SplinePath.cpp`, and `Game/Scheduler/Tags/0x04.cpp`. xim calls this chunk kind "Route"; XIClient calls it Camera.

Scheduler stage `0x04 <name>` looks up the Camera chunk `<name>` in the same file and calls `CameraResource::CreateCameraTask(scaledDuration, caster, target)`. A task drives the camera manager's next eye position, look-at target, roll and projection focal length every frame for the duration; a locked resource with duration 0 applies its point immediately (a cut).

Layout after the 16-byte chunk header:

```
CameraAttachmentHeader (16 bytes)
  u32 AttachmentInfo      0 = world space; nonzero = points are relative to the caster/target attach matrix
  u32, ptr, ptr           overwritten by the client at load
CameraHeader (16 bytes)
  u8  ControlPointCount
  u8  InterpFactor
  u16 Flags               1<<3 START_AT_CURRENT_POS, 1<<4 END_AT_CURRENT_POS (each adds a virtual endpoint)
  u32 SmoothingType       0 Linear, 1 Decelerate, 2 Accelerate, 3 DecelerateToMidpointThenAccelerate, 4 AccelerateAndDecelerate
  ptr, u32                filled by the client
SplineControlPoint[ControlPointCount] (48 bytes each)
  f32[3] Position
  f32    FovCalculationParameter   focal length; retail defaults 280 (first person) and 350; the Sandy routes use 280, 500, 682
  f32[3] Target
  f32    Roll
  f32[3] Param                     Param.x is the point's normalized time along the path in shipped data
  f32    Unused
```

Focal length to FOV: `FOV_deg = 2 * atan2(192, focal)` with 192 the half viewport height (XiClient + cexi, web tier); the 280/350 selection is a global-byte branch @rva 0x5940A feeding the focal setter, but the 192 term appears nowhere in FFXiMain.dll and is derived from the runtime viewport (E15, E19).

Path mode from the effective point count (count plus the two flag endpoints): more than 2 is a spline, exactly 2 is a straight line, otherwise locked. Verified against bytes: `c043` in 30834 is two points, world space, linear (a 600-frame dolly); `c077` is three points with Param.x 0 / 0.67 / 1.0 (a spline); `c006` is a straight two-point move. `8c01` in a zone scene DAT is two points with AttachmentInfo 1 (attached).

---

## 9. Timing: what holds a scene together

- 0x1C WAIT n: n units of 1/60 s; every cue in a retail scene is spaced by these.
- 0x23 MESWAIT / 0x25 QUERYWAIT: player input.
- 0x55 / 0x53 / 0x54: a routine still running. In the Sandy scene the master waits on each 600-frame camera routine before moving on, so the fades, text and REQSETs stay in step with the camera.
- 0x2A REQWAIT / 0x29: a child actor's script still running (the guard finishing a walk or a gesture).
- 0x1F MOVE case 1: the walking actor has not arrived.
- 0x43 / 0x47 case 1: the server has not acknowledged the pending tag.
- Inside 0x5B/0x66: the motion resource has not finished loading.

A client that implements the cues but not the holds plays the whole scene at "press Enter" speed with everyone standing still, which is what an incomplete port looks like.

---

---

## 10. Where kuluu stands (tree state 2026-09-13)

Southern San d'Oria's opening scene (event 503) runs end to end against a local LSB stack with retail DATs mounted: input-gated frames on the G1 -> "adventuring" -> G4a path fire in authored order, the holds pace it, camera lock takes and releases, gestures load on every guard line, and the map opens with the "Ailevia" marker. Per-ask status and the phase history live in `newplayrecslist.md` at the repo root.

### 10.1 What is in the tree

- **VM** (`ffxi-event/src/vm.rs`, `opcode_meta.rs`): widths for every opcode, messages and menus, 0x1C WAIT on a real clock, 0x46 camera lock (take and release), 0x4E event hide, HUD show/hide, stop/restore clock, music volume. 0x01 GOTO / 0x02 IF jump to absolute EventData offsets (`exec_pointer = eventgetcode(6)` when taken); a doc comment cites both readings and the corpus check (zones 77, 234, 241: every target lands on an instruction boundary only when read absolute), pinned by `if_taken_branch_target_is_absolute_into_event_data` and the `zz-jump-check` example. This came out of the Harara signet regression (zone 241 event 32759 returning EndPara 0 after a relative-offset change).
- **Request stacks** (`ffxi-event/src/vm/scene.rs`): 0x27/0x28/0x29/0x2A drive per-actor 16-slot stacks (priority, tag, child VM); lowest priority number runs each tick and a preempted child resumes from its saved pointer; 0x27 yields only on a full stack, 0x29 holds on the tag, 0x2A on priority. Placeholder (0xFFFF) tag entries are the REQSET entry points. A child that stops on an opcode the VM does not run is dropped from the stack so it cannot zombie-park the parent's REQWAIT (10.2).
- **Holds** (`vm.rs hold_action` / `hold_move`, `runner.rs`): the session owns time. When it publishes a motion cue it reads the routine's authored length from the DAT (`routine_units()` over the scheduler/camera chunk) and arms a hold keyed by (actor, key); 0x53/0x54/0x55 yield while a matching hold has frames left and fall through when none was armed. `kuluu-session/src/event_dialog.rs arm_motion_holds` arms 0x45 (with the duration override, same rule as `kuluu-render/src/cutscene.rs scheduler_speed_ratio`) and non-tpc 0x5B; move holds come from entity distance / speed; 0x2C arms nothing because its routine lives in the actor's model DAT, which the session never opens (TODO 2).
- **Gestures** (`ffxi-event/src/cue.rs`): 0x5B maps through the five fixed bands (`event_motion_dat_id`, matches E4); 0x66 goes out flagged `tpc` with `tpc_motion_dat_id` = flat `32104 + package`, which retail does not do (section 6.3, TODO 1). `CutsceneCue::ExtScheduler` plus the five actor cues (ActorMove, ActorPlace, ActorFace, ActorLookAt, ActorStopAction) are `kuluu-snapshot` PROTOCOL_VERSION 29.
- **Zone scheduler**: 0x2D emits `CutsceneCue::ZoneScheduler` (routine out of the global ROM/0/23.DAT) and 0x54 parks on the host-armed zone-sentinel hold; PROTOCOL_VERSION 30. The Sandy opening does not exercise it.
- **Renderer** (`kuluu-render/src/scheduler_runtime.rs dispatch_cutscene_motion`): ActorMotion plays the actor's own routine; non-fade Scheduler and non-tpc ExtScheduler load through ActionDatCache and dispatch with the partner as target; tpc cues try the actor's own routines then log; move/place/face/look/stop are applied client-side and released at CutsceneEnded. Fades stay in `cutscene.rs`; camera routes and the camera task in `cutscene_camera.rs` (kind 0x06 chunks parse to eye / look-at / roll / focal; c043 is pinned byte-for-byte against ROM/62/82.DAT).
- **Non-player motion**: MOVE / face / turn / lookat / stop cues for NPCs; 0x5A CodeMOVE2 is an alias of 0x1F MOVE with a mode-dependent width (10.2).
- **Pending-tag round trip** (0x43 SENDTAG / 0x47 EVENTPOSSET): `ffxi-event` `PendingTag` is `SendTag { end_para }` (c2s 0x005B, EndPara = Work_Zone[1]) or `SendXzy { x, y, z, dir, end_para }` (c2s 0x005C; coordinates scaled by XZY_COORD_SCALE, heading converted to the wire's 0..=255 scale via LSB `radianToRotation`). `StepResult::AwaitServerAck(PendingTag)` yields on the case-1 poll while a tag is pending, re-stepping re-yields without resending, `ack_server()` releases past the pair like retail's next-tick advance; `PENDING_NUM_WORK_ZONE_BASE = 2`. OP_EVENTPOSSET has its own arm (case 0 builds the tag from work slots, left-to-right fold). `ffxi-proto`: c2s `EVENT_END_XZY = 0x05C` with `event_end_mode { END = 0, UPDATE_PENDING = 1 }` (the 0x05C validator accepts only mode 1); s2c `PENDINGNUM = 0x05C` (eight i32 into Work_Zone from index 2, `decode/pending.rs`) and `PENDINGSTR = 0x05D`; ucoff `EVENT_RECV_PENDING = 1` is the ack both c2s event-end handlers push. The opcode collisions across directions are intentional. `kuluu-session/src/session/mod.rs` and `event_dialog.rs` carry the real send / ack wiring. PENDINGSTR is decoded but its only retail reader (0xB4 case 1) is skipped by width (TODO 3).
- The player's MOVE / 0x37 modeled against the server, and the full skeleton scheduler runtime the cutscene reuses.

### 10.2 What building event 503 taught us (retail mechanisms, each pinned by a test)
- **The zombie-park failure mode.** A child request that stops on an unrunnable opcode must be *removed* from the actor's ReqStack, not just logged. If it stays, a `REQWAIT`/`REQEW` at a priority <= its slot parks forever and the whole master script hangs. This was the E4 stall: D7's party child died on its first 0x5A, zombied the stack, and the master's REQWAIT never released. Handled in `vm/scene.rs step_stacks`; regression test in `vm.rs`.
- **0x5A CodeMOVE2 width is mode-dependent.** The retail dispatch data lists two widths for it (2 and 8, same as 0x1F MOVE), in the "wait" sub-mode retail advances by just 2. Reading the wrong width desyncs every later operand in the child's block, so the child dies on an unrelated opcode and zombies. `opcode_meta sub_size(0x5A, mode)` returns `{0 => 8, 1 => 2}`; pinned by a width test plus a "walks like MOVE" regression test.
- **The live test must record event end.** The session emits `AgentEvent::EventEnded` on normal completion (and cancel), and the live playback test `kuluu-session/tests/event_503_live.rs handle_event` consumes it, gated on the 503 cutscene having started (EventEnded is a unit variant with no id).
- **The gate setPos is a server reward, not an event instruction.** onEventFinish[503] runs `giveItem(536)` then `setPos(-100, 1, -40, 224)`. In live runs the map-marker message + item both arrive (so onEventFinish ran), but no WPOS/position update reaches the client, last player pos is ~(-98.8,-39.6,+1) at event end and zero `forced_move` events. The client's WPOS path (`s2c::WPOS|WPOS2 -> decode::ForcedMove`, gated on `unique_no==self_char_id && mode.carries_position()`) is correct and unit-tested; this is server-side behavior in the local LSB build, not an event-VM bug.
- **ESC lock-in is authored bytecode, not a client quirk.** Retail gates ESC-cancel behind two globals: 0x42 clears `CliEventCancelSetData` (and `CliEventCancelFlag` when `CliEventCancelSetFlag` is open) and 0x2E sets them (`research/XiEvents/OpCodes/0x0042.md`, `0x002E.md`). Event 503's master block runs 0x42 as its second opcode, 14× in the block, zero re-arms, so ESC is a no-op for the whole cutscene: retail sends *no* EVENT_END at all. kuluu models this as one `cancel_armed` flag (armed at event start; 0x42->false, 0x2E->true) that rides on every dialog frame to the client, where the NavCancel branch of `handle_dialog_key` drops the key while disarmed instead of sending EndEvent. The two-flag gate collapses safely because `CliEventCancelSetFlag` is empirically open on events using these opcodes.
- **The map beat holds open until Enter, MESWAIT sits between marker and close.** Event 503's epilogue runs `MAP_TUTORIAL` -> `MAP_MARKER` ("Ailevia") -> `GET_STORE`(item 536) -> the coupon line -> **MESWAIT (0x23)** -> `CLOSE_MAP`, identically in both copies of the block (master-block offsets +045D9..+04606 and +047A2..+047CF). MESWAIT sets RetFlag while `CliEventMessOpenFlag` marks a message open, so `XiEvent::ExecProg` halts with the map still on screen (`research/XiEvents/OpCodes/0x0023.md`, `research/XiEvents/Event VM Functions.md`); retail holds the full-screen map over the coupon line until the player presses Enter, and only then runs CLOSE_MAP. (An earlier "no visible flash" reading came from a test client that auto-dismissed instantly; retail holds the map, user-confirmed.) kuluu's VM paces this correctly (the MESWAIT parks with the map open; CLOSE_MAP runs post-dismissal); `event_map_sync_system` still handles a back-to-back burst gracefully when all three arrive in one pass (open -> marker upsert -> close, mode back to World, marker persists, "Used by NPCs that help new players and mark your maps", `research/XiEvents/OpCodes/0x008B.md`), pinned by test `event503_same_tick_open_marker_close_leaves_the_marker_not_the_map`.
- **A finished camera route holds its last frame while the lock outlives it.** While 0x46 case 1 is in force retail has disabled user camera control and menu drawing; when the lock releases, case 0 runs `YmCameraTask_KillAll()` and re-seats the chase camera at the player's position (`research/XiEvents/OpCodes/0x0046.md`). So while locked with no active route, the operator camera simply stays where the finished route left it, event 503 holds its camera from +037F7 to +04683 across all of its MESWAITs. kuluu reproduces this: `advance_cutscene_camera_task` runs after `resolve_camera` (whose chase write would otherwise take the camera back every frame) and re-applies the last applied frame while `camera_locked`; a lock with no route yet captures the operator state once and freezes it. Pinned by tests `a_finished_route_holds_its_last_frame_while_the_lock_outlives_it` and `a_lock_before_any_route_freezes_the_operator_state` (`kuluu-render/src/cutscene_camera.rs`).
- **Input belongs to the event while a frame is up.** Retail's 0x46 case 1 disables menu drawing for the whole camera hold, and its message flag only clears on dismissal (`research/XiEvents/OpCodes/0x0046.md`, `0x0023.md`), so during a VM-driven event the player's keys belong to the event: Enter advances even with the map open in Menu(Map) mode (event 503's epilogue beat, without this routing a human playthrough hangs there, because the menu handler would swallow Enter), and ESC respects the cancel_armed disarm instead of closing whatever window is up. kuluu gates at the top of `text_input_system`: while `CutsceneMode.active` and a dialog frame is up, every key routes through `handle_dialog_key`. Pinned by system-level tests in `kuluu/src/view_native/text_input/mod.rs cs_input_lock_tests` (Enter advances with the map open; ESC is a no-op while disarmed; without an active CS session the menu handler keeps its own keys). Dismissal itself clears the displayed frame (`AgentEvent::DialogDismissed`, emitted on the up->down edge of `message_awaiting()`), so the advance hint cannot linger over camera holds. Retail's advance affordance is not literally "Press Enter" (observation suggests dots or similar), TODO 5.
- **HIDE_HUD is narrower than its name.** 0x67 only repositions the two event message boxes (`PresetEventMessageMode` ×2) and sets `CompassDrow` (`research/XiEvents/OpCodes/0x0067.md`), it never touches the map window, which is an event-opened MAPSCHEDULOR surface. kuluu's cutscene HUD-hide therefore exempts the dialog panel root and both map roots via `HudHideExempt` (`kuluu-render/src/hud_hide.rs`); draw-level tests in `hud/dialog.rs` and `hud/map_screen.rs` pin that a cutscene hide keeps those surfaces visible while plain HUD chrome hides.
- **Dialog speaker identity is per-frame, resolved against the live entity buffer.** Retail's message opcodes carry a target index and resolve it at draw time: 0x1D uses `EntityTargetIndex[1]` as the speaker (missing entity prints "???: "), 0x2B embeds its own actor reference, 0xB0 resolves a speaker *and* listener pair and skips the line entirely if either lookup fails; 0x48/0x49 are narration with no speaker (`research/XiEvents/OpCodes/0x001D.md`, `0x002B.md`, `0x00B0.md`). kuluu carries `speaker_index` on every dialog frame (VM: OP_MESSAGE -> trigger index, OP_MESSAGE_ACTOR/_PAIR -> looked-up actor, narration -> None) and resolves act_index->unique_no via a target cache built from CHAR_PC|CHAR_NPC heads, then unique_no->name; unresolvable or narration frames get an honest blank header rather than the render fallback's trigger-name (which would mis-attribute every line). `npc_id` stays pinned to the trigger because EndEventChoice echoes it back for EVENT_END validation. Divergence: retail prints "???: " on a missing 0x1D entity; kuluu blanks; event 503's 51 frames all resolve, so it never surfaces there (TODO 6).

### 10.3 Known divergences from retail

- **Holds are DAT-timed; retail polls** (E13, section 6.4). Exact when the routine runs at authored speed; the 0x45 duration override is applied to both the hold and `scheduler_speed_ratio`, so the two stay in step. 0x2C arms no hold at all.
- **0x66 Tpc mapping is flat; retail is banded A/B** (section 6.3, TODO 1).
- **0x1D speaker on a missing entity**: retail prints "???: ", kuluu blanks (TODO 6).
- **Attached camera routes** (AttachmentInfo nonzero) are dropped with a log (TODO 4).
- **PENDINGSTR** is a named no-op (TODO 3).
- **0x76 TURNCHECK / 0x70 TURNWAIT** never hold; kuluu advances by width (TODO 7).

### 10.4 The "skip intro CS" toggle

Client side is correct and the toggle is a no-op against vanilla LSB. `kuluu-session/src/lobby_client.rs` masks the flag with `supports_skip_intro_cs()`, true only when the char-list reply carried KULUU_CAP_KEY (867309) with CAP_SKIP_INTRO_CS set; vanilla LSB zero-inits that field (`src/login/data_session.cpp`), so the log says "server did not advertise CAP_SKIP_INTRO_CS; register sent without the skip flag" every time. Server side is section 2.2. Fixes are outside kuluu: patch the LSB build to stamp the cap and honor the register byte, set `NEW_CHARACTER_CUTSCENE = 0` in settings/main.lua, or clear the charvar on a test character.

---

## 11. Quick reference: ids and paths

| Thing | Where |
|---|---|
| Zone event bytecode | `ffxi_dat::event_locate` table; starter cities ROM/21/39..50 |
| Zone strings | `zone_id_to_string_file_id` (build-time table) |
| Scheduler DAT base for 0x45 | 30704 + DatIdHelper(operand) |
| Screen fades | 30904 = ROM/62/110.DAT (`fdi*`, `fdo*`, `ovl*`) |
| Sandy opening camera routines | 30834 = ROM/62/82.DAT, 30912 = ROM/94/123.DAT |
| Gesture banks for 0x5B | 32104 + bank (ROM/68/76.DAT up), bank 60 = 32164; Type gate {1,2,7,8} |
| Default humanoid talk set | 32104 (tlk0, thk1, ...) |
| Tpc packages for 0x66 | banded A/B, `tpc_package_table.md`; pkg 20 -> 32732 = ROM/72/87.DAT; kuluu still flat (TODO 1) |
| Zone scene DAT for 0x2D | global ROM/0/23.DAT (file id 23); per-zone runtime derivation open (TODO 9) |
| Shared routine library | ROM/0/0.DAT (`syst`) |
| Rank-up routine (0x7D) | 5112 + operand, tag `main` |
| Opcode docs | research/XiEvents/OpCodes/0xNNNN.md; dispatch table `event_opcode_table.md` |
| VM internals | research/XiEvents/Event VM Structures.md, Event VM Functions.md; local decode `event_vm.md` |
| Camera format and task | research/XIClient include/World/Camera/*.h, source/World/Camera/*.cpp |
| Scheduler tag handlers | research/XIClient source/Game/Scheduler/Tags/0xNN.cpp |
| Authoring notes, bank inventories | research/cexi-docs/cutscene_authoring.md, external_source/New-Player-Cutscene-Pipeline.md |
| Server triggers and end handlers | vendor/server scripts (startEvent, onEventUpdate, onEventFinish), src/map/packets/c2s/0x05b_*.cpp, 0x05c_*.cpp |
| Per-ask status for event 503 | `newplayrecslist.md` (repo root) |
| Event-503 master-block disassembly, camera-byte dumps | session scratchpad (`evt503_master_disasm.txt`, `col_c043.txt`); excerpts in sections 5.4 and 8 |

---

## 12. TODO

1. **0x66 Tpc mapping.** Replace `ffxi-event/src/cue.rs tpc_motion_dat_id` (flat `32104 + package`) with the four-band A/B rule from `tpc_package_table.md`. The cue must carry both file ids (tag 1 / tag 2) so the host loads both containers; B is selected by `Cib::body_armour_waist` (ffxi-dat/src/cib.rs), which every rendered actor already has (`cib: Option<Cib>` in kuluu-render/src/ffxi_actor_render.rs): byte 1 -> A + B_set, 2..0x7F -> A + B_clear, 0 or >= 0x80 -> A only. Anchors: pkg 20 -> 32732 (tlk0 + thk1), pkg 12 -> 32724 (kka0).
2. **0x2C SCHEDULOR has no hold.** The routine lives in the actor's model DAT, which the session never opens, so `arm_motion_holds` arms nothing and 0x53 falls through. Either document it as accepted or add a renderer-reported finish.
3. **PENDINGSTR** (s2c 0x005D) is decoded but never consumed. Wire it only if a scene uses it.
4. **Attached camera routes.** Need retail's bone attach matrix to resolve AttachmentInfo-nonzero points (`cutscene_camera.rs` drops them with a log). The ROM/0/23.DAT routes are all attached, so 0x2D scenes will hit this.
5. **Retail advance affordance** is not literally "Press Enter" (dots or similar). Research note.
6. **0x1D speaker divergence**: print "???: " on a missing entity instead of blanking.
7. **0x76 TURNCHECK / 0x70 TURNWAIT** are render-state polls kuluu never holds on. Confirm no scene depends on the result.
8. **Windurst Waters 531 `onEventUpdate` round trip** is unit-verified, not live-verified.
9. **Per-zone scene DAT selection.** File 23 is established for 0x2D, but which scene file a zone object loads at runtime is not statically readable (E14); low zones look like identity. Needs a runtime read or a second data point.
10. **Holds vs polling.** Decide whether to keep the DAT-length approximation (documented) or move to a "poll until the routine reports not running" predicate now that the renderer can report finishes.
