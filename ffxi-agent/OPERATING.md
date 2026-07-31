# Operating the FFXI agent harness

Operator-facing runbook for driving an LLM harness against a live
LandSandBoat-family server. The LLM-facing playbook is in `CLAUDE.md` /
`AGENTS.md`; this doc is for the human on the stage.

Stack bring-up, surface selection, and evidence capture are **not** repeated
here — they live in the `verify` skill (`.agents/skills/verify/`), which is
the maintained source of truth:

| Need | Read |
|---|---|
| Get the LSB stack up, container/port table, colima gotchas | `references/stack.md` |
| Headless MCP drive, raw stdio, live integration tests | `references/drive-headless.md` |
| Native window + MCP attach, screenshots, keystrokes | `references/drive-gui.md` |

## 1. Build

```bash
cargo build -p ffxi-mcp
```

The binary lands at `target/debug/ffxi-mcp`. Integration tests locate it by
walking up from `current_exe` (`tests/common/mcp_client.rs:36`), so skipping
this step makes `agent_session` / `disconnect_recovery` panic with an explicit
build instruction. Note `.mcp.json` invokes `cargo run --release` instead —
harness runs and test runs use different profiles, so build both if you're
alternating.

## 2. Tests

Unit tests come with the normal gate; don't hand-roll per-crate invocations:

```bash
scripts/checks.sh test
```

Live integration tests self-skip when nothing is reachable on
`SERVER_HOST:AUTH_PORT` (defaults `127.0.0.1:54231`). Each is a whole-layer
proof, so run the one that matches the layer you suspect:

```bash
cargo test -p ffxi-client --test play_lifecycle    -- --nocapture
cargo test -p ffxi-client --test zone_change       -- --nocapture
cargo test -p ffxi-client --test agent_session     -- --nocapture
cargo test -p ffxi-client --test action_dispatch   -- --nocapture
cargo test -p ffxi-client --test delivery_box_live -- --nocapture
```

- **`play_lifecycle`** — auth → lobby → map → InZone → disconnect, on the bare
  session actor with no supervisor/MCP wrapping. Use it first to decide whether
  a failure is in the session layer or above it.
- **`zone_change`** — GM `!zone N` → reconnect → re-zone-in. Requires
  `gmlevel ≥ 1`, which the fixture sets. Validates Blowfish key rotation across
  the transition.
- **`agent_session`** — the transport-conformance floor. Drives `ffxi-mcp` over
  JSON-RPC stdio: `initialize` → `tools/list` → `resources/list` →
  `resources/subscribe scene://current` → wait-for-InZone → read
  `scene://current` → `tools/call snapshot` → expect
  `notifications/resources/updated` → `tools/call disconnect`. Does **not**
  exercise aggro detection, party packets, `/tell`, `RequestZoneChange`, or any
  reactor goal.

`disconnect_recovery` is destructive and opt-in — it restarts the map-server
container mid-session, which disrupts anything else sharing the stack:

```bash
RESTART_MAP_SERVER=1 cargo test -p ffxi-client --test disconnect_recovery -- --nocapture
```

It restarts `server-map-1` (override with `MAP_SERVER_CONTAINER`), asserts the
supervisor notices within 30 s and is back InZone within 60 s, then prints
`reconnect_downtime_ms=…`. See §5 for why the budget is what it is.

## 3. Driving an LLM harness manually

Two ways in. **Standalone** — `ffxi-mcp` owns its own session:

```bash
export FFXI_USER='your_account'
export FFXI_PASS='your_password'
export FFXI_CHAR_ID=12345678        # u32 from chars.charid
export FFXI_CHAR='YourCharName'
export FFXI_SERVER=127.0.0.1
export RUST_LOG=info,ffxi_client=info,ffxi_mcp=debug
```

**Attach** — a native `ffxi-client` window owns the session and the harness
joins it over a unix socket, so you can watch what the LLM is doing:

```bash
cargo run -p ffxi-client -- --agent-listen auto play
```

That writes `$TMPDIR/ffxi-agent.pid` with the socket path; the `ffxi-attach`
server in `.mcp.json` sets `FFXI_ATTACH=auto` to resolve it
(`ffxi-mcp/src/attach.rs:10`). Recipe and gotchas: `drive-gui.md`.

Then point a harness at the binary:

* **Claude Code**: `cd ffxi-agent && claude` (auto-discovers `.mcp.json`).
* **OpenCode**: `cd ffxi-agent && opencode` (same `.mcp.json`).
* **MCP Inspector**: `npx @modelcontextprotocol/inspector ./target/debug/ffxi-mcp` —
  fastest way to confirm tools/resources surface correctly without an LLM.

### 3a. Harness compatibility

All supported harnesses speak MCP over stdio with the same `.mcp.json`; no
special transport flags or wrapper scripts.

| Harness         | Config file        | Transport | Env interpolation | Auto-discovery | Notifications |
|-----------------|--------------------|-----------|-------------------|----------------|---------------|
| Claude Code     | `.mcp.json`        | stdio     | `${VAR}`          | yes (CWD)      | yes           |
| OpenCode        | `.mcp.json`        | stdio     | `${VAR}`          | yes (CWD)      | yes           |
| pi.dev          | `.mcp.json`        | stdio     | `${VAR}`          | yes (CWD)      | yes           |
| MCP Inspector   | n/a (CLI args)     | stdio     | shell             | n/a            | yes           |

Cross-harness invariants:

