#!/usr/bin/env bash
# Stop sub-check (priority 20): files this session wrote that are still
# uncommitted. If any, nudge the agent to group uncontroversial changes
# into a commit.
#
# Attribution is the edit ledger (session-edits.lib.sh), NOT the dirty-vs-
# SessionStart delta: in a shared checkout a concurrent session's writes
# also land in that delta, and this check would demand commits for work
# the agent never made. The SessionStart baseline still filters out
# inherited dirt.
#
# Contract: see stop-lib.sh. Exit 0 = pass; fire = exit 10 + reason.

set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=../stop-lib.sh
. "$here/../stop-lib.sh"
# shellcheck source=../session-edits.lib.sh
. "$here/../session-edits.lib.sh"
load_payload

git -C "$CWD" rev-parse --git-dir >/dev/null 2>&1 || exit 0

snap_file="${TMPDIR:-/tmp}/claude-commit-nudge/${SESSION_ID}.porcelain"
[ -f "$snap_file" ] || exit 0  # no baseline → can't tell what's session work

current=$(git -C "$CWD" status --porcelain 2>/dev/null || true)
[ -z "$current" ] && exit 0

# comm -23 needs sorted inputs; --porcelain lines are stable.
session_lines=$(comm -23 \
  <(printf '%s\n' "$current" | sort -u) \
  <(printf '%s\n' "$(cat "$snap_file")" | sort -u) \
  | grep -v '^$' || true)
[ -z "$session_lines" ] && exit 0

# Keep only lines whose path this session actually wrote. No ledger at all
# means the recording hooks are unregistered — stay silent rather than fall
# back to the contaminated tree delta.
ledger_exists "$SESSION_ID" || exit 0
session_lines=$(comm -12 \
  <(printf '%s\n' "$session_lines" \
      | sed -E 's/^.{3}//; s/^"(.*)"$/\1/; s/.* -> //' | sort -u) \
  <(ledger_read "$SESSION_ID") \
  | grep -v '^$' || true)
[ -z "$session_lines" ] && exit 0

file_count=$(printf '%s\n' "$session_lines" | grep -c . || true)
shown=$(printf '%s\n' "$session_lines" | head -20)
[ "$file_count" -gt 20 ] && shown="${shown}
... (+$((file_count - 20)) more)"

# Signature = session file list + tracked-content diff, so both a new
# file and more edits to a listed file count as "new work".
sig=$( { printf '%s\n' "$session_lines"; git -C "$CWD" diff HEAD 2>/dev/null; } \
  | shasum -a 256 | cut -d' ' -f1)
sig_changed claude-commit-nudge "$sig" || exit 0

fire "$(printf 'Stop-hook checkpoint (silent — output NO prose either way): this session wrote %s file(s) that are still uncommitted:\n%s\n\nThese are files THIS session wrote (recorded per tool call), so they are yours to commit. Group them into one or more coherent, uncontroversial commits with clear messages. Stage scoped by path: `git add <path>`. A file here may still hold another session edits interleaved with yours — if the diff shows hunks you did not write, stage only your own (`git add -p`), never `-A`. The commit(s) your ONLY output. If mid-flight, just stop. Never narrate this checkpoint. Quiet until the work changes.' \
  "$file_count" "$shown")"
