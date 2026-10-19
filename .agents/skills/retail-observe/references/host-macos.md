# macOS Wine

Use `scripts/macos.py` from this skill directory for the Wine procedure.
It requires Python 3.9 or later and standard macOS tools; it requires no pip packages or jq.
Jev is optional. Native vision can inspect the same captures.
The keyboard helper uses AutoHotkey inside the game's Wine prefix.
Host CGEvent input did not control the tested FFXI client.

## Setup

Supply a working Wine build, its prepared prefix, your game installation, and a compatible xiloader executable.
On Apple Silicon, install Rosetta if the Wine build requires it.
Grant Screen Recording permission to the calling application.
The Wine keyboard path does not require macOS Accessibility permission.
Setup does not install Wine, game files, or client registry entries.

```bash
python3 scripts/macos.py --profile local setup \
  --runner /absolute/path/to/wine \
  --prefix /absolute/path/to/prefix \
  --install '/absolute/path/to/FINAL FANTASY XI' \
  --loader /absolute/path/to/xiloader.exe \
  --server 127.0.0.1 --loader-arg=--hairpin
```

Setup saves `~/.config/ffxi-observe/local.macos.json` with permissions `0600`.
It caches the loader and AutoHotkey under `~/.cache/ffxi-observe/`, outside the checkout.
It honors `XDG_CONFIG_HOME` and `XDG_CACHE_HOME`.
It verifies AutoHotkey v2.0.30 against the [official release checksum](https://github.com/AutoHotkey/AutoHotkey/releases/tag/v2.0.30).
For offline setup, add `--ahk-archive /path/to/AutoHotkey_2.0.30.zip`.
Run setup on each machine; absolute paths are local configuration.
If a recorded executable changes, verify the update before running setup with `--replace`.
The Wine executable hash does not cover every library in its distribution.

Keep credentials outside the profile.
Set `FFXI_USER` and `FFXI_PASS` in the environment before `launch`.
The command does not print loader arguments, but credentials can appear in local process arguments.

## Capture and input

```bash
python3 scripts/macos.py --profile local doctor
python3 scripts/macos.py --profile local launch
python3 scripts/macos.py --profile local capture /tmp/ffxi-before.png
python3 scripts/macos.py --profile local key menu 0.2
python3 scripts/macos.py --profile local capture /tmp/ffxi-menu.png
python3 scripts/macos.py --profile local key esc 0.2
```

Use `launch` only when the client is not running.
Inspect captures to verify each visible result.
A successful key command proves submission, not the game's response.
Capture and input require one visible FFXI window.
The capture result includes the pixel dimensions, window dimensions, and scale.
Input rejects a locked screen, ambiguous windows, unknown keys, and holds longer than five seconds.
A prefix lock excludes concurrent packaged inputs; other tools and human input can still interfere.
Background operation without macOS focus changes remains unverified.

## Jev or native vision

For Jev, store the key in `~/.config/typesafe/api-key` with permissions `0600` and a private parent directory.
Use `--jev-key-file /private/path` during setup to select another key file.
Capture and input do not require this key.

```bash
python3 scripts/macos.py --profile local decide /tmp/ffxi-menu.png --character 'Exact Name'
```

Only `decide` sends an image to Jev.
It uses the pinned model `jev-1.14.0` and resizes a temporary image to the API limits.
Jev image support remains [experimental](https://docs.typesafe.ai/images).
Missing credentials, API failures, invalid responses, and low confidence produce `status: needs_vision`.
The result supplies a capture path, hash, instructions, and JSON schema.
Use `--vision` to select native vision without an API call.

Open the capture with the current agent's image tool, or give the request to a vision-capable subagent.
If vision is unavailable, stop and report the missing capability.
Save the returned JSON and validate it:

```bash
python3 scripts/macos.py --profile local decide /tmp/ffxi-menu.png \
  --character 'Exact Name' --result /tmp/vision-result.json
```

The decision format covers login only: `enter`, `wait`, or `stop`.
Neither provider accepts agreements or sends input automatically.
Use existing session approval for separate agreement handling when the harness permits it.

## Optional local OrbStack server supplement

`scripts/lsb-retail.compose.yaml` adds connect and profile services to an existing compatible LSB stack.
The database, world, map, search, account, and reachable zone addresses must already exist.
The supplement pins the server image used for the local login; check compatibility with the existing database schema.
Do not start it if another service occupies its ports.

```bash
export LSB_NETWORK=your-lsb-project_default
export LSB_SQL_PASSWORD=your-local-database-password
# Optional: LSB_SQL_HOST, LSB_SQL_DATABASE, LSB_SQL_LOGIN, LSB_WORLD_HOST
# LSB_VER_LOCK defaults to 2; use 0 only for an intentionally older local test client.
docker --context orbstack compose -p retail-observe \
  -f scripts/lsb-retail.compose.yaml config --quiet
docker --context orbstack compose -p retail-observe \
  -f scripts/lsb-retail.compose.yaml up -d
```

Add these arguments to setup for this supplement:

```bash
--loader-arg=--authport --loader-arg=57432 \
--loader-arg=--viewport --loader-arg=57003 \
--loader-arg=--dataport --loader-arg=57433
```

The distinct internal auth port avoids the default connect server's routing identifier.
Profile ports are `51220` and `51240`. Published ports bind only to `127.0.0.1`.

## Setup diagnostics and verification limits

Keep a Wine build and its prefix together; switching builds can stall prefix initialization.
A prepared prefix needs the client's PlayOnline install/interface registry values and FFXI settings.
Register `polcore.dll`, `FFXi.dll`, `FFXiMain.dll`, and `FFXiVersions.dll` for the selected installation.
The interface values must exist in the 32-bit registry view and contain the installation's real interface identifiers.
Use the installation's registration procedure rather than guessed values.
See [client-and-server.md](client-and-server.md) for loader and client-version compatibility.

`doctor` checks local dependencies and permissions; it does not prove login or correct client registration.
If no window exists, it reports a null window; launch before the visible input check.
If capture fails, check Screen Recording permission and the display state.
If inputs have no visible effect, stop before diagnosing a stale window or another active driver.
For explicit VM or host OCR/pointer work, use `observe.sh help` and `observe.sh doctor`.
That separate host input path requires Accessibility permission and uses shell `.conf` profiles.

On 2026-10-19, packaged Wine input opened the HorizonXI 2023 client's menu on Apple Silicon against local OrbStack LSB.
The captures were inspected. Jev and a separate vision subagent identified gameplay.
A clean installation on another Mac and background input remain unverified.
