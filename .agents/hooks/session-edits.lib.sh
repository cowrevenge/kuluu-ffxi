#!/usr/bin/env bash
# Shared accessors for the per-session edit ledger. Source this; do not
# execute it.
#
# The ledger answers "which paths did THIS session write?" — the question
# a SessionStart porcelain snapshot cannot answer, because "dirty now but
# not dirty at session start" also captures every concurrent writer in a
# shared checkout. Stop checks intersect their dirty set against it so a
# neighbouring session's edits can never be attributed here.
#
# Ledger path: $TMPDIR/claude-session-edits/<session_id>.paths
# One repo-relative path per line, append-only, deduped on read.

ledger_dir() { printf '%s/claude-session-edits' "${TMPDIR:-/tmp}"; }

ledger_path() {
  [ -n "${1:-}" ] || return 1
  printf '%s/%s.paths' "$(ledger_dir)" "$1"
}

# ledger_add <session_id> <cwd> <abs-or-relative-path>...
ledger_add() {
  local sid="$1" cwd="$2" file rel
  shift 2
  [ -n "$sid" ] || return 0
  local out
  out=$(ledger_path "$sid") || return 0
  mkdir -p "$(ledger_dir)" || return 0
  for file in "$@"; do
    [ -n "$file" ] || continue
    rel="$file"
    case "$rel" in "$cwd"/*) rel="${rel#"$cwd"/}" ;; esac
    printf '%s\n' "$rel" >> "$out"
  done
}

# ledger_read <session_id>: sorted unique paths, empty when absent.
ledger_read() {
  local f
  f=$(ledger_path "${1:-}") || return 0
  [ -f "$f" ] && sort -u "$f" || true
}

# ledger_exists <session_id>
ledger_exists() {
  local f
  f=$(ledger_path "${1:-}") || return 1
  [ -f "$f" ]
}
