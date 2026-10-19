# Host: macOS

Use Wine for client observation on macOS.
Discover the runner on PATH or select the runner from an existing local profile.
Keep the selected runner and Wine prefix together.
`scripts/observe.sh` uses CGWindowList, `screencapture -l`, CGEvent, and Vision for the host operations.
Check permissions with `scripts/observe.sh doctor` before driving a client.

| Arrangement | What owns the window | Set up |
|---|---|---|
| **Wine** (no VM) | the Wine process, usually named `wine` | nothing, or `FFXI_OBSERVE_RUNNER=wine` for `launch` |

No named VM or external volume is a prerequisite.
The backend retains optional VM support for an explicit user request.

## Packaged macOS procedure

Use `scripts/macos.py` for the Wine keyboard and capture procedure.
The same command works from Codex, Claude Code, or a terminal.
It requires Python 3.9 or later and the standard macOS tools.
It does not require jq, an OCR engine, or a Python package installation.
Jev is optional.

### Prepare the machine

1. Install a Wine build that can run FFXI on this Mac.
2. Prepare a Wine prefix with the client registrations described below.
3. Install your game files outside the checkout.
4. Obtain an xiloader version compatible with your client and server.
5. On Apple Silicon, install Rosetta when the selected Wine build requires it.
6. Give the calling application Screen Recording permission.

The package does not install game files, Wine, Rosetta, or client registry entries.
Those components require a compatible installation before observation can work.
The package records the selected Wine, loader, and client hashes.
A changed executable causes a clear failure until setup records the verified update.
The Wine executable hash does not cover every library in its distribution.

### Save local configuration

Run this command from the skill directory.
Replace the example paths with paths on the current machine.

```bash
python3 scripts/macos.py --profile local setup \
  --runner /absolute/path/to/wine \
  --prefix /absolute/path/to/wine-prefix \
  --install '/absolute/path/to/FINAL FANTASY XI' \
  --loader /absolute/path/to/xiloader.exe \
  --server 127.0.0.1 \
  --loader-arg=--hairpin
```