- Tool surface — movement (`follow`, `engage`, `path_to`, `walk`, `cancel`),
  actions (`cast`, `weaponskill`, `job_ability`, `use_item`, `bank_when_full`),
  social (`chat`, `tell`), flow (`request_zone_change`, `end_event`,
  `raise_menu`, `tractor_menu`, `homepoint_menu`, `wait_for_event`), and
  session (`snapshot`, `debug_heights`, `read_resource`, `disconnect`).
  `ffxi-mcp/src/main.rs` is the list that counts — `tools/list` in the
  Inspector is the cheapest way to confirm what a given build exposes.
- Resource surface — 5 resources (`scene://current`, `party://members`,
  `diagnostics://session`, `goal://current`, `inventory://current`).
  `read_resource` re-exposes all five as a tool for clients without
  `resources/read`.
- Notifications — `notifications/resources/updated` fires on the `AgentEvent`s
  gated in `ffxi-mcp/src/main.rs::uris_for_event`.
- Working directory — start the harness from `ffxi-agent/` so `.mcp.json`
  resolves; it points at `../Cargo.toml -p ffxi-mcp`, compiling against the
  workspace root. `cargo test -p ffxi-client` etc. run from the repo root.

Observed gotchas:

- **OpenCode** may surface env-interpolation errors if your shell doesn't
  export the referenced variables before launch. Check with `env | grep FFXI_`.
- **MCP Inspector** doesn't auto-discover `.mcp.json` — pass the binary path
  and set env vars in your shell first.
- **All three** assume one MCP server per harness. Two harnesses against the
  same stack with the same `FFXI_USER` produce "char already logged in" from
  the lobby.

## 4. Verification scenarios

### 4a. Autonomous goal — 60-minute farming loop

Goal definition for the LLM (paste into the harness's first message):

> Farm crawler cocoons in West Sarutabaruta for 60 minutes; bank to mog
> house when inventory hits 30/30.

Mid-run, force a disconnect:

```bash
docker restart -t 0 server-map-1
```

The supervisor must reconnect and resume the persisted goal from the user
config dir (`kuluu/goal.json`, override with `FFXI_MCP_GOAL_PATH`). Watch for:

* `INFO ffxi_client::supervisor: supervisor.attempt.start attempt=2 replaying_goal=true`
* `INFO ffxi_client::supervisor: supervisor.reconnected attempt=2 downtime_ms=…`

### 4b. Co-play goal — agent-as-healer

Run a second character (a melee) under manual control, partied with the agent:

> Follow the party leader; cure them when their HP drops below 75%; cure
> on `/tell @cure`. Do not engage mobs.

Validation:

* `/tell @cure` from the leader → agent's reaction in party chat within ~1.5 s.
* Pull aggro onto the agent → `EngagedBy` event, harness re-prioritises.
* Walk away → reactor's `Follow` keeps stepping until back in range.

## 5. Reading the latency instrumentation

Tracing events fire at three layers, each at a level that keeps defaults quiet:

| Event                          | Level | Fields                                        |
|--------------------------------|-------|-----------------------------------------------|
| `reactor.tick`                 | trace | `elapsed_us`, `cmds_emitted`                  |
| `supervisor.attempt.start`     | info  | `attempt`, `replaying_goal`                   |
| `supervisor.attempt.end`       | info  | `attempt`, `duration_ms`, `outcome`           |
| `supervisor.reconnected`       | info  | `attempt`, `downtime_ms`                      |
| `mcp.tool_dispatch`            | debug | `kind`, `elapsed_us`, `ok`                    |
| `mcp.resource_read`            | debug | `uri`, `elapsed_us`                           |

To profile reactor ticks:

```bash
RUST_LOG=info,ffxi_client::reactor=trace cargo run -p ffxi-mcp …
```

Events are key=value, not JSON; switch to the `tracing-subscriber` JSON
formatter if you need machine parsing.

Budgets:

* Reactor decisions ≤ 250 ms p99
* MCP tool dispatch ≤ 50 ms p99 (excluding LLM time)
* Reconnect downtime ≤ 8 s p95 on transient drops; ≤ 90 s on a hard crash

That last split is forced by the UDP floor. `net_health::MAP_SILENCE_TIMEOUT`
declares a disconnect after 60 s without any inbound server packet
(`ffxi-client/src/net_health.rs:4`, enforced at `session.rs:3517`) — UDP gives
no socket-level "connection lost" signal, so silence detection is the only
mechanism. On a hard map-server crash expect ~60 s to notice, ~5–10 s to
re-auth and re-zone, ≥ 65 s total. Lowering the threshold (15 s = 15 missed
1 Hz keepalives) would trade robustness against short server stalls for faster
recovery; we kept 60 s and moved the target instead, which is why
`disconnect_recovery` asserts a 60 s recovery ceiling rather than 30 s.

## 6. Live-calibration caveats

Dating from Stage 7 and not re-confirmed against a live run since — treat as
"unproven", not "broken":

* **Heading math** (`reactor::heading_toward`) — n=0/e=64/s=128/w=192, pinned
  by `heading_toward_pins_cardinal_quarters`, but the unit test can't rule out
  a constant offset versus what the server expects. Test by issuing `move`
  north and watching the character in another client.
* **`RequestZoneChange`** — packet builder unit-tested; server-side acceptance
  not observed in a live run.
* **BtTargetID offset** — `body[40..44]` per
  `Phoenix/src/.../char_update.cpp:187`, feeding the reactor's `n_id`.
* **`/tell` layout** — `unknown00` / `unknown01` modelled per
  `Phoenix/src/map/packets/c2s/0x0b6_chat_name.h`.
