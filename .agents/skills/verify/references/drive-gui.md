# GUI drive: native window + agent socket

For changes observable only in pixels/audio: rendering, HUD, camera, minimap,
materials, input-driven movement.

**Drive focus-free by default.** Verification runs on the human's own desktop
while they are using it. Everything except menu navigation can be driven and
captured without the window being frontmost, so stealing focus is a choice you
make deliberately for the few things that need it — not the default posture.
The one condition you cannot escape: macOS stops rendering a **fully occluded**
window, so *some* part of the client must stay on screen. Any sliver is enough;
focus is not.

## Hold the display awake FIRST

```bash
caffeinate -d -u -t 5400 &      # before launch, held for the whole session
```

Skipping this is the single most common way a GUI drive dies. When the host
display sleeps or detaches, winit logs `Monitor removed <id>` and the window
count drops to zero, which Bevy treats as a normal quit — so the client exits
through its full teardown path with **no panic and no error**, and the log tail
looks like a clean voluntary shutdown rather than a fault:

```
bevy_winit::system: Monitor removed 675v0
bevy_window::system: No windows are open, exiting
ffxi_client::view_native::exit_watchdog: teardown checkpoint stage="app.run() returned — winit loop exited cleanly"
```

Read that signature as *the display slept*, not as a client bug — it has cost
whole verification sessions to misdiagnose. `-u` (simulate user activity) is
needed alongside `-d`: once a monitor has actually been removed, `scripts/
capture.sh` returns blank frames until a real wake, so an already-asleep
display must be woken, not merely kept awake from that point on.

## Launch with the agent socket

```bash
.agents/skills/verify/scripts/launch.sh /tmp/verify-client.log
```

That is the whole launch. It uses the local GM drive account, passes
`--unfocused --mute`, waits for map traffic, and prints the agent socket path.
Export `FFXI_VERIFY_SOUND=1` to keep audio on when the change under test is
audio; `FFXI_VERIFY_USER`/`_PASS`/`_CHAR` override the character.

It also hands focus back to whatever app was frontmost. This matters: macOS
activates a newly launched app at the *process* level, which winit's
`focused: false` does not suppress — so a bare launch yanks the user out of
full-screen video even with `--unfocused`. Restoring afterwards is the only
lever available from outside the client, because Bevy builds the winit event
loop itself and exposes no macOS `ActivationPolicy` hook. `--unfocused` is
still worth passing: it keeps the window from being made key, so the blip is
shorter.

Doing it by hand instead — **do not ask the user for credentials** (see
SKILL.md "Character strategy"):

```bash
target/debug/ffxi-client --agent-listen auto play --unfocused --mute \
  verilight 'TestPass!1234' Verilamp
```

`verilight`/`TestPass!1234`/`Verilamp` (gmlevel 5) is this machine's throwaway
dev account — a documented local example credential, not a secret; type it
into launch commands freely. Use the user's real env-var account only when the
check needs their own character.

Credentials are **positional args to `play`** — the GUI path reads no
`FFXI_USER`/`FFXI_PASS`/`FFXI_CHAR` env vars (only headless test fixtures
do); launching without them leaves the in-window launcher waiting for input
while the log looks alive (zone geometry loads behind the launcher).

`--agent-listen auto` writes `$TMPDIR/ffxi-agent.pid` with the unix-socket
path — but that file goes stale across the cargo-wrapper→binary re-exec and
after a dead run. Resolve the socket from the client log instead
(`grep -ao "/var/folders[^ ]*ffxi-agent-[0-9]*\.sock" <log>`) or glob
`$TMPDIR/ffxi-agent-*.sock` newest-first. The GUI session runs the full
reactor, so goals work.

Launch it with the harness's background mechanism (`run_in_background`), not a
detached `&` subshell — a subshell-detached client gets reparented and its log
stops growing mid-run.

## What is focus-free (almost everything)

| Need | Command | Notes |
|---|---|---|
| Session state, chat, GM `!cmds`, actions, zoning | agent socket `AgentCommand` | pure IPC, never touches the window |
| Movement through the real `input.rs` path | `debug_drive` / MCP `walk` | kuluu-0pof; exercises heading, wall-slide, re-ground |
| Grounding numbers | `debug_heights` / MCP `debug_heights` | logged under `tracing target: debug_heights` |
| **Screen capture** | `scripts/capture.sh <out.png>` or MCP `screenshot` | kuluu-wwwv; GPU readback, see below |

### Capture

```bash
.agents/skills/verify/scripts/capture.sh artifacts/verify/<what>.png
```

This sends `{"cmd":"screenshot","path":...}` over the socket, firing the same
`ScreenshotRequest` the `/screenshot` slash command does. Bevy captures by
reading the render target back off the GPU (`copy_texture_to_buffer` +
`map_async`), so unlike `screencapture -l <window_id>` it needs no Screen
Recording permission, never raises the window, and cannot hand back the stale
cached frame the window server keeps for a background window. Output is the raw
client frame at backing resolution — no macOS title bar to crop around.

The write is async, so the script waits for the file, then **asserts the frame
isn't blank**. A fully occluded (or `Hide`-den) client renders nothing and the
readback is solid black — a perfectly valid PNG of nothing, which is exactly
the kind of silent failure that gets cited as evidence by mistake. On a blank
frame it raises the client once, re-captures, and hands focus straight back,
logging `FOCUS WILL BLIP` so you know the human was interrupted. Correct
evidence beats zero disruption; a ~1s blip is cheaper than a black PNG being
cited as proof. If it is *still* blank after raising, the console is probably
locked — that exits 2 and no artifact from it is citable.

