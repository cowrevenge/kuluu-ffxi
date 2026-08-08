#!/usr/bin/env bash
# PostToolUse(Edit|Write|NotebookEdit): record the written path in this
# session's edit ledger. Direct attribution — the tool payload names the
# file, so no inference from tree state is needed.
#
# Contract: never blocks, never speaks. Any failure exits 0 silently; a
# missing ledger entry only costs a Stop check some precision.

set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=./session-edits.lib.sh
. "$here/session-edits.lib.sh"

payload=$(cat)
session_id=$(printf '%s' "$payload" | jq -r '.session_id // empty' 2>/dev/null || true)
cwd=$(printf '%s' "$payload" | jq -r '.cwd // empty' 2>/dev/null || true)
[ -z "$cwd" ] && cwd="$PWD"
[ -n "$session_id" ] || exit 0

# Edit/Write use tool_input.file_path; the response echoes filePath on some
# tools. Take whichever is present.
file=$(printf '%s' "$payload" \
  | jq -r '.tool_response.filePath // .tool_input.file_path // .tool_input.notebook_path // empty' \
  2>/dev/null || true)
[ -n "$file" ] || exit 0

ledger_add "$session_id" "$cwd" "$file"
exit 0
