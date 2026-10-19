# Local LSB stack: startup, recovery, and provisioning

Start or recover an applicable local stack before reporting Docker as a verification blocker.
Use the task's existing authority for local development setup.
A stopped service or a missing container is not, by itself, a final blocker.
Preserve existing profiles, database contents, and unrelated workloads.

## Configured stack

`scripts/lsb-stack.sh` controls the default Colima profile and these configured containers.
It starts existing containers; it does not install Docker or create a new server.
Use it only when its runtime and container assumptions match the selected stack.
Before `up`, confirm that no unrelated workload needs the default VM.
The helper starts an idle reaper that can stop the entire VM.
If ownership is unknown, use explicit Docker commands or an isolated project.

| Container | Role | Host ports |
|---|---|---|
| `server-connect-1` | Authentication and lobby | TCP 54231, 54001, 54230 |
| `server-world-1` | World and character list | Internal |
| `server-search-1` | Search | Internal |
| `server-map-1` | Map | UDP 54230 |
| `server-database-1` | MariaDB (`xidb`) | TCP 3306 |

```bash
scripts/lsb-stack.sh status
scripts/lsb-stack.sh up
```

If startup fails, continue with the applicable procedure below.
Do not repeat the same startup command without a new diagnostic or a changed condition.

## Check the Docker endpoint

A Docker context selects the engine endpoint that the Docker command uses.
Check the selected endpoint before changing a runtime.
Inspect `DOCKER_HOST` and `DOCKER_CONTEXT` overrides without printing authentication variables or certificate contents.

```bash
command -v docker
docker context show
docker context ls
docker context inspect --format '{{.Name}} {{.Endpoints.docker.Host}}'
docker info --format '{{.ServerVersion}}'
```

If the sandbox denies socket access, use the permitted tool-escalation procedure and repeat the read-only check.
Do not diagnose a stopped Docker daemon from a sandbox denial alone.
If the selected endpoint is wrong, use an applicable local context for individual commands.
Keep the user's global context selection unchanged.

For the configured default Colima stack, check its context directly:

```bash
env -u DOCKER_HOST -u DOCKER_CONTEXT docker --context colima info --format '{{.ServerVersion}}'
colima list
colima --profile default status
```

`colima start` can report "already running" while the selected Docker endpoint remains unavailable.
A successful `docker info` response establishes engine access; a VM status alone does not.

## Recover the installed runtime

