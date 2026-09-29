# kuluu_headless — canonical headless testing recipes

The single tracked source of truth for running kuluu without a visible window. Merges the
old `Cow_doc/Headless_testing.md` and `.agents/skills/verify/references/drive-headless.md`;
those files now point here. Windows-first (this machine), with the other-OS equivalents kept
inline — do not drop them.

Two headless surfaces, pick by what you're verifying:

- **A. AnimationTest box (pre-server)** — the launcher-phase VFX/combat test scene inside
  kuluu.exe (`enhanced-animationtest`, `view_native/animation_test_scene.rs`). No game
  session; needs the stack only for login up to character select. Use it for particles,
  routines, zone/weather load, actor placement, and **any render-layer question** (it is
  the only surface that loads MZB geometry + particles). §1.
- **B. Session headless** — a real session driven over MCP / raw stdio / integration tests.
  Use it for wire protocol, zoning, cutscene flow, chat, entity spawn. §2–§4.

Companion docs: `Cow_doc/LSB_Docker.md` (the stack itself), `.agents/skills/verify/SKILL.md`
(canonical verify recipes + evidence recording).

## 0. The rules that keep runs from wrecking the desktop

1. **Build with `.\build_cowland.bat`** (Windows) — release + full local feature batch, exe
   synced to repo root. Never an ad-hoc `cargo build -p kuluu --features <subset>`: a
   half-feature binary is not what gets verified and behavior can differ. On other OSes the
   equivalent is the same cargo invocation with that feature list (see the bat file header).
2. **Never open a visible window for agent runs.** Surface A: `KULUU_WINDOW_HIDDEN=1`.
   Surface B: raw stdio / MCP have no window at all; GUI attach (`FFXI_ATTACH=auto`) is only
   when pixels are required, and then warn first.
3. **Always mute** launches (`--mute`): a hidden run still decodes and plays BGM/SFX.
4. **Kill the process when done.** Windows: `taskkill //F //IM kuluu.exe`. Other OSes:
   `pkill -f kuluu`. Check first — never fire a test if one is already running
   (`tasklist | grep -i kuluu` / `pgrep -a kuluu`).
5. **Run from the repo root** — game files resolve from `vendor/game-files` relative to CWD.

## 1. Surface A: AnimationTest box (pre-server)

The box lives in `AppPhase::Launcher`. Launch with user+password only (no char name): that
stops at character select, where the box auto-opens when `ANIMTEST_AUTO` is set. Passing a
char name auto-starts a session and plays the intro cutscene instead — never do that here.

### Stack check

```bash
docker ps --format "{{.Names}}\t{{.Status}}" | grep cow-     # all Up
# host ports (container ports are remapped on this machine):
#   auth 53232   data 53231   view 54001
```

If the stack is down: `bash scripts/lsb-stack.sh up` (`Cow_doc/LSB_Docker.md`).

### Launch (Windows, Git Bash)

```bash
cd /c/Cow_Kuluu_ffxi-engine
KULUU_WINDOW_HIDDEN=1 \
ANIMTEST_AUTO="zone,weather,nhit,chit,dhit,mobnhit,mobchit,respawn,hi26,sb00" \
FFXI_MAP_LOCAL_PORT=47500 WGPU_ADAPTER_NAME=NVIDIA \
nohup ./kuluu.exe --server 127.0.0.1 --auth-port 53232 --data-port 53231 \
  play verilight 'TestPass!1234' --mute > /dev/null 2> client.log &
```

Other OSes: same env vars, `./target/release/kuluu` (or the synced repo-root binary), no
`WGPU_ADAPTER_NAME` pinning. The port numbers are this machine's docker remap — read them
from `docker port cow-connect`, not from memory.

- `ANIMTEST_AUTO` — comma-separated case names fired on a fixed clock with no input: first
  fire ~4s after app start, then one every 7s. The full list runs ~95s; pass only the cases
  you need (e.g. `zone,g141,g144`). Case names:
  `nhit chit dhit mobnhit mobchit respawn levelup g141 g144 hit1full hi26 sb00 zone weather shot`
  (`kuluu/src/view_native/animation_test_scene.rs::case_from_name`).
