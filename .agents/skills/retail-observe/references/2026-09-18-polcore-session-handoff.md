# How polcore hands the session to FFXiMain (2026-09-18)

Static inspection only. No Square Enix binary was executed, patched or
modified; no traffic was sent to any production host. This record answers the
question the lobby login raises: where do retail's 16-byte packet hash and
64-byte authCode come from, and can a third-party client obtain them without
touching the viewer's process?

Builds inspected, from the registered `retail` install (`version.dat` reads
`1.18.00n`, no trailing newline). All are PE32 at image base `0x10000000`.

| Module | SHA-256 prefix | Size |
|---|---|---|
| `PlayOnlineViewer/pol.exe` | `5c2d45bd277e` | 1600512 |
| `PlayOnlineViewer/polhook.dll` | `e4ba164a0e3a` | 45056 |
| `PlayOnlineViewer/viewer/com/polcore.dll` | `f5af5837ffef` | 541696 |
| `PlayOnlineViewer/viewer/com/app.dll` | `bcddc16de7ff` | 4274688 |
| `FINAL FANTASY XI/FFXiMain.dll` | `f2245d1c9d06` | 2901584 |
| `FINAL FANTASY XI/FFXi.dll` | `9053d4106160` | 93251 |

## The channel is in-process COM

`polcore.dll` is an in-process COM server. Its only exports are the four ATL
entry points (`DllGetClassObject`, `DllRegisterServer`, `DllCanUnloadNow`,
`DllUnregisterServer`). The class is `CPOLCoreCom` implementing `IPOLCoreCom`
(RTTI `.?AVCPOLCoreCom@@`, `.?AUIPOLCoreCom@@`, ProgID `POLCore.POLCoreCom.1`).
Registration is `InprocServer32` with `ThreadingModel=Apartment` for all three
region CLSIDs: `{07974581-0DF6-4EF0-BD05-604B3ADA9BE9}` worldwide,
`{E5966FB3-...}` US, `{3501F5DD-...}` EU. There is no `LocalServer32` and no
marshaling AppID, so the authenticated object exists only inside the process
that ran the handshake. A `CoCreateInstance` from another process returns a
fresh, unauthenticated object.

`polcore` carries the classic function-table export string
`GetCommonFunctionTableWW` at VA `0x10407340` and `0x10407960` (packed data;
"WW" is the worldwide build). This is the table of polcore function pointers
the game calls back through.

`FFXiMain.dll` is the COM client and is itself a COM server ("GameMain Class",
CLSID `{1027DC46-750D-4B1F-8834-1D25B8BEBAB8}`). It imports `ole32`
(`CoCreateInstance`, `CLSIDFromString`, `CoInitialize`, `CoSetProxyBlanket`)
but does not embed polcore's CLSID. Inference, not observation: `pol.exe`
constructs both objects and passes the live interface or function table into
the game at init, and the game fetches the session through it.

`polhook.dll`'s 16-byte writable `SHARED` section (`.shared_`, VA
`0x1000C000`, virtual size `0x10`) is `SetWindowsHookExA` window-hook state
(`CPolHook`, `MakeHook`, `UnHook`, USER32 hook imports). It carries no auth
data, which settles the earlier open question about that section.

## What the game fetches, and where it lands in the 0x26

`research/XIClient` stubs the same call sequence with the fetches marked.
`LoginStateMachine::HandleLogin` holds a `char[0x40]` it feeds to
`ntTcpDLLSetAthCode`, annotated "get authcode", and a 16-byte buffer for
`ntTcpDLLSetPasswd`, annotated "get random value from pol". Earlier states
(`HandleStart`, `HandleCreateClient`) are annotated "something from pol": the
polcore state machine must succeed first. Those setters populate the lobby
session context, and `ntTcpDLLRequestLobbyLogin` builds C2S `0x26`.

The header XIClient declares is `{u32 length; u32 tag; u32 opcode; u8
hash[16]}`. **The 16 bytes at offset `0x0C` are a per-packet MD5
authenticator** (`ntLoginHashPacket`), not an account identity. LSB reuses that
field to carry its own session hash, which is why Kuluu's LSB path fills it
with the auth server's `session_hash`; retail computes it per packet. The
64-byte field at `0x34` is the authCode polcore builds. Version string at
`0x74` and client expansions at `0x84` match what Kuluu already sends.

## Can Kuluu obtain the session without touching SE processes?

No out-of-process handoff exists to read. There is no named file mapping, no
file under `usr/` or `polcfg/`, no registry value, no command line and no
`WM_COPYDATA` that carries the authCode. Three postures follow:

1. **Host the genuine, unmodified `polcore.dll` in a Kuluu-launched process
   and drive it over COM.** Injection-free and patch-free: Kuluu takes over
   `pol.exe`'s role using Square Enix's own binary as a library, then reads the
   authCode from the instance it authenticated. Cost: Kuluu must reimplement
   the login flow that drives polcore, which is the profile and chat handshake
   mapped in the 2026-09-16 record. Windows or Wine with COM only.
2. **`ReadProcessMemory` against an already-running `pol.exe`.** No code
   injected and no byte patched, but a foreign-process memory read is a
   different posture from (1), and it is pinned to one build's offsets.
3. **`CoCreateInstance` in Kuluu's own process.** Useless. The inproc-only
   server returns an unauthenticated object with no session.

xiloader is not a model for any of these. It hosts `FFXiMain` itself and either
mints its own authCode for private servers or hooks polcore's function table.
It never obtains a real Square Enix session.

## Unverified

- The `pol.exe`-constructs-both-objects claim is inferred from the registration
  shape and from FFXiMain not embedding polcore's CLSID, not from a call site.
  `polcore.dll` is POL1-LZSS-packed on disk, so its code was not disassembled.
- Which function-table slot returns the authCode and which returns the 16-byte
  random value. Unpacking polcore and disassembling the table builder would
  settle it.
- The account id the data server reads from `0xA1` at offset 1. XIClient's
  `HandleTag0xA1` is unimplemented and the value is not in these binaries'
  strings.
- Retail's byte-exact `0x26` field map rests on XIClient field order and
  XiPackets prose, not on disassembling FFXiMain's builder. XIClient targets a
  modified server and writes only 16 bytes at `0x34`, so it is not retail truth
  for that field.
