#!/usr/bin/env bash
# Escape hatch for a ledger misattribution: drop one or more paths from a
# single session's edit ledger and leave every other session's alone, so the
# tree stays red for whoever actually owns the change.
#
#   .agents/hooks/session-edits-forget.sh --session <id> <path>...
#
# Hand-run or callable from a hook; deliberately registered nowhere, because
# nothing should be forgetting paths automatically.
#
# Contract: never blocks, never speaks.

set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=./session-edits.lib.sh
. "$here/session-edits.lib.sh"

sid="${CLAUDE_SESSION_ID:-}"
cwd="$PWD"
paths=()
while [ $# -gt 0 ]; do
  case "$1" in
    --session) sid="${2:-}"; shift 2 ;;
    --session=*) sid="${1#--session=}"; shift ;;
    --cwd) cwd="${2:-}"; shift 2 ;;
    --cwd=*) cwd="${1#--cwd=}"; shift ;;
    --) shift; while [ $# -gt 0 ]; do paths+=("$1"); shift; done ;;
    *) paths+=("$1"); shift ;;
  esac
done

[ -n "$sid" ] || exit 0
[ "${#paths[@]}" -gt 0 ] || exit 0
# Ledger lines are worktree-root relative, so a hand-run from a
# subdirectory has to be normalised the same way the hooks do.
ledger_forget "$sid" "$(repo_root "$cwd")" "${paths[@]}"
exit 0