Setup downloads the official AutoHotkey v2.0.30 archive and verifies its SHA256.
The pinned archive hash comes from the [official release](https://github.com/AutoHotkey/AutoHotkey/releases/tag/v2.0.30).
For offline setup, add `--ahk-archive /path/to/AutoHotkey_2.0.30.zip`.
Setup verifies the cached executable before each operation.
It also copies the loader into the user cache.
The resulting profile does not depend on files in a temporary checkout.

Configuration goes to `~/.config/ffxi-observe/local.macos.json` with permissions `0600`.
Cached executables go to `~/.cache/ffxi-observe/`.
The command honors `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`.
Run setup on each machine instead of copying a profile with absolute paths.
Add `--replace` after checking an intentional executable update.
Setup does not change an existing Wine prefix or stop a running client.

Keep credentials outside the profile.
For launch, set `FFXI_USER` and `FFXI_PASS` in the environment.
The launch command passes them to xiloader without printing its arguments.
They can still appear in local process arguments while the loader runs.

```bash
python3 scripts/macos.py --profile local doctor
python3 scripts/macos.py --profile local launch
python3 scripts/macos.py --profile local capture /tmp/ffxi-before.png
python3 scripts/macos.py --profile local key menu 0.2
python3 scripts/macos.py --profile local capture /tmp/ffxi-menu.png
python3 scripts/macos.py --profile local key esc 0.2
```

Inspect the captures to verify that the menu opened and closed.
A successful input command proves submission, not the visible result.
The doctor command checks dependencies, permissions, executable hashes, Rosetta, and the current window.
It does not prove login, network access, or client registration correctness.
Capture and input require one visible FFXI window.
If no game window exists, doctor reports a null window.
Launch the client before the visible input check.

Input checks reject a locked screen, ambiguous windows, unknown keys, and holds longer than five seconds.
A prefix lock prevents concurrent input from these packaged commands.
Other tools and human input can still interfere.
Use one driver at a time.
Windows activation does not guarantee that macOS foreground focus stays unchanged.
Background operation remains unverified.

### Optional Jev observation and native vision fallback

Store the Jev key in `~/.config/typesafe/api-key` with permissions `0600`.
Keep its parent directory private.
To select another key file, add `--jev-key-file /private/path` during setup.
The profile stores the key path, not its value.
Capture and keyboard commands do not need this key.

```bash
python3 scripts/macos.py --profile local decide /tmp/ffxi-menu.png --character 'Exact Name'
```

The command resizes a temporary copy to meet Jev image limits.
It preserves the original capture and records both image hashes.
Only `decide` sends a capture to Jev.
Jev image support remains experimental, as stated in the [image documentation](https://docs.typesafe.ai/images).
The model is pinned to `jev-1.14.0`.

If Jev is missing or fails, the command requests native vision.
It also requests vision when the decision confidence is below `0.9`.
It returns `status: needs_vision`, the capture path, its hash, and a JSON schema.
It also saves the request beside the capture as `.decision.json`.
The command sends no input and does not retry indefinitely.
Use `--vision` to select this path without calling Jev.

When `needs_vision` appears, open the named capture with the current agent's image tool.
Alternatively, give the request and capture to a vision-capable subagent.
Require the subagent to return only the specified JSON.
Do not give the subagent input control.
If no image tool is available, report the missing capability and stop.

Save the returned JSON to a temporary file.
Validate it against the capture before use:

```bash
python3 scripts/macos.py --profile local decide /tmp/ffxi-menu.png \
  --character 'Exact Name' --result /tmp/vision-result.json
```

Validation rejects a different image hash, unsupported fields, and unsafe Enter decisions.
The login decision format permits only `enter`, `wait`, or `stop`.
It does not control general gameplay or accept agreements.
Preserve existing session approval for separate, permitted agreement handling.
Before input, inspect a current capture and check that the proposed action still applies.
Neither Jev nor fallback decisions execute automatically.

### Local OrbStack LSB configuration

The optional `scripts/lsb-retail.compose.yaml` supplies a separate connect server and profile server.
It requires an existing LSB database, world server, map server, and Docker network.
It pins the server image used for the verified local login.
Check that your existing stack uses a compatible server revision and database schema.
It publishes service ports only on `127.0.0.1`.

Set these values for the existing stack:

```bash
export LSB_NETWORK=your-lsb-project_default
export LSB_SQL_PASSWORD=your-local-database-password
# Optional: LSB_SQL_HOST, LSB_SQL_DATABASE, LSB_SQL_LOGIN, LSB_WORLD_HOST
# Default client-version lock: 2
# Use LSB_VER_LOCK=0 only for an intentionally older local test client.
docker --context orbstack compose -p retail-observe \
  -f scripts/lsb-retail.compose.yaml config --quiet
docker --context orbstack compose -p retail-observe \
  -f scripts/lsb-retail.compose.yaml up -d
```

Do not start this supplement when another server already occupies these ports.
The connect server uses internal auth port `57432` to avoid the default server's routing identifier.
Use these additional setup arguments for this supplement:

```bash
--loader-arg=--authport --loader-arg=57432 \
--loader-arg=--viewport --loader-arg=57003 \
--loader-arg=--dataport --loader-arg=57433
```

The profile server uses ports `51220` and `51240`.
The existing stack still supplies map and search services.
The supplement does not create accounts or update database zone addresses.
Check the existing stack configuration before launch.

### Verification scope

On 2026-10-19, the packaged input command opened the FFXI menu on Apple Silicon.
The later capture still showed a menu, so menu closure was not verified.
The captures used the existing HorizonXI 2023 client in Port San d'Oria on local OrbStack LSB.
Jev and a separate vision subagent identified gameplay.
Tests cover corrupt downloads, credential permissions, input guards, ambiguous windows, and structured fallback validation.
A clean installation on another Mac remains unverified.

## Host permissions -- the two silent failures

The terminal running `observe.sh` needs, in System Settings > Privacy &
Security:

- **Screen Recording**, or every capture comes back black;
- **Accessibility**, or every key and click is silently dropped.

Both fail quietly, which is why `observe.sh doctor` preflights them
(`CGPreflightScreenCaptureAccess` / `AXIsProcessTrusted`) instead of letting you
misread the result as a broken window match. A sleeping display also captures
black: `observe.sh` runs `caffeinate -u` before each capture, and for a long
session keep `caffeinate -d -u` running.

## Wine

Community wrappers exist because a plain Wine build is not the hard part -- the
client's DirectX 8 rendering is. Prefer one of them over assembling your own:

- **FFXI on Mac** (`danielalanbates/HorizonXI-on-Mac`) -- Apple Silicon, macOS 13+,
  ships its own Wine build and downloads a client per world, with separate game
  folders and logins per server. Roughly 24-28 fps at 4K on an M1, and a few
  Ashita plugins fail to load harmlessly.
- **Whisky** or **CrossOver** -- general Wine wrappers; you supply the client.
- **Plain `wine`** -- workable for the launcher and tools; the game's D3D8 path is
  the part that needs a wrapper's attention.

### The wine binary is a variable, not a constant

FFXI on Mac ships a **patched CrossOver build** (`athei/wine-build`, the
`cx-*-macos-x86_64` releases) and treats it as mandatory, because a stock build
"exits one second after login" -- its `docs/WINE-BUILD.md` measures the two side
by side in one prefix. It is a self-contained tree: unpack it and point the
profile's `runner` at its `bin/wine`, with `WINEPREFIX` in the environment.

Two things follow for this skill. **Give it its own prefix.** A prefix built by
one wine build and then opened by another spends minutes in `wineboot` and can
wedge there; a fresh prefix boots in about two. And **`show` starts working**:
`System Events` can raise the CrossOver build's windows, where the Homebrew
build exposes no AX windows at all and reports the terminal as frontmost no
matter what you do -- which is worth knowing before you go hunting a focus bug
that belongs to the wine build.

A fresh prefix needs the client's own registry facts put back, none of which
come from the install: the PlayOnline `InstallFolder` and `Interface` values
(see the traps below), `regsvr32` for the viewer's `polcore.dll` and the game's
`FFXi.dll` / `FFXiMain.dll` / `FFXiVersions.dll`, and the
`PlayOnlineUS\SquareEnix\FinalFantasyXI` settings key -- with no settings key
the client exits at boot exactly as if the interface check had failed. Those
registrations are per-install, so switching the prefix between two installs
means re-running `regsvr32` for the one you want.

What was verified on this host (macOS 26, Apple Silicon, Homebrew `wine-stable`
11.0, driving a Win32 app under Wine):

- the CGWindow **owner is `wine`**, and the **title is the app's own Win32 title**
  -- so the default `FINAL FANTASY` title match finds the game with no config;
- `screencapture -l` captures the Wine window correctly, including the 2x Retina
  scale factor that `capture` reports;
- CGEvent input worked in a Win32 test application. This does not prove FFXI
  accepts the same input. Use the Wine keyboard procedure below if FFXI ignores it;
- **OCR coordinates land where they should**: `click-text` on a word moved the
  app's caret into that word, which is the end-to-end check of the
  pixel-to-point conversion;
- `System Events` lists the GUI process as `wine`, which is how `show` focuses
  it when the window's owning pid is a child process it cannot address.

### Wine keyboard input

If host input does not reach FFXI, use AutoHotkey inside the same Wine prefix.
This method does not require the Wine window to expose accessibility elements.
Use an official AutoHotkey v2 executable.
Set `FFXI_OBSERVE_AHK` to its absolute path.
The helper uses the Win32 key codes in `keys.tsv`.

```bash
export WINEPREFIX=/path/to/prefix
export FFXI_OBSERVE_RUNNER=/path/to/wine
export FFXI_OBSERVE_AHK=/path/to/AutoHotkey64.exe
scripts/observe.sh wine-key enter 0.2
scripts/observe.sh capture /tmp/ffxi-after.png
```

The helper requires one matching Wine window.
It activates that window inside Wine before sending the key.
Inspect the next capture to verify the result.
Windows activation does not prove that macOS foreground focus stays unchanged.
Do not claim background input support without a separate focus test.

On 2026-10-19, AutoHotkey v2.0.30 confirmed character selection in the FFXI client under Wine.
The macOS CGEvent input methods did not change that screen.

Four Wine-specific traps:

- **The game exits about a second after login, with no window and no error.**
  This is not the renderer: under `WINEDEBUG=+d3d8` the client makes no D3D
  calls at all, so it quits before graphics. It is FFXI's `patch.ver`
  interface-id check reading `HKLM\SOFTWARE\PlayOnlineUS\Interface`, and it
  needs all three of: the values present in the **32-bit view**
  (`Software\Wow6432Node\...`, which is the one a 32-bit client reads -- having
  them only in the 64-bit view reads as "absent"), `0001` holding a real
  interface id rather than `"0"`, and `0002`/`1000` likewise. A private-server
  install ships those values in its own registry script; run its NSIS installer
  unattended (`wine <installer>.exe /S`) rather than clicking through it. A
  server whose client expects Ashita's Sandbox to fake the check instead will
  write `0001="0"`, and then the client only boots under Ashita.
- **A registry edit does not reach the next launch while `wineserver` lives.**
  The running server holds the pre-edit registry, so the game reads the old
  values and the change looks like it did nothing -- which turns one wrong
  value into an afternoon of contradictory results. `wineserver -k` between the
  edit and the launch, every time.
- **A window can outlive its app.** Kill `wineserver` (or the app hangs) and
  macOS keeps compositing the last frame: the window still resolves, captures
  fine, and accepts no input. The tell is OCR that is byte-identical across
  several inputs while a caret or counter never moves. Relaunch rather than
  debugging the input path.
- **First launch in a fresh prefix is slow.** Prefix bootstrapping can take tens
  of seconds before any window appears. Poll `observe.sh status` rather than
  concluding the launch failed, and suppress the Mono/Gecko installer prompts
  with `WINEDLLOVERRIDES="mscoree,mshtml="` when scripting a bootstrap.

## Optional VM support (explicit user request)

Use this section only when the user explicitly requests an existing VM.
The default observation procedure uses Wine.
Set `FFXI_OBSERVE_VM_NAME` so a console window titled after the VM can be
matched when the game window itself is not separately visible.

### Coherence mode -- why the window looks like a macOS app

In Parallels Coherence, guest windows are blended onto the macOS desktop. That
inverts a few intuitions:

- The game window's CGWindow **owner is the guest exe** (`horizon-loader.exe`),
  not `prl_vm_app`. Matching on the title rather than the owner is what makes
  this work by default.
- The title carries **trailing NUL bytes** (`FINAL FANTASY XI\0\0`). Matching is
  substring, so this is harmless -- but never match on exact equality.
- `screencapture -l` grabs the window **even across Spaces and when macOS reports
  it off-screen**, so `capture` works without fighting the window forward.
  `show` (needed only for input focus) raises the guest app; frontmost then
  reads as **`WinAppHelper`**, the Parallels input shim. That is success, not
  failure -- keys and clicks still land in the guest.
- Classic (non-Coherence, full-screen) windows go through `prl_vm_app` normally.

### Parallels Standard has no guest-side tooling

`prlctl exec` and `prlctl capture` are Pro/Business features. That is the
original reason this whole skill goes through the window server instead of a
guest agent -- and the reason the same code now works for Wine. Do not reach for
`prlctl exec`; it will simply error.

### Getting from the guest desktop to the game

Windows desktop -> the server's launcher -> the FFXI loader. From the host you
can start either through its Parallels app stub:

```bash
open ~/"Applications (Parallels)"/*Applications.localized/"HorizonXI-Launcher.app"
```

The game window itself is `horizon-loader.exe.app` in that same folder. A cold
start runs: PlayOnline user agreement (**Accept**) -> "Acquiring FINAL FANTASY XI
server data" -> character select. Character select is the login handoff point.

Launcher windows are usually titled differently from the game, so drive them
with an override, e.g.
`FFXI_OBSERVE_WINDOW_TITLE='HorizonXI Launcher' observe.sh ocr`.

### VM-specific gotchas

- **Launch tickets expire within seconds.** Click the launcher's "Ready! Click to
  Launch" fast (or fire `--bg` clicks in a tight loop). A crashed prior session
  shows "Ticket not found, expired, or already used" until the server drops the
  ghost session -- wait it out rather than logging out, since re-login needs the
  account password.
- **A slow disk stalls the client's UI thread**: the title shows "(Not
  Responding)" during loads and input sent then is *dropped*. The 3D background
  keeps animating on another thread, so a live capture is not proof of a
  responsive client. Send confirm keys only once the title clears.
- **Windows Update can restart the VM** mid-session (watch for "We've got an
  update for you" -- dismiss with **Another time**), which loses the login.

## Troubleshooting

| Symptom | Cause -> fix |
|---|---|
| Black or empty capture | Screen Recording not granted, or the display is asleep -> `doctor`, and keep `caffeinate -d -u` for long sessions |
| Keys/clicks silently ignored | Accessibility not granted, or the window is not frontmost -> `doctor`, then `show` |
| Captures and OCR keep working, but no input has any effect, and `show` reports some other app frontmost | The Mac is **locked**. The window server still hands `screencapture -l` the client's window, so every read-only verb looks healthy while the login window swallows all synthesized input. `doctor` reports the lock and the input verbs refuse. Unlock the machine; nothing on the agent side can work around it |
| Input lands in the terminal instead of the game | A raise that happened in an *earlier* invocation. Every input verb re-raises in-invocation; do not batch keys around a single `show` |
| OCR identical across several inputs, caret/counter frozen | Wine app orphaned from its `wineserver` (stale composited window) -> relaunch the client |
| No window matches, but the client is running | It is titled differently (launcher, config tool) -> `targets`, then set `FFXI_OBSERVE_WINDOW_TITLE` |
| VM `running` but no guest window exists at all, and raising does nothing | Coherence with zero guest windows open -- there is nothing to raise and launching an app stub silently no-ops. Open the VM console (`open "<path>/<vm>.pvm"`), which is owned by **`Parallels Desktop`** and titled after the VM: set `FFXI_OBSERVE_VM_NAME`. In this mode the game window lives INSIDE the console, so `capture` grabs the whole guest desktop (crop before reading), `ocr` picks up desktop clutter, and click coordinates are console-relative |
| Every window query fails, which reads as "wrong Space" | A JXA error, not a missing window -- rerun `host_windows_json` without `2>/dev/null`. A regex containing a space once word-split the osascript arguments; the owner regex is passed as argv for exactly this reason |
| `show` never raises in a VM | Some installs expose no System Events process named "Parallels Desktop", only `prl_client_app`/`prl_vm_app`. The raise cascade tries all of them plus a direct AXRaise |
| Frontmost reads as `WinAppHelper`/`prl_vm_app` after `show` | Expected in Coherence -- that shim holds focus for the guest; input still lands |
| Loader opens no game window | The client's background resolution is too high -- 4096 squared wedged it, 2048 squared works. Fix it in the launcher or FFXI Config, not here |
| Clicks land off-target | Pixel coordinates used as points -> divide by the scale factor `capture` prints |
| Game ignores held keys | Hold shorter than the input poll -> use at least 0.1s |
| Clicks are OCR-verified and land, but nothing happens; the client closes by itself | Another session (human or agent) is driving the same client concurrently, and the inputs interleave. Do not fight for focus: stop, report the contention, agree who owns the client |
| A native Win32 popup ignores input entirely (e.g. the standalone gamepad config tool) | Posting to a pid does not reach guest-spawned native dialogs -> fall back to `osascript` `System Events` `key code ...` after activating the VM app; answer any "Save changes?" prompt No unless changes were intended |
