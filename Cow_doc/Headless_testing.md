# Headless testing — how to drive the headless test system

> **New 2026-09-13.** Companion docs: `Cow_doc/LSB_Docker.md` (the stack itself),
> `Cow_doc/cs_docs/cutscenes.md` (what the event 503 live test proves). The canonical
> recipes for verifying a change and recording evidence live in
> `.agents/skills/verify/SKILL.md` and its `references/` — this doc explains the headless
> system and how to drive it.

## What "headless" means here

The client runs as a plain process with no window. The session is driven over the agent
socket: JSON commands in, typed JSON events out, tracing log on a separate stream.
Everything protocol-observable (wire, session state, zoning, cutscene event flow, chat,
entity/spawn) is visible headless; pixels need the GUI window (out of scope here — see the
verify skill's GUI reference).

Three surfaces, strongest first:

1. **`kuluu-mcp` standalone** — full supervisor→reactor→session pipeline, MCP
   tools/resources over stdio. The only headless path with the reactor (goals, keepalive,
   event auto-dismiss) and the only one with event-driven waits instead of log polling.
2. **`play --headless` raw stdio** — zero extra deps; JSON commands on stdin. Uses the
   reactor's explicit agent profile (goal commands and fishing automation behave like MCP);
   no supervisor/reconnect layer and no event-driven MCP waits.
3. **Live integration tests** — canonical layer proofs; self-skip when no server is
   reachable.

## The stack (pointer)

Five containers under colima/docker (`dev-live/docker-compose.yml`): `cow-connect`
(auth 54231/tcp, data 54230/tcp, view 54001/tcp), `cow-map` (54230/udp), `cow-db` (MariaDB
`xidb`), `cow-search`, `cow-world`, plus the `cow-dnat` sidecar that keeps the s2c UDP
return path alive. Bring-up, the static-IP layout, and failure modes:
`Cow_doc/LSB_Docker.md`. Readiness: `docker logs cow-map -f` until
"The map-server is ready to work", then `nc -z 127.0.0.1 54231`.

## 1. kuluu-mcp standalone (preferred)

```bash
cargo build -p kuluu-mcp          # binary at target/debug/kuluu-mcp
FFXI_USER=... FFXI_PASS=... FFXI_CHAR=... FFXI_SERVER=127.0.0.1 target/debug/kuluu-mcp
```

Drive it as an MCP server over stdio (`.mcp.json` — `ffxi-agent/.mcp.json` is the
canonical config). The high-value calls for driving:

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

## 2. Raw stdio (`play --headless`)

```bash
D=$(mktemp -d); mkfifo $D/in; (exec 3>$D/in; sleep 900 & wait) &   # hold write end open
cargo run -q -p kuluu --features native-window -- \
  play <user> '<pass>' <CharName> --headless < $D/in > $D/events.jsonl 2> $D/client.log &

echo '{"cmd":"move","x":164.9,"y":164.8,"z":-5.5,"heading":64}' > $D/in
echo '{"cmd":"request_zone_change","line_id":812805498}' > $D/in    # zmr0, S. San d'Oria
```

- Commands are `AgentCommand` serde: `{"cmd":"snake_case", ...}`; events are `AgentEvent`:
  `{"type":"snake_case", ...}` (`kuluu-session/src/state.rs`).
- Credentials are **positional args**, not env — env vars only feed the interactive
  launcher, which otherwise blocks on a `Username:` prompt.
- Coordinate space in commands/events: `x` = native x, `y` = ground (native z),
  `z` = vertical (native y).
- Zoneline ids are the fourcc as LE u32 — look them up in `vendor/server/sql/zonelines.sql`
  (comments name each line).

## 3. Live integration tests (canonical layer proofs)

All in `kuluu-session/tests/`. They drive the real client against the real server and
**self-skip when the auth port or xidb is unreachable** (`tests/common/mod.rs`) — they are
runtime-verification harnesses, not CI re-runs. Use them to bisect which layer is broken
before hand-driving.

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

## Accounts

- **Local GM drive account** (default): `verilight` / `TestPass!1234`, char `Verilamp`
  (gmlevel 5). The LSB provisioning-doc example credential for this machine's dev DB —
  safe to type into launch commands and logs, not a real secret.
- **The user's real character**: credentials come from env
  (`FFXI_USER`/`FFXI_PASS`/`FFXI_CHAR`); never commit or log them; use only when the check
  needs the user's own character/progress.
- **Fresh provisioned chars** (no DB teleport needed for the basics):

  ```bash
  cargo run -p kuluu --features native-window -- provision <user> 'TestPass!1234'
  cargo run -p kuluu --features native-window -- create-char <user> 'TestPass!1234' <Name> 1 1 0 1 1
  docker exec cow-db mariadb -uxiadmin -ppassword xidb \
    -e "UPDATE chars SET pos_zone=230,pos_x=..,pos_y=..,pos_z=.. WHERE charname='<Name>';"
  ```

  The old "fresh chars get all c2s silently ignored" blocker was two client bugs, both
  fixed: the c2s datagram header must be the last subpacket's sync
  (`session.rs::datagram_header_id` — drift = server skips every subpacket silently), and
  the new-char intro cutscene rides the 0x00A LOGIN packet (`decode::ZoneInEvent`) and must
  be answered with 0x05B or the char sticks InEvent. If those symptoms return, check the
  sync/header invariant first.

## Evidence

- `events.jsonl` (stdout JSON events), `client.log` (stderr tracing), `scene://current`
  snapshots, and the map-server log (`docker logs cow-map --since 5m` — LoadChar /
  cleanupSessions / `Invalid <name> packet from <char>` validator failures).
- Keep the raw captures until the report is delivered; quote the observed lines inline.
- Recording for the stop-hook verify gate:
  `.agents/skills/verify/scripts/record-evidence.sh --verdict pass --summary "<what was
  observed>" --artifact <path>` (rules in the verify skill, §Recording evidence).

## Gotchas (headless-specific)

- **Ghost sessions after killing a client** — the next lobby login times out
  ("server did not respond within 20s"); the map server holds the char for 2-5 min. Prefer
  a clean `disconnect` (MCP tool / client exit) over `kill`. Otherwise wait for the
  server's own `cleanupSessions` line, or clear the one stale row:
  `DELETE FROM accounts_sessions WHERE charid=<charid>;` (keep the `WHERE` — other sessions
  on this stack are not yours to drop, and a whole-table delete is destructive).
- **colima dead / one-way UDP / login stuck at "Authenticating"** — the VM slept and
  virtiofs went stale: `colima restart`, then `docker start` the server containers.
- **Map UDP return path** — s2c UDP replies are kept alive by the `cow-dnat` sidecar
  (`Cow_doc/LSB_Docker.md`); if login succeeds and then map traffic goes silent, check
  that before theorizing about the client.
- **Raw stdio credentials are positional** — env vars only feed the interactive launcher
  (which blocks on a `Username:` prompt).
- **Coordinate swap** — in commands and events `y` is ground and `z` is vertical.
