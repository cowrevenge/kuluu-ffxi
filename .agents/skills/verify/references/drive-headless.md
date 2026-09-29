# Headless drive — see `kuluu_headless/` (canonical)

The full recipes live in [`kuluu_headless/README.md`](../../../kuluu_headless/README.md):
Surface A animationtest box, Surface B MCP / raw stdio / integration tests, accounts,
evidence, gotchas — Windows and other-OS variants inline. This file is kept only as the
verify skill's entry point; do not duplicate recipes here.

Quick surface pick:

| Verifying | Surface | Where |
|---|---|---|
| Particles, routines, zone/weather load, actor placement, **any render-layer question** | A — AnimationTest box (hidden window) | `kuluu_headless/` §1 |
| Wire protocol, session state, reactor goals, zoning, chat, entity/spawn flow | B — headless MCP (preferred) or raw stdio | `kuluu_headless/` §2–§3 |
| Session/zone/MCP transport layers as a whole | B — live integration tests (self-skip without a server) | `kuluu_headless/` §4 |

Key facts that cost hours when forgotten:

- **`play --headless` is session-only** — no Bevy, no MZB geometry, no particles. Render
  questions go to Surface A (`kuluu_headless/` §3).
- Raw stdio launch = three separate commands (Git Bash precedence race), prebuilt repo-root
  exe, `--mute`, ports from `docker port cow-connect` (§3).
- Pixels: in-app `shot` case first (works on the zero-size hidden window);
  `scripts/cap-window.ps1` only when the window has a client area (§1).
- Ending a driven session cleanly: `/shutdown` chat from a GM account, ~35s wait — not
  `taskkill` (§7).