- Env overrides for the LoadZone case: `ANIMTEST_ZONE_ID`, `ANIMTEST_MZB_FILE_ID`,
  `ANIMTEST_WORLD_POS="x,y,z"` (zone geometry lands at absolute mzb_to_bevy(native) coords
  when ZERO), `ANIMTEST_HOUR=<h>` (pin VanaClock for night/midday captures),
  `ANIMTEST_CAM="px,py,pz,tx,ty,tz"` (re-frame the box camera).
- `verilight / TestPass!1234` — the local GM drive account (see §5).

### Reading the log

Everything of interest is an `[animationtest]` line on stderr:

| Line | Meaning |
|---|---|
| `ANIMTEST_AUTO: ...` | queue parsed at startup |
| `loaded: worm=... hume=[face=... sword=...]` | actor load requests out |
| `check ok: face=...(14) ...` / `ERROR: slot N (...) unreadable` | per-part mesh-buffer check before draw |
| `drawn on player: N mesh parts (all checked parts, sword included)` | post-spawn drawn count vs expected |
| `AUTO fired <case>` / `case: <case>` | a case started |
| `route <routine> spawned particle generator <gen> mesh <mesh> ... origin=(x,y,z)` | per-generator placement — the target-focus evidence |
| `wire lost: worm` / `wire lost: hume` | an actor was despawned mid-run (zone load, sync sweep) |
| `worm dead — respawning before hit` | auto-respawn guard fired |

Pass criteria for a grounded scene: particle origins at torso height (`y ≈ 0.6` worm-side /
`1.1` target-facing), worm-side generators at `x < 0`, target-facing at `x > 0`; zero
`wire lost` lines; no `ERROR:` lines.

### Capturing screenshots (hidden window, never on screen)

Two paths, in order of reliability:

1. **In-app `shot` case (preferred)** — Bevy reads back the render target itself, so it
   works even when the hidden window is zero-size (which is what `KULUU_WINDOW_HIDDEN=1`
   produces): add `shot` to `ANIMTEST_AUTO` and set `ANIMTEST_SHOT_PATH=C:/tmp/name.png`.
   It fires on the AUTO clock (~4s + 7s per preceding case), so order matters: put `zone`
   (or whatever you're framing) before `shot`, and remember zone placement takes longer than
   one tick — if the frame is empty, add more cases between them or re-run.
2. **`scripts/cap-window.ps1`** — `PrintWindow(PW_RENDERFULLCONTENT)` on the buried window;
   needs a non-zero client area (`bounds=0x0` = "zero-size window", use path 1):

```bash
powershell -NoProfile -ExecutionPolicy Bypass -File scripts/cap-window.ps1 kuluu artifacts/verify/<name>.png
```

- Fire it a few seconds after the `[animationtest] AUTO fired <case>` line you want to
  frame. Particle lifetimes are authored in 30 fps frames, so capture within ~1s of the
  fire for mid-burst shots; zone/weather shots can wait until placement settles (the log's
  `queued ... placements` / weather lines).
- Output is the window client area at its configured resolution (default 1280x800,
  `KULUU_RESOLUTION=WxH` to change). A black frame means the capture beat the first
  rendered frame — wait and re-shoot.
- These are the only sanctioned pixel paths for surface A. Do not launch a visible window
  "to check", do not use screen-grab of the desktop, do not point an example binary at the
  zone: those put pixels on Shane's screen.

### Kill

```bash
taskkill //F //IM kuluu.exe        # Windows   (other OSes: pkill -f kuluu)
tasklist | grep -ic kuluu          # must be 0 (pgrep -c kuluu elsewhere)
```

## 2. Surface B, option 1: kuluu-mcp standalone (preferred for sessions)

Spawns the full supervisor→reactor→session pipeline and exposes MCP tools/resources over
stdio. The only headless path with the reactor (goals, keepalive, event auto-dismiss) and
event-driven waits instead of log polling.

```bash
cargo build -p kuluu-mcp          # binary at target/debug/kuluu-mcp
FFXI_USER=... FFXI_PASS=... FFXI_CHAR=... FFXI_SERVER=127.0.0.1 target/debug/kuluu-mcp
```

Drive it as an MCP server over stdio (`.mcp.json` — `ffxi-agent/.mcp.json` is the canonical
config). High-value calls:

- `wait_for_event {kinds, timeout_ms}` — block until `zone_changed` / `entity_upserted` /
  `connected` fires. Use instead of polling.
- `read_resource scene://current` — entities, zone, self state as JSON.
- `read_resource diagnostics://session` — seq/sync counters, net health.
- `request_zone_change {line_id}` — zoneline trigger (char must be standing in the rect;
  move there first with `path_to`).
- `snapshot`, `chat`, `cast`, `engage`, `follow`, `disconnect` — full vocabulary in
  `ffxi-agent/instructions/playbook.md`.

`FFXI_ATTACH=auto` instead attaches to an already-running client — that is the GUI path,
not headless.

## 3. Surface B, option 2: raw stdio (`play --headless`)

Zero extra deps; JSON commands on stdin, typed JSON events on stdout, tracing on stderr.
Uses the reactor's explicit agent profile (goal commands and fishing automation behave as
MCP); no supervisor/reconnect layer or event-driven MCP waits.

