# PlayOnline in-house login: the whole account handshake (2026-09-19)

Static inspection of the PlayOnline Viewer and FFXi binaries of a legally
installed retail client. No Square Enix binary was executed, patched, or
contacted; no production host was resolved. This record is the durable map;
the byte-exact detail lives in the `ffxi-pol` crate, where each routine cites
the polcore build it was read from, and in the git-ignored working notes under
`artifacts/polre/inhouse/`.

Builds: polcore.dll `73b1864b` (viewer 1.18.15e, the decompiled build),
cross-checked against `f5af5837` (1.18.00n, retail tree); app.dll `7ba99828`;
pol.exe `5c2d45bd`; FFXiMain.dll `f2245d1c`; FFXi.dll `9053d410`. All were
POL1-LZSS-packed on disk and unpacked for reading.

## Why this exists

Retail FFXI has no auth server of its own. The PlayOnline Viewer signs the
account in over two services and hands the game a 16-byte value and a 64-byte
authCode that the lobby validates. Kuluu once read a session a producer wrote
to a file beside the Viewer; that handoff is gone. Kuluu performs the account
handshake itself, the same way the Viewer does: the PlayOnline auth flavor of
`kuluu_session::auth_client` takes the member name and password and runs
`kuluu_session::pol_inhouse`. It adds reach, not power: the player
authenticates their own account against Square Enix, and someone without a
paid account gains nothing.

## The two services

- **Chat service** (IRC dialect, TCP 51240/51241/51242). Establishes the
  session Blowfish key by an ephemeral-RSA exchange. Ported in
  `ffxi-pol::{rsa, chat, crypto}`.
- **Profile service** (fixed-frame, TCP 51220 on `pp%03d.pol.com`). Carries the
  member login and the transactions that yield the lobby session. Its frames
  are enciphered with the Blowfish key the chat service agreed. Ported in
  `ffxi-pol::{profile, authcode}`.

## The ordered handshake

1. **Chat key agreement** (`ffxi-pol::transport::agree_session_key`): connect,
   read the greeting (which carries a challenge and a clock-sync word), send
   `USER` with the client's 256-bit RSA modulus in the realname, read numeric
   300 and RSA-decrypt the session Blowfish key from it, then `NICK` and read to
   numeric 422 (which the client treats as login complete). The stream cipher is
   on from numeric 300; each IRC line reciphers from an IV seeded by the
   client's own modulus.
2. **Profile member login** (category 4, opcode 7): a 0x40-byte body of the
   mode, the member name, and a SHA-1 over the hex of a 20-byte secret and the
   minute-rounded unix time. Proves possession; registers nothing.
3. **Content-id list** (category 2, opcode 3): the account's service list, from
   which a content id is chosen.
4. **World / service select** (category 4, opcode 6).
5. **Enter community** (category 4, opcode 5): its 0x20-byte reply is the only
   server-issued entropy in the whole session.

## The lobby session is assembled client-side

The community reply becomes the lobby values by a transform that needs no
further server participation (`ffxi-pol::authcode`):

- the **16-byte value** is the reply's first sixteen bytes;
- the **64-byte authCode** is a scratch of `MD5(reply[0:16] || clock ||
  chat_key)`, the 20-byte POL address struct, and two derived head bytes,
  enciphered with polcore's `*5`-chain cipher (an 8-byte key baked into the
  DLL), then run through a forward XOR chain and four head fixups. polcore fills
  52 of the 64 bytes; the retail client leaves the rest uninitialised.

So the session cannot be minted offline (the op5 reply is server-issued), but
once that reply is in hand the assembly is entirely client-side over state the
client already holds. This is the decisive fact: a faithful client that
completes the real handshake can produce the lobby session without any Viewer
process and without reading another process's memory.

## The hosts

Every host name is a literal in the binaries; nothing is configured by the
player in the Viewer, and Kuluu's PlayOnline profile needs only the lobby.

| Service | Host | Where it is read from |
|---|---|---|
| chat (IRC dialect) | `pc%03d%s.pol.com` | app.dll `7ba99828`; the index and suffix are filled by the Viewer's application layer, not yet traced. polcore itself carries only `gm000`/`gd000.pol.com` (GM chat) and stores the member chat host through a setter. |
| profile | `pp%03d.pol.com`, port 51220 | polcore `0x10075430`; the index is bits 9..15 of the member identity's high dword. |
| FFXI lobby | `ffxi00.pol.com`, ports 54230/54001 | FFXiMain.dll of the retail-2026-09 row (sha256 f2245d1c9d06e02c): the string at VA 0x10362044 is resolved from VA 0x100ed84d when the connection-mode global at VA 0x104ca430 is 0. Modes 1 and 2 are development paths (`ci000.pol.com` sits beside a `172.16.x` address and `c:\image\ffxi\serv`). |

So a PlayOnline profile's Host is the FFXI lobby server, and `ffxi00.pol.com`
is the game's own default for it (`ffxi_pol::hosts::LOBBY_HOST`); the auth
port is unused because the account services replace the auth server. Whether
the world-select reply (category 4, opcode 6, 0x80 bytes) carries a lobby
address that would override this default was not decoded.

## The Viewer-to-game seam (for the COM posture)

FFXiMain reads the finished session through a polcore function table, not a file
or a shared section. `IPOLCoreCom` slot 7 `GetCommonFunctionTableWW` is a
getter returning that table; FFXi.dll hands it to FFXiMain, which calls
table+0xEA0 for the authCode bytes and table+0xFAC for the 16-byte value. This
is why hosting the genuine unmodified polcore over COM is a viable
injection-free posture as well.

## What is not yet pinned

- The derivation of the 20-byte member-login secret from the typed password;
  its writer is in app.dll and was not traced (inferred to be `SHA1(password)`).
- The origin of the chat `NICK` credential for a first login (it is installed
  after a successful login, a lifecycle chicken-and-egg), and the `NICK`
  trailing token from an un-decompiled routine.
- Live wire compatibility. Every test vector in `ffxi-pol` is self-derived from
  the static reading and pins the reading, not interoperability. Verifying
  against Square Enix contacts their servers with a real account and is the
  player's own action, out of scope for automated runs.