Select the installed provider that owns the required local engine.
If runtime tools are missing, install the host-appropriate runtime through its maintained installation instructions and permitted package tools.
Use [Docker installation](https://docs.docker.com/engine/install/) or [Colima installation](https://github.com/abiosoft/colima#installation) as applicable.
Keep this work local to the development task.

- For stopped Colima, start its existing profile with `colima --profile default start --activate=false`.
- For installed Docker Desktop, use `docker desktop start` when its CLI supports that command.
- For a Linux system Docker service, use `sudo systemctl start docker` when the host uses systemd.
- For rootless Docker, use its installed user service and socket instead of the system service.

After startup, repeat the explicit-context `docker info` check.
If host access still fails, compare host and guest access before selecting a repair:

```bash
colima --profile default ssh -- sudo docker info --format '{{.ServerVersion}}'
```

If the guest engine works, inspect the host socket, context, and forwarding diagnostics.
If the guest engine is stopped, inspect its installed service manager and Docker service logs.
Start the installed Docker service when the diagnostic supports that action.
Do not assume every Colima guest uses systemd.

If a stale VM prevents recovery, check workload ownership before restarting it.
Restart a dedicated verification profile when no other session uses it.
If ownership is unknown or a restart would interrupt other work, use an isolated profile below.
For Colima's `Broken` state, a forced stop is permitted only for a session-owned profile with no retained workload.
Use the [Colima recovery reference](https://github.com/abiosoft/colima/blob/main/docs/FAQ.md#broken-status) for that specific condition.
Do not use profile deletion, volume deletion, or `docker system prune` as routine recovery.

After an identified repair, retry startup once and check the engine again.
If that repair fails, use isolation where possible or retain the specific remaining failure.

## Provision an isolated stack

Use this procedure when the configured stack is absent or shared-runtime recovery is unsuitable.
Use a healthy local engine when one is available.
If necessary, create a session-owned Colima profile with an unused name and host-appropriate resource limits.
Record the profile name and its Docker context in the bead.

For example, this profile reserves four CPU cores and eight GiB of memory:

```bash
lsb_profile="kuluu-verify-$(date +%s)"
colima start --profile "$lsb_profile" --runtime docker --cpu 4 --memory 8 --activate=false
docker context ls
```

Check available host resources before selecting those limits.
Identify the new profile's context from the context list; do not assume it became the global default.
Use explicit `--context` arguments for its Docker commands.
Do not point `scripts/lsb-stack.sh` at another profile through Docker environment overrides.
Its Colima lifecycle commands still control the default profile.

Read [the maintained LSB Docker instructions](../../../../vendor/server/docker/README.md) before creating the server project.
The [upstream copy](https://github.com/LandSandBoat/server/blob/base/docker/README.md) is available when the vendor checkout is absent.
Initialize the pinned `vendor/server` submodule when a source build or revision comparison requires it.
Do not edit vendor files.

1. Select a server revision that supports the behavior under test.
   Record the image digest or source revision.
   Current JSON-only LSB cannot establish successful legacy binary authentication.
2. Create a session-owned Compose file and project name outside the tracked source tree.
   Adapt the maintained server example, including MariaDB, database initialization, and the server services.
   `dev.docker-compose.yml` primarily supports upstream CI; it is not a replacement for the full server example.
3. Use separate database and navmesh volumes for the new project.
   Initialize and migrate only that new database.
   Do not attach an existing database to a different server revision.
4. Use local test credentials and bind published ports to loopback.
   Check port availability before startup.
   Configure both client endpoint ports and server-advertised addresses if alternate ports are necessary.
   Map traffic requires a UDP publication; a TCP port with the same number is insufficient.
5. Initialize navmeshes through the documented image or source workflow.
   Start the project with explicit context, project, and Compose-file arguments.
   Verify successful database initialization before testing authentication.

Set `lsb_context`, `lsb_project`, and `lsb_compose` to the recorded values before these commands:

```bash
docker --context "$lsb_context" compose --project-name "$lsb_project" --file "$lsb_compose" config --quiet
docker --context "$lsb_context" compose --project-name "$lsb_project" --file "$lsb_compose" up --detach
docker --context "$lsb_context" compose --project-name "$lsb_project" --file "$lsb_compose" ps --all
```

Keep credentials out of published logs and repository files.
Use the [character provisioning guidance](../SKILL.md#character-strategy) for a local test account and character.

## Readiness and stopping conditions

Check database health, initialization results, server logs, and published TCP and UDP ports.
For the configured stack, inspect these surfaces:

```bash
docker logs server-map-1 --since 5m
docker port server-map-1
nc -z 127.0.0.1 54231
```

The map log must show `The map-server is ready to work` for a map-session check.
Authentication-only checks require the database and authentication service used by the request.
An open port or a running container does not establish successful authentication.
Resume the affected [client verification](drive-headless.md) after stack recovery or provisioning.
Capture the real client result before replacing a blocked evidence marker with a pass.

Stop when an applicable repair and permitted isolation cannot provide the required service.
A rejected permission, unsupported virtualization, or failed installation can remain a concrete blocker.
Record the context, profile, server revision, attempted repair, exact diagnostic, and affected client path.
Keep verification open and report the recorded reason.
Do not treat local TLS test-server results as successful LSB authentication.

## Cleanup and known session failures

Record which runtime, containers, and volumes existed before the check.
Stop only the resources created or exclusively owned by this verification session.
For the configured stack, use `scripts/lsb-stack.sh down` only when no other session needs its runtime.
The helper also stops the default Colima VM.
Its idle reaper and stop hook are backstops, not ownership checks.
For an isolated stack, use the recorded context and Compose project for cleanup.
Preserve diagnostics and evidence before removing session-owned resources.
Do not remove existing volumes or profiles.

- Keep the configured map UDP publication at `127.0.0.1:54230/udp`.
  Check it with `docker port server-map-1` if map traffic stops.
- Use a clean client disconnect to avoid retained map sessions.
  A killed client can leave its character active for several minutes.
  Wait for `cleanupSessions`, or remove only the stale row for the test character when authorized.
  Keep the `WHERE` clause when applying that database repair.
- For ignored zone changes on a new character, read the character guidance before restarting the stack.

Read server logs and the applicable handler under `vendor/server/src/map/packets/c2s/` before attributing failures to the client.