**`--headless` is session-only: it runs NO render pipeline.** No Bevy app, no MZB geometry,
no particles — only protocol/session state (that's why `client.log` carries zero
`kuluu_render` lines). For any render-layer question use Surface A; a live session answers
"what does the server send around here", not "what is drawn".

Verified working recipe (Windows / Git Bash, this machine's stack). **Three separate
commands** — do not chain them:

```bash
# 1) setup its own command: D=C:/tmp/shity; rm -rf $D; mkdir -p $D; mkfifo $D/in

# 2) launch (prebuilt repo-root exe from build_cowland.bat, ports from `docker port cow-connect`):
(exec 3>$D/in; sleep 900) &   # hold the fifo write end open for the session's lifetime
./kuluu.exe --server 127.0.0.1 --auth-port 53232 --data-port 53231 \
  play <user> '<pass>' <CharName> --headless --mute < $D/in > $D/events.jsonl 2> $D/client.log &

# 3) drive / read, each its own command:
sleep 12; head -c 3000 $D/events.jsonl    # connected -> zone_changed -> entity_upserted
echo '{"cmd":"move","x":164.9,"y":164.8,"z":-5.5,"heading":64}' > $D/in
echo '{"cmd":"request_zone_change","line_id":812805498}' > $D/in    # zmr0, S. San d'Oria
```

Simpler stdin holder (any OS, no mkfifo): `(sleep 1800 | ./kuluu.exe ... --headless) > events.jsonl 2> client.log &`
— the pipe's write end stays open for `sleep`'s lifetime; EOF ends the session cleanly.

Other OSes: same three steps with `./target/release/kuluu` (or the synced repo-root binary);
`mktemp -d` is fine for `$D`; kill with `pkill -f kuluu`.

- Commands are `AgentCommand` serde: `{"cmd":"snake_case", ...}`; events are `AgentEvent`:
  `{"type":"snake_case", ...}` (`kuluu-session/src/state.rs`).
- Credentials are **positional args**, not env — env vars only feed the interactive
  launcher, which otherwise blocks on a `Username:` prompt.
- Coordinate space in commands/events: `x` = native x, `y` = ground (native z),
  `z` = vertical (native y).
- Zoneline ids are the fourcc as LE u32 — look them up in
  `vendor/server/data/zones/<zone>/zone.yaml` zonelines.
- `--headless` opens **no window at all** (Surface B never needs `KULUU_WINDOW_HIDDEN`,
  which is a Surface A thing); still pass `--mute`.
- Use the prebuilt repo-root `./kuluu.exe` from `build_cowland.bat`, not an ad-hoc
  `cargo run`: a half-feature debug binary is not what gets verified (rule 0).
- **Git Bash precedence gotcha**: `A && B & C` backgrounds the whole chain. If mkfifo sits
  in the same command as the launch, kuluu's `< $D/in` redirect can race fifo creation and
  die with "No such file or directory" before it ever connects. Setup and launch must be
  separate commands (verified failure mode).
- **Injecting commands into a running session**: once stdin is held by `sleep`, write to the
  agent TCP listener instead — launch with `FFXI_AGENT_LISTEN=:48199` and send one JSON line
  per connection:
  - Windows: `powershell -NoProfile -Command "$t=New-Object Net.Sockets.TcpClient('127.0.0.1',48199); $s=$t.GetStream(); $b=[Text.Encoding]::UTF8.GetBytes('{\"cmd\":\"move\",...}'); $s.Write($b,0,$b.Length); Start-Sleep -Milliseconds 300; $t.Close()"`
    (from Git Bash the `$` vars get eaten — put it in a .ps1 file and use `-File`).
  - Unix: `echo '{"cmd":"move",...}' | nc 127.0.0.1 48199`.
- Reading what the server sends around you: every nearby entity is an `entity_upserted`
  event; self is the one with `kind:"pc"` (shitypants = id 5 on this stack). Compute
  distance from self's `pos` over all upserts to see exactly who/what the server placed in
  the area — zone fixtures (lamps, torches) are NOT entities, they come from the MZB and
  only show up in Surface A runs.

## 4. Surface B, option 3: live integration tests (canonical layer proofs)

All in `kuluu-session/tests/`. They drive the real client against the real server and
**self-skip when the auth port or xidb is unreachable** (`tests/common/mod.rs`) — runtime
verification harnesses, not CI re-runs. Use them to bisect which layer is broken before
hand-driving.

| Test | Proves | Run |
|---|---|---|
| `play_lifecycle.rs` | auth→lobby→map→InZone→disconnect (~3s) | `cargo test -p kuluu-session --test play_lifecycle -- --nocapture` |
| `zone_change.rs` | GM `!zone` → reconnect → re-zone-in | `cargo test -p kuluu-session --test zone_change -- --nocapture` |
| `agent_session.rs` | full MCP-driven session (transport floor); spawns `target/debug/kuluu-mcp` — rebuild it first | `cargo test -p kuluu-session --test agent_session -- --nocapture` |
| `disconnect_recovery.rs` | map-server restart mid-session; destructive, opt-in | `RESTART_MAP_SERVER=1 cargo test -p kuluu-session --test disconnect_recovery -- --nocapture` |
| `event_503_live.rs` | end-to-end playback of the SSD new-character cutscene: cues in authored order, input-gated frames, onEventFinish rewards (item 536 + setPos to the gate); also self-skips when no FFXI install can be opened | `cargo test -p kuluu-session --test event_503_live -- --nocapture` |
| `auction_search_live.rs` | AH search-server smoke: browse a category + sale history over TCP SEARCH_PORT; no map session needed | `cargo test -p kuluu-session --test auction_search_live -- --nocapture` |
| `delivery_box_live.rs` | server-side delivery-box flow driven directly | `cargo test -p kuluu-session --test delivery_box_live -- --nocapture` |
| `action_dispatch.rs` | offline: subpacket layouts (cast/weaponskill/job-ability/item-use) match the phoenix structs | `cargo test -p kuluu-session --test action_dispatch` |

They use the `EphemeralChar` fixture (`tests/common/mod.rs`): an isolated account + char
stamped into MariaDB, gmlevel set before first login. If a manual flow fails where the
matching test passes, diff your flow against the fixture's — that delta is the bug or the
blocker. Gotcha: when the accounts AUTO_INCREMENT outruns the fixture's sentinel accid
scheme, the lobby rejects the char select ("mismatched character name" in connect logs)
and the test dies at the 0x02 ack step.

## 5. Accounts

- **Local GM drive account** (default): `verilight` / `TestPass!1234`, char `Verilamp`
  (gmlevel 5). The LSB provisioning-doc example credential for this machine's dev DB —
  safe to type into launch commands and logs, not a real secret.
- **The user's real character**: credentials come from env
  (`FFXI_USER`/`FFXI_PASS`/`FFXI_CHAR`); never commit or log them; use only when the check
  needs the user's own character/progress. On this stack: `cow / cowpass`, char
  `shitypants`.
- **Fresh provisioned chars** (no DB teleport needed for the basics):

  ```bash
  cargo run -p kuluu --features native-window -- provision <user> 'TestPass!1234'
  cargo run -p kuluu --features native-window -- create-char <user> 'TestPass!1234' <Name> 1 1 0 1 1
  docker exec cow-db mariadb -uxiadmin -ppassword xidb \
    -e "UPDATE chars SET pos_zone=230,pos_x=..,pos_y=..,pos_z=.. WHERE charname='<Name>';"
  ```

  (container name varies per stack — `cow-db` here; the fixture's own runs use whatever
  `tests/common/mod.rs` points at). The old "fresh chars get all c2s silently ignored"
  blocker was two client bugs, both fixed: the c2s datagram header must be the last
  subpacket's sync (`session.rs::datagram_header_id` — drift = server skips every
  subpacket silently), and the new-char intro cutscene rides the 0x00A LOGIN packet
  (`decode::ZoneInEvent`) and must be answered with 0x05B or the char sticks InEvent. If
  those symptoms return, check the sync/header invariant first.

