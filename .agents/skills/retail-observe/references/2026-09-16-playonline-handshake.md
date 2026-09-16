# PlayOnline handshake: the profile and chat services (2026-09-16)

Static inspection of the PlayOnline Viewer modules shipped in the local
installs. No live traffic was sent to Square Enix; probing their production
hosts is out of scope for this repository. Everything below is read from
binaries under `vendor/game-files/targets/`.

Builds inspected:

| Module | SHA-256 prefix | Viewer |
|---|---|---|
| `PlayOnlineViewer/viewer/com/polcore.dll` | `73b1864bf522` | 1.18.15e (hxi tree) |
| `PlayOnlineViewer/viewer/com/polcore.dll` | `f5af5837ffef` | 1.18.00n (retail tree) |
| `PlayOnlineViewer/viewer/com/app.dll` | `7ba99828` | 1.18.15e |
| `PlayOnlineViewer/polhook.dll` | `e4ba164a0e3a` | 1.18.15e |

All addresses are VAs in the **unpacked** `polcore.dll` `73b1864bf522` at image
base `0x10000000`. Unpack with `xi dll polcore unpack` or the POL1 LZSS reader
described in `.agents/skills/retail-grounding/references/binary-inspection.md`.
The two viewer builds share this code; `f5af5837ffef` has the same routines at
shifted addresses.

## Two services, not one

`polcore` speaks two unrelated protocols, and they are easy to conflate because
they share one Blowfish implementation. Keep them apart:

| | Profile service | Chat service |
|---|---|---|
| Port | 51220 | 51240, 51241, 51242 |
| Framing | fixed 0x28-byte binary request | CRLF-terminated text lines |
| Protocol | POL's own category/opcode transactions | IRC (RFC 1459) plus POL extensions |
| Context | `0x10404ad0`, `0x338` bytes per slot | `0x103e58a0`, `0x3a00` bytes per slot, three slots |
| Blowfish context | slot `+0x50` | slot `+0x2c0` |

The Blowfish key is negotiated **only** on the chat service. The profile
service never performs a key exchange; it copies the chat service's finished
key and schedule out of a process global. So a profile transaction cannot be
read or written without first completing the chat login.

One byte-order note, because the stores look wrong at a glance. The address
struct these services fill is POL's own, not a `sockaddr_in`: it holds the port
and the address in **host** order, and the socket wrapper byteswaps both on the
way out (the port at `0x100104e9`, the address immediately above it). A literal
like `0xC814` stored with no `htons` in sight is therefore 51220 as written,
not its byteswap.

## The profile server

`polcore` builds its host from `pp%03d.pol.com` (`0x1001e90f`, `0x1001f160`)
and connects on port `0xC814` = **51220**. That is the same port LandSandBoat
reserves as `LOGIN_CONF_PORT` in `vendor/server/settings/default/network.lua`,
which is a useful corroboration: LSB mirrors the retail port map, it just never
implements this listener. Kuluu's lobby work has always been on 54001, a
different service.

The first reply carries a redirect. `0x1001f690` reads a big-endian dword at
reply offset `0x08` and, when a cached address is not already set, stores
`{family, port 0xC814, that address}` into the global at `0x10404ab8`, which
later connections reuse instead of resolving the name again.

## Framing

A transaction is a fixed **0x28-byte request** and a **0x18-byte reply header**,
optionally followed by a body whose length the request declares.

The request builder is `0x1001f5e0(ctx, category, opcode, length)`:

| Offset | Content |
|---|---|
| `0x00` | `2`, constant |
| `0x01` | category |
| `0x02` | opcode |
| `0x03` | `0` |
| `0x04` | declared body length |
| `0x08` | sixteen zero bytes |
| `0x18` | MD5 digest, sixteen bytes |

The digest is the per-request authenticator. It covers three inputs in order:
an eight-byte identity derived from the globals at `0x10404a88`, a fifteen-byte
secret at `0x10404a94`, and a four-byte rolling token at `ctx+0xB8` that the
previous reply supplied. This is the mechanism XiPackets describes as the
session `passwd` that is "MD5 hashed using the current internal md5key state",
seen from the PlayOnline side rather than the lobby side.

Both the identity and the secret are kept obfuscated in memory rather than in
the clear. `0x1001a550` and `0x1001a5e0` store the secret XORed against a
rolling eight-byte pad at `0x100aa848`, and the identity is XORed with the same
pad whenever it is read (`0x1001a050`). `0x1001ea60` and `0x1001eab0` are the
installers that take a fresh identity and secret after a successful login.

Reply header byte `0x01` is the status: zero means success, anything else is
negated into an error code and mapped by `0x1001f400`.

## The chat service and the key agreement

The line-oriented service is an IRC client. The command table at `0x10073f48`
is the RFC 1459 set in order, `PASS` `NICK` `USER` `SERVER` `OPER` `QUIT` ...
`ISON`, with POL's own commands appended past the standard forty. Inbound lines
are split on `\n` by `0x100164e0`, dispatched by `0x10015e80`, and numerics are
routed by value; outbound lines are formatted through the table at
`0x10073b4c` and queued by `0x10013730`. The strings `(POL `, `IRC%02x` and
`Kicked by same NICK` belong to this service.

The exchange that produces the Blowfish key runs in this order:

1. **Ephemeral RSA keypair, per slot.** `0x10013610` calls `0x1002bb40`, which
   draws two random 128-bit primes, forms a 256-bit modulus, fixes the public
   exponent at `0xFFFF`, and derives the private exponent as its inverse mod
   `(p-1)(q-1)`. The key struct pointer lives at slot `+0x39c8`.
