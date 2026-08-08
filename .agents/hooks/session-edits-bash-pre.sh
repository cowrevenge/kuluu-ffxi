#!/usr/bin/env bash
# PreToolUse(Bash): snapshot the dirty set so the Post counterpart can
# attribute whatever this command writes. Covers the writes Edit/Write
# never see — sed -i, cargo fmt, codegen, rm, heredocs.
#
# Snapshot is keyed by session + a hash of the command so parallel Bash
# calls in one turn don't clobber each other's baseline.
#
# Contract: never blocks, never speaks.

set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=./session-edits.lib.sh
. "$here/session-edits.lib.sh"

payload=$(cat)
session_id=$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)
cwd=$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)
[ -z "$cwd" ] && cwd="$PWD"
[ -n "$session_id" ] || exit 0
git -C "$cwd" rev-parse --git-dir >/dev/null 2>&1 || exit 0

cmd=$(printf '%s' "$payload" | jq -r '.tool_input.command // empty' 2>/dev/null || true)
key=$(printf '%s' "$cmd" | shasum -a 256 | cut -c1-16)

snap_dir="$(ledger_dir)/bashpre"
mkdir -p "$snap_dir" || exit 0
git -C "$cwd" status --porcelain 2>/dev/null > "$snap_dir/${session_id}.${key}" || true
exit 0