## 6. Evidence

- Surface A: `client.log` `[animationtest]` lines (table in §1) — quote them inline; pixels
  via the `shot` case / `scripts/cap-window.ps1` (§1 capture), saved under `artifacts/verify/`.
- Surface B: `events.jsonl` (stdout JSON events), `client.log` (stderr tracing),
  `scene://current` snapshots, and the map-server log (`docker logs cow-map --since 5m` —
  LoadChar / cleanupSessions / `Invalid <name> packet from <char>` validator failures).
- Keep the raw captures until the report is delivered; quote the observed lines inline.
- Recording for the stop-hook verify gate:
  `.agents/skills/verify/scripts/record-evidence.sh --verdict pass --summary "<what was
  observed>" --artifact <path>` (rules in the verify skill, §Recording evidence).

## 7. Gotchas

**Both surfaces**

- **Ghost sessions after killing a client** — the next lobby login times out ("server did
  not respond within 20s"); the map server holds the char for 2–5 min. Prefer a clean
  `disconnect` (MCP tool / agent TCP / client exit) over `kill`. Otherwise wait for the
  server's own `cleanupSessions` line, or clear the one stale row:
  `DELETE FROM accounts_sessions WHERE charid=<charid>;` (keep the `WHERE` — other sessions
  on this stack are not yours to drop, and a whole-table delete is destructive).
- **colima dead / one-way UDP / login stuck at "Authenticating"** — the VM slept and
  virtiofs went stale: `colima restart`, then `docker start` the server containers.
- **Map UDP return path** — s2c UDP replies are kept alive by the `cow-dnat` sidecar
  (`Cow_doc/LSB_Docker.md`); if login succeeds and then map traffic goes silent, check that
  before theorizing about the client.

**Surface A**

- Char name in the play args = session auto-start + intro cutscene, not the box. User+pass
  only.
- `KULUU_WINDOW_HIDDEN=1` without `--mute` = audible BGM from an invisible window.
- The hidden window is **zero-size**: `cap-window.ps1` reports "zero-size window" — use the
  in-app `shot` case for pixels (§1).
- A zone capture that shows a different zone's terrain: the `shot` fired before MZB
  placement settled (the AUTO clock is fixed at ~4s + 7s/case; a full zone load takes
  longer). Re-order/re-run with more lead time, and confirm in the log that the target
  file_id's placements spawned before judging what you see.
- Particle origins at `y ≈ 40` after a zone load meant the collision block was baked
  without its `world_pos` (fixed in dat_mzb `spawn_mzb_overlay`) — if it ever returns, that
  is the first thing to check before blaming placement code.

**Surface B**

- **Raw stdio credentials are positional** — env vars only feed the interactive launcher
  (which blocks on a `Username:` prompt).
- **Coordinate swap** — in commands and events `y` is ground and `z` is vertical.
- **Session-only, no render** — see §3; do not expect geometry/particle log lines here.
- **Ending a driven session**: send `{"cmd":"chat","text":"/shutdown"}` from a GM account to
  restart the server and release every held session, then wait ~35s for docker to bring it
  back up before logging in again. A plain `taskkill` leaves a ghost session that the lobby
  rejects for minutes ("server did not respond within 20s", "no valid sessionHash").