2. **`USER` goes out** (`0x10016cf0`), carrying the slot's `+0x220` buffer as
   the realname field.
3. **The server answers with numeric 300.** Its payload is 0x2e characters in a
   custom base64 alphabet, decoded to 34 bytes by `0x100078a0`.
4. **The client RSA-decrypts that block** with its own private exponent
   (`0x1002bde0`, modexp in `0x1002bfb0`).
5. **The first eight plaintext bytes, byte-reversed, are the Blowfish key.**
   They are stored at slot `+0x2b8`, the schedule is built by `0x10063eb0`, and
   `+0x209` is or'd with `0x44` to switch the stream cipher on in both
   directions. Nothing before this point is encrypted.
6. **Only now does `NICK` go out** (`0x10016ae0`), as
   `NICK <identity>:<md5-hex>`. The digest covers a server challenge the
   greeting supplied (slot `+0x39f0`) followed by a sixteen-byte secret (slot
   `+0x39f4`). Both are held XOR-obfuscated under pads whose pointers are
   themselves stored XORed against the buffer pointers.
7. **The key is published process-wide.** `0x1001a170` copies the eight-byte
   key to `0x100aa858` and the 0x68-byte schedule to `0x100aa860`;
   `0x1001a1a0` copies them into a profile-service context. That is the whole
   of the profile service's key agreement.

### The cipher is a Blowfish variant

`0x10064220` is a textbook Blowfish block function: sixteen rounds, four
256-entry S-boxes at `+0x000`, `+0x400`, `+0x800`, `+0xC00`. The **key
schedule is not textbook**, and a stock Blowfish implementation will not
interoperate. `0x10064300` derives the initial P-array and every S-box word
from the key instead of seeding them with the digits of pi:

- MD5-expand the eight-byte key to 4096 bytes, repeatedly hashing the buffer so
  far and appending the sixteen-byte digest.
- Take the eighteen P-array entries from **overlapping** byte windows at the
  head of that buffer, advancing one byte per entry rather than four.
- Fill all 1024 S-box words from the same buffer, four bytes per word.
- XOR the P-array with the key repeated cyclically.
- Run the standard self-encryption pass over P and then the S-boxes.

The hash is MD5 throughout (`0x10018830` init, `0x10018860` update,
`0x10018910` final).

Two details matter for anyone reading a capture. The stream layer
(`0x10063ef0`) keeps separate state per direction, at `ctx+0x50` and
`ctx+0x58`, with a byte counter at `ctx+0x60` that pulls a fresh keystream
block every eight bytes. And bytes that are `\n` or `\r` before or after XOR
are passed through in the clear, so a capture looks like ciphertext with intact
line breaks. A packet checksum is computed separately by `0x10063df0`, a dword
sum with a tail fold, not a hash.

## Profile-service application layer

Fifteen distinct profile transactions call the same connect/send/receive helpers. Their
`(category, opcode, length)` triples read:

| Category | Opcodes seen | Body length |
|---|---|---|
| 2 | 6 | computed per record count |
| 3 | 0, 1, 2, 3, 4 | `0x198`-`0x39C` |
| 4 | 3 | `0x70` |
| 7 | 1, 2, 3, 0x0B | `0x10`-`0x78` |

Each transaction is a small state machine over the shared steps: connect
(`0x1001f0f0`), send request (`0x1001f4d0`), read reply header (`0x1001f690`),
stream a body in or out (`0x1001f800`, `0x1001f970`), tear down
(`0x1001fbd0`). The viewer UI in `app.dll` drives them from its `pol::CLogin`
and `pol::CLoginMemberPasswordFrame` screens.

## What is still unknown

Both transports are understood, and so is the key agreement. What remains is
the credential itself:

- Where the sixteen-byte secret at chat slot `+0x39f4` comes from.
  `0x10045e10` produces it at construction time, and whether it is the POL
  password, a hash of it, or a stored token is not yet established. This is now
  the only unsolved step in the chat login.
- Which of the fifteen profile transactions carries the account name, and what
  the server returns that becomes the profile identity and secret.
- How the sixty-four byte `authCode` that the FFXI lobby reads at C2S `0x26`
  offset `0x34` is derived. XiPackets says polcore builds it from the client
  IP, a Blowfish key, the login time and random data, then encrypts and XORs it
  against itself.
- How that value reaches `FFXiMain.dll`. `polhook.dll` carries a sixteen-byte
  writable **shared** PE section at VA `0x1000C000`, read and written by its own
  code at `0x10001060`-`0x10001117`. Sixteen bytes is too small to be the
  `authCode` itself, so it is more likely a handle or ready flag, with the real
  payload passed another way.

## Two ways to use this

**Reimplement the PlayOnline login inside Kuluu.** This needs the four unknowns
above, and it means reimplementing Square Enix's account authentication rather
than the game protocol. That is a different posture from everything else in
this repository: the wire protocol is mirrored by an open-source server we can
read, and account auth is not. Windower and Ashita both decline to do this.

**Let the real `pol.exe` authenticate and take over at the lobby.** This is the
xiloader model and the one that matches "adds reach, not power". The player
runs the PlayOnline Viewer they already own, it performs the handshake above
against Square Enix exactly as it does today, and Kuluu joins at the lobby with
the `authCode` that the viewer produced. Nothing about Square Enix's
authentication is reimplemented or weakened, and a player without a paid
account gains nothing. The open question is the handoff: how to read the
session out of the running viewer, which is what the `polhook.dll` shared
section and the `GetCommonFunctionTable` export are for.

The second path is the one to scope first. It is smaller, it does not require
solving SE's auth, and if it fails it fails for a reason worth recording.
