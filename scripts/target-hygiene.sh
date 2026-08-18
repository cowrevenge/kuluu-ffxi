#!/usr/bin/env bash
# Report and (opt-in) prune build-cache growth under target/.
#
# WHY: cargo never removes superseded artifacts. Every distinct feature
# unification and profile mints a new -C metadata hash, so a workspace driven
# from several entry points (agent sessions, rust-analyzer, checks.sh, a
# per-package `cargo test -p`) accumulates parallel artifact universes. Measured
# here: target/ reached 241 GB, with 18-19 stale copies of every example binary
# and single examples at 213 MB. `cargo clean` is all-or-nothing and throws away
# the warm cache too, which is what turns a full day of fast 4-12s incremental
# builds into a 304s+ cold rebuild. See bead kuluu-p5a5.
#
# `cargo -Zgc` tracks and collects the GLOBAL cache (~/.cargo), not target/, so
# it does not cover this.
#
# Usage:
#   scripts/target-hygiene.sh                 # report only (default)
#   scripts/target-hygiene.sh --prune         # delete artifacts not touched recently
#   scripts/target-hygiene.sh --prune --days 3
#
# Deleting from target/ is always safe -- it is a cache and cargo rebuilds what
# it needs. Pruning trades a slower next build for reclaimed disk.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

readonly DEFAULT_PRUNE_DAYS=7
# Below this, a report is noise; the tree is healthy.
readonly REPORT_THRESHOLD_GB=40

prune=0
days=$DEFAULT_PRUNE_DAYS
while [ $# -gt 0 ]; do
  case "$1" in
    --prune) prune=1 ;;
    --days) shift; days="${1:?--days needs a value}" ;;
    -h|--help) sed -n '2,26p' "$0"; exit 0 ;;
    *) echo "target-hygiene: unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

[ -d target ] || { echo "target-hygiene: no target/ directory"; exit 0; }

echo "== size =="
du -sh target 2>/dev/null || true
du -sh target/* 2>/dev/null | sort -rh | head -6 || true

echo
echo "== stale duplicate artifacts (same target, different -C metadata hash) =="
for d in target/debug/examples target/debug/deps; do
  [ -d "$d" ] || continue
  printf '%s:\n' "$d"
  ls "$d" 2>/dev/null \
    | sed 's/\.\(d\|rlib\|rmeta\|dylib\|o\)$//' \
    | sed 's/-[0-9a-f]\{16\}$//' \
    | sort | uniq -c | sort -rn | awk '$1 > 3 {printf "  %3d copies  %s\n", $1, $2}' | head -8
done

echo
echo "== Spotlight =="
if [ -f target/.metadata_never_index ]; then
  echo "  .metadata_never_index present"
else
  touch target/.metadata_never_index
  echo "  .metadata_never_index was MISSING -- created"
fi
if command -v mdfind >/dev/null 2>&1; then
  indexed=$(mdfind -onlyin "$PWD/target" "kMDItemFSName == '*.rlib'" 2>/dev/null | wc -l | tr -d ' ')
  echo "  indexed .rlib entries under target/: $indexed"
  if [ "${indexed:-0}" -gt 0 ]; then
    echo "  NOTE: the marker only stops FUTURE indexing. Stale entries persist until the"
    echo "        volume is reindexed. To clear them, add target/ to System Settings ->"
    echo "        Spotlight -> Search Privacy (cannot be scripted without full disk access)."
  fi
fi

if [ "$prune" -eq 0 ]; then
  size_gb=$(du -sg target 2>/dev/null | cut -f1)
  echo
  if [ "${size_gb:-0}" -ge "$REPORT_THRESHOLD_GB" ]; then
    echo "target/ is ${size_gb}GB (>= ${REPORT_THRESHOLD_GB}GB). Re-run with --prune to reclaim."
  else
    echo "target/ is ${size_gb}GB -- healthy, nothing to do."
  fi
  exit 0
fi

echo
echo "== pruning artifacts not accessed in $days days =="
before=$(du -sg target | cut -f1)
# Only artifact files, never fingerprints/incremental state -- removing those
# forces a full rebuild rather than a partial one.
find target -type f \
  \( -name '*.rlib' -o -name '*.rmeta' -o -name '*.dylib' -o -name '*.o' \) \
  -atime "+$days" -delete 2>/dev/null || true
find target/debug/examples target/release/examples -type f -atime "+$days" -delete 2>/dev/null || true
find target -type d -empty -delete 2>/dev/null || true
after=$(du -sg target | cut -f1)
echo "  ${before}GB -> ${after}GB"
