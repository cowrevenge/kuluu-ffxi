#!/usr/bin/env bash
# PostToolUse(Bash): diff the dirty set against the pre-snapshot and
# append whatever this command touched to the session edit ledger.
#
# A stale snapshot would mis-attribute a neighbour's concurrent write, so
# snapshots older than SESSION_EDITS_SNAP_TTL seconds are discarded rather
# than trusted.
#
# Contract: never blocks, never speaks.

set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=./session-edits.lib.sh
. "$here/session-edits.lib.sh"

SNAP_TTL="${SESSION_EDITS_SNAP_TTL:-3600}"

payload=$(cat)
session_id=$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)
cwd=$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)
[ -z "$cwd" ] && cwd="$PWD"
[ -n "$session_id" ] || exit 0
git -C "$cwd" rev-parse --git-dir >/dev/null 2>&1 || exit 0

cmd=$(printf '%s' "$payload" | jq -r '.tool_input.command // empty' 2>/dev/null || true)
key=$(printf '%s' "$cmd" | shasum -a 256 | cut -c1-16)
snap="$(ledger_dir)/bashpre/${session_id}.${key}"
[ -f "$snap" ] || exit 0

snap_mtime=$(stat -f %m "$snap" 2>/dev/null || echo 0)
now=$(date +%s)
if [ "$((now - snap_mtime))" -gt "$SNAP_TTL" ]; then
  rm -f "$snap"
  exit 0
fi

# Paths dirty now that were not dirty before the command ran. Strip the
# porcelain status columns, surrounding quotes, and rename arrows.
touched=$(comm -13 \
  <(sort -u "$snap") \
  <(git -C "$cwd" status --porcelain 2>/dev/null | sort -u) \
  | sed -E 's/^.{3}//; s/^"(.*)"$/\1/; s/.* -> //' \
  | grep -v '^$' || true)
rm -f "$snap"

[ -n "$touched" ] || exit 0
while IFS= read -r p; do
  ledger_add "$session_id" "$cwd" "$p"
done <<< "$touched"
exit 0
