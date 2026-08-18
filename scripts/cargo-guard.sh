#!/usr/bin/env bash
# Run cargo with a stall watchdog and visible queueing.
#
# WHY: cargo takes an exclusive lock on the build directory, so concurrent
# invocations (agent sessions, rust-analyzer, a pre-push hook) serialize
# silently — and when the jobserver wedges, every rustc parks at 0% CPU on its
# jobserver pipe and cargo never returns. Measured in this repo: four rustc
# blocked 16-20 minutes, including a 437-line crate, while two other cargo
# processes sat 25+ minutes on target/debug/.cargo-lock. That reads as "Rust is
# slow" when nothing is compiling at all.
#
# Isolation was measured and rejected as the alternative: a second build with
# its own build.build-dir never blocked but rebuilt everything (387s, 7.4 GB)
# against 10s for the shared-dir run. Serializing is ~40x cheaper, so this
# wrapper makes the wait visible and bounded rather than trying to avoid it.
# See bead kuluu-p5a5 for the full measurements.
#
# Usage:
#   scripts/cargo-guard.sh build --workspace --features native-window
#   CARGO_GUARD_TIMEOUT=600 scripts/cargo-guard.sh test -p ffxi-dat --lib
#
# Env knobs:
#   CARGO_GUARD_TIMEOUT   hard ceiling in seconds        (default below)
#   CARGO_GUARD_STALL     zero-CPU seconds before wedge  (default below)
#   CARGO_GUARD_QUIET     set to 1 to suppress the queue notice
#
# Exit codes: cargo's own status, or 124 on timeout, or 125 on a detected wedge
# (matching GNU timeout's 124 so callers can special-case it).

set -uo pipefail

# A cold full-workspace build measured 304s here, so the ceiling has to clear
# that with headroom; anything past it is a wedge, not a build.
readonly DEFAULT_TIMEOUT_SECS=1800
# The longest plausible single-unit gap with no CPU progress. Linking the
# ~200 MB client binary is the worst case and stays well under this.
readonly DEFAULT_STALL_SECS=180
# Liveness is polled tightly so a fast build (the common case — the measured
# incremental edit loop here is 4-12s) returns the instant cargo does; the far
# more expensive CPU-tree sample is taken only every SAMPLE_SECS.
readonly POLL_SECS=1
readonly SAMPLE_SECS=10
readonly EXIT_TIMEOUT=124
readonly EXIT_WEDGED=125

timeout_secs=${CARGO_GUARD_TIMEOUT:-$DEFAULT_TIMEOUT_SECS}
stall_secs=${CARGO_GUARD_STALL:-$DEFAULT_STALL_SECS}

if [ $# -eq 0 ]; then
  echo "cargo-guard: no cargo arguments given" >&2
  exit 2
fi

note() { [ "${CARGO_GUARD_QUIET:-0}" = "1" ] || echo "cargo-guard: $*" >&2; }

# Cumulative CPU time of a process tree, in seconds. Sampling this beats
# `ps -o %cpu` because %cpu is a decaying average that reads near zero for a
# process that is merely between bursts; cumulative time only stops advancing
# when nothing in the tree is actually running.
tree_cpu_secs() {
  local root=$1
  ps -Ao pid,ppid,time= 2>/dev/null | awk -v root="$root" '
    { pid[$1]=$2; t[$1]=$3 }
    END {
      # walk every pid up to root
      total=0
      for (p in t) {
        q=p; depth=0
        while (q != "" && q != "0" && depth < 64) {
          if (q == root) { total += hms(t[p]); break }
          q = pid[q]; depth++
        }
      }
      printf "%d", total
    }
    function hms(s,   n, a, v, i) {
      n = split(s, a, ":")
      v = 0
      for (i = 1; i <= n; i++) v = v * 60 + a[i] + 0
      return v
    }'
}

others_running() {
  pgrep -f "bin/cargo (build|test|check|clippy|run|doc)" 2>/dev/null | grep -vx "$$" || true
}

# Visible queueing: report who we are about to wait behind, and for how long.
pre_existing=$(others_running)
if [ -n "$pre_existing" ]; then
  note "another cargo invocation is already running; this run will queue behind it:"
  for p in $pre_existing; do
    info=$(ps -o etime=,args= -p "$p" 2>/dev/null | head -1 | cut -c1-100)
    [ -n "$info" ] && note "    [$p] $info"
  done
fi

cargo "$@" &
cargo_pid=$!

start=$(date +%s)
last_cpu=$(tree_cpu_secs "$cargo_pid")
last_progress=$start
last_sample=$start

while kill -0 "$cargo_pid" 2>/dev/null; do
  sleep "$POLL_SECS"
  now=$(date +%s)
  kill -0 "$cargo_pid" 2>/dev/null || break

  [ $((now - last_sample)) -ge "$SAMPLE_SECS" ] || continue
  last_sample=$now

  cpu=$(tree_cpu_secs "$cargo_pid")
  if [ "${cpu:-0}" -gt "${last_cpu:-0}" ]; then
    last_cpu=$cpu
    last_progress=$now
  fi

  if [ $((now - last_progress)) -ge "$stall_secs" ]; then
    note "WEDGE DETECTED — no CPU progress for $((now - last_progress))s."
    note "  This is the cargo jobserver stall, not a slow build. Blocked processes:"
    ps -Ao pid,%cpu,etime,args 2>/dev/null | grep "[r]ustc --crate-name" \
      | sed 's/\(--crate-name [a-z_0-9]*\).*/\1/' | while IFS= read -r line; do
        note "    $line"
      done
    note "  Holder of the build lock:"
    lsof target/debug/.cargo-build-lock 2>/dev/null | tail -n +2 \
      | awk '{print "    pid " $2 " (" $1 ")"}' | while IFS= read -r l; do note "$l"; done
    pkill -P "$cargo_pid" 2>/dev/null
    kill -9 "$cargo_pid" 2>/dev/null
    wait "$cargo_pid" 2>/dev/null
    note "  Killed. Re-run; if it recurs, no other cargo should be running concurrently."
    exit "$EXIT_WEDGED"
  fi

  if [ $((now - start)) -ge "$timeout_secs" ]; then
    note "TIMEOUT after $((now - start))s (CARGO_GUARD_TIMEOUT=$timeout_secs)."
    pkill -P "$cargo_pid" 2>/dev/null
    kill -9 "$cargo_pid" 2>/dev/null
    wait "$cargo_pid" 2>/dev/null
    exit "$EXIT_TIMEOUT"
  fi
done

wait "$cargo_pid"
exit $?
