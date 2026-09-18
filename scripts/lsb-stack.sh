#!/usr/bin/env bash
# Lifecycle for the local LSB dev stack (five containers under colima).
#
#   up      bring colima + the containers up, wait for map-server ready,
#           and arm the idle reaper
#   down    stop the containers; --vm also stops the colima VM
#   status  one screen of state (VM, containers, reaper deadline)
#   touch   push the idle deadline out (call from anything long-running
#           that the process check below can't see)
#   reap    internal: the idle backstop, spawned detached by `up`
#
# The reaper is a sleeping process, not a poller: it blocks in a single
# `sleep` until the deadline, wakes once, and either re-arms (a client is
# still attached) or stops the stack and exits. A blocked sleep costs no
# CPU and no timer wakeups, so there is nothing to amortise by checking
# less often. launchd's WatchPaths was the other candidate; it fires on
# activity, never on the absence of it, so it cannot express "idle for N
# minutes" without a timer anyway.

set -uo pipefail

CONTAINERS=(server-database-1 server-connect-1 server-world-1 server-search-1 server-map-1)
MAP_READY_LOG='The map-server is ready to work'
READY_TIMEOUT_SECS=120
IDLE_SECS=${LSB_STACK_IDLE_SECS:-1800}
STATE_DIR="${TMPDIR:-/tmp}/kuluu-lsb-stack"
DEADLINE_FILE="$STATE_DIR/deadline"
REAPER_PID_FILE="$STATE_DIR/reaper.pid"

mkdir -p "$STATE_DIR"

say() { printf 'lsb-stack: %s\n' "$*" >&2; }

vm_running() { colima status >/dev/null 2>&1; }

# Exact-name filters only. A substring match on "server-" would reach
# containers this stack does not own.
name_filters() { printf -- '--filter\nname=^/%s$\n' "${CONTAINERS[@]}"; }

containers_running() {
  local filters
  IFS=$'\n' read -r -d '' -a filters < <(name_filters; printf '\0')
  [ -n "$(docker ps -q "${filters[@]}" 2>/dev/null)" ]
}

# A live client or MCP bridge holds the stack open regardless of the clock.
# Matches only binaries out of a cargo target dir so an editor buffer or a
# grep for the same word never reads as "in use".
in_use() {
  pgrep -f 'target/[^ ]*/(kuluu|kuluu-mcp)( |$)' >/dev/null 2>&1
}

arm() { printf '%s' "$(( $(date +%s) + IDLE_SECS ))" > "$DEADLINE_FILE"; }

reaper_alive() {
  local pid
  pid=$(cat "$REAPER_PID_FILE" 2>/dev/null) || return 1
  [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null
}

spawn_reaper() {
  reaper_alive && return 0
  nohup "$0" reap </dev/null >>"$STATE_DIR/reaper.log" 2>&1 &
  printf '%s' "$!" > "$REAPER_PID_FILE"
}

wait_for_map() {
  local deadline=$(( $(date +%s) + READY_TIMEOUT_SECS ))
  while [ "$(date +%s)" -lt "$deadline" ]; do
    docker logs server-map-1 --since 10m 2>&1 | grep -qF "$MAP_READY_LOG" && return 0
    sleep 2
  done
  return 1
}

cmd_up() {
  vm_running || { say 'starting colima'; colima start || return 1; }
  docker start "${CONTAINERS[@]}" >/dev/null || return 1
  wait_for_map || { say "map-server never logged readiness within ${READY_TIMEOUT_SECS}s; check 'docker logs server-map-1'"; return 1; }
  nc -z 127.0.0.1 54231 || { say 'auth listener (54231) is not accepting connections'; return 1; }
  arm
  spawn_reaper
  say "up; idle teardown in $(( IDLE_SECS / 60 ))m unless a client is attached"
}

cmd_down() {
  local with_vm=${1:-}
  containers_running && { say 'stopping containers'; docker stop "${CONTAINERS[@]}" >/dev/null; }
  if [ "$with_vm" = '--vm' ]; then
    vm_running && { say 'stopping colima'; colima stop; }
  fi
  rm -f "$DEADLINE_FILE"
}

cmd_status() {
  local filters
  IFS=$'\n' read -r -d '' -a filters < <(name_filters; printf '\0')
  colima list 2>/dev/null | sed -n '1,2p'
  docker ps -a "${filters[@]}" --format '{{.Names}}\t{{.Status}}' 2>/dev/null | sort
  if [ -f "$DEADLINE_FILE" ]; then
    local left=$(( $(cat "$DEADLINE_FILE") - $(date +%s) ))
    if in_use; then
      printf 'idle teardown: held open by a live client\n'
    else
      printf 'idle teardown: %sm %ss\n' $(( left / 60 )) $(( left % 60 ))
    fi
  else
    printf 'idle teardown: not armed\n'
  fi
}

cmd_reap() {
  printf '%s' "$$" > "$REAPER_PID_FILE"
  while :; do
    local deadline now
    deadline=$(cat "$DEADLINE_FILE" 2>/dev/null) || exit 0  # torn down by hand
    now=$(date +%s)
    if [ "$now" -lt "$deadline" ]; then
      sleep $(( deadline - now ))
      continue
    fi
    if in_use; then
      arm
      continue
    fi
    say "idle for $(( IDLE_SECS / 60 ))m with no client attached"
    cmd_down
    exit 0
  done
}

case "${1:-status}" in
  up)     cmd_up ;;
  down)   cmd_down "${2:-}" ;;
  status) cmd_status ;;
  touch)  arm; spawn_reaper ;;
  reap)   cmd_reap ;;
  *)      say "unknown verb '${1}'; expected up|down|status|touch"; exit 2 ;;
esac