Launching unfocused makes this fallback more likely, since nothing guarantees
the window ends up visible. Leaving the client somewhere it stays partly
on screen (a free corner, a second display) avoids the blip entirely.

Read every PNG back with the Read tool before citing it. A guard reporting
`lit=100%` only proves the GPU drew *something*.

### Talking to the socket

Use a one-shot Python `AF_UNIX` client with `settimeout()` and an explicit
`close()`. Do **not** shell out to `nc -U`: BSD `nc` has no reliable
idle-timeout, blocks past `-w`, gets backgrounded by the harness, and the
abandoned connection holds the socket's single-peer slot open so every later
send silently no-ops until you kill the stray process.

`AgentCommand` fields are exact and a wrong key is dropped **silently** — the
socket accepts the line, nothing errors in the client or map log, and the
command never happens. `chat` is `{"cmd":"chat","kind":0,"text":"!hp 9999"}`;
sending `message` instead of `text` deserializes to nothing. Confirm a GM
command actually landed (re-`snapshot`, check the value moved) before
concluding the server rejected it. Variant names come from `AgentCommand` in
`ffxi-client/src/state.rs` — read the enum rather than guessing. `ActionKind`
is internally tagged, so a cast nests as
`{"kind":"cast_magic","spell_id":896,…}` inside the `kind` field.

## What still needs focus

**Menu navigation and anything typed.** The socket carries session-level
commands, not keystrokes, so the target-action menu, main menu, chat bar, and
Tab-targeting need real key events through System Events — which requires the
process frontmost:

```bash
osascript -e 'tell application "System Events"
    set frontmost of (first process whose unix id is <pid>) to true
    delay 0.6
    key code 48    -- Tab (36 Enter, 53 Esc, 125/126/123/124 arrows, 27 main menu)
end tell'
```

Resolve `<pid>` with `pgrep -f "^target/debug/ffxi-client"` — a bare
`pgrep -f ffxi-client` also matches the harness's own shell wrapper, and the
`osascript` then fails with "Invalid index". Needs Accessibility permission.
Keep delays ≥0.3s, capture after each step, and Read the result — keystrokes
are fire-and-forget.

Use `key code`, not `keystroke`, for keys the client binds physically:
`keystroke "/"` arrives as a text-insertion event, misses the client's
`KeyCode::Slash` chat-open binding, and falls through to in-world hotkeys
(observed: it opened the Job Abilities menu instead). `key code 44` (physical
Slash) works. Letters typed *into an already-open* text field are fine as
`keystroke`.

Because this steals focus, batch the keystroke legs of a run together instead
of interleaving them with focus-free work, and warn the user before you start
taking over their keyboard.

**Movement *feel*** (wall-slide, re-ground) and **chase-camera orbit/zoom feel**
still need human eyes. Set the scene up over the socket, then hand off with
exact instructions — "walk into the north wall and watch whether the camera
clips into your head" beats "check the camera". Socket `move` teleports the
session position and bypasses the input layer, so it does NOT exercise
input-driven bugs; use `debug_drive` for those.

## Gotchas

- macOS: the Bevy/winit loop owns the OS main thread; the window opens on the
  user's desktop — tell them before spawning it.
- Bevy's unfocused update mode is `reactive_low_power` at 60Hz, so a background
  window keeps rendering and stays capturable. A **hidden** app (`Hide`, or
  System Events `set visible to false`) does not, and un-hiding via
  `set visible to true` often doesn't stick — use `set frontmost … to true`.
- **Console lock kills everything visual**: if the macOS session locks
  (`CGSSessionScreenIsLocked=1` via `Quartz.CGSessionCopyCurrentDictionary()`),
  captures go black and System Events sees 0 windows regardless of TCC grants.
  Check this first when captures are blank; only a human unlock fixes it.
- Ghost sessions: prefer a clean socket `disconnect` over `kill`. The map server
  holds a killed char for 2–5 min and the next login times out. To clear one:
  `DELETE FROM accounts_sessions WHERE charid=<id>` (Verilamp is 17455719).
  That table's `targid` column is also the authoritative live targid —
  `charid & 0x7FF` is **not** it, and the server rejects actions built on it.
- Agent-socket `chat` bypasses the client's local `/`-command parser and sends a
  raw wire SAY. Server-side `!` GM commands work; client-side `/` commands need
  real keystrokes.
- Agent-socket `move` persists server-side and can be clamped back to the
  navmesh's nearest valid vertex — position telemetry echoes what you asked for
  while the rendered transform snaps back. Vary `x` as well as `z` if a teleport
  looks stuck.
- One GUI client at a time — it holds the char's session, and a parallel
  headless login with the same char fights it.
- **Don't run `scripts/checks.sh test` while a GUI session is live** — the
  `agent_session` integration test logs into the same LSB and kicks the running
  session mid-verify. Gate first, then launch.
- Known intermittent: a `slab_allocator Use-after-free` burst at zone-in can
  black out all zone geometry for the whole session (kuluu-172i); relaunch once
  before diagnosing a rendering change.
