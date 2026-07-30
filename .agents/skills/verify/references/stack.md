# LSB dev stack: bring-up and environment gotchas

The stack is five docker containers under colima:

| Container | Role | Host port |
|---|---|---|
| `server-connect-1` | auth (TCP/TLS) + lobby | 54231 / 54001 |
| `server-world-1` | world / char-list | internal |
| `server-search-1` | auction search | internal |
| `server-map-1` | map (UDP + Blowfish) | 54230/udp |
| `server-database-1` | MariaDB (`xidb`) | 3306 |

## Bring-up

```bash
colima status || colima start
docker ps -a --format '{{.Names}} {{.Status}}' | grep server-
docker start server-map-1 server-world-1        # often Exited (255) after VM sleep
docker logs server-map-1 -f                     # wait for "The map-server is ready to work"
nc -z 127.0.0.1 54231                           # auth listener up
```

DB access for fixtures/inspection:

```bash
docker exec server-database-1 mariadb -uxiadmin -ppassword xidb -e "..."
```

## Failure modes (all observed, all documented fixes)

- **colima dead / one-way UDP / login stuck at "Authenticating"** — the
  external Sidecar drive slept and virtiofs went stale. Fix: `colima restart`,
  then `docker start server-map-1 server-world-1`. Symptom variant: map UDP
  exchanges the login burst then goes silent both ways.
- **Map UDP must stay published `127.0.0.1:54230/udp`** — publishing on
  0.0.0.0 hits a lima forwarding race. Check with `docker port server-map-1`.
- **Ghost sessions after killing a client** — the next lobby login times out
  (`lobby lpkt_next_login (view): server did not respond within 20s`). The map
  server holds the char for 2–5 min. Prefer a clean `disconnect` (MCP tool /
  client exit) over `kill` to avoid this entirely; back-to-back relaunches
  otherwise land in a lockout loop costing ~2 min per retry.

  Either wait for the server's own `cleanupSessions` log line, or clear the one
  stale row for the character you are driving:

  ```bash
  docker exec server-database-1 mariadb -uxiadmin -ppassword xidb \
    -e "DELETE FROM accounts_sessions WHERE charid=<charid>;"
  ```

  Get `<charid>` from `SELECT charid FROM chars WHERE charname='<name>';`
  (Verilamp is 17455719 on this machine's dev DB). Keep the `WHERE` — other
  sessions on this stack are not yours to drop, and a whole-table delete is a
  destructive action that needs the user's say-so.
- **Zone changes silently ignored for a manually provisioned fresh char** —
  see SKILL.md §Character strategy. Not an env problem; don't restart the
  stack over it.

## Server-side introspection

- `docker logs server-map-1 --since 5m` — LoadChar / IncreaseZoneCounter /
  cleanupSessions / GM traces at debug level; packet-validator failures log as
  `Invalid <name> packet from <char>`.
- Authoritative handler behavior lives in `vendor/server/src/map/packets/c2s/`
  — when the server does something surprising, read the handler before
  theorizing.
