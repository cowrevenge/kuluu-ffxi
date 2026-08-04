#!/usr/bin/env bash
# PreToolUse hook (Edit|Write): scan the comment lines in the text
# *about to be written* for rot heuristics and surface a one-line,
# non-blocking heads-up so the agent self-corrects before the edit
# lands. Mirrors lsb-boundary-reminder.sh — stderr + exit 0, never
# blocks (the regexes are too blunt to gate work on).
#
# For Edit the incoming text is tool_input.new_string; for Write it is
# tool_input.content. Either way we only see what is being added, which
# is exactly the new comments — pre-existing ones are not re-flagged.

set -uo pipefail

dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=comment-rot.lib.sh
. "$dir/comment-rot.lib.sh"

payload=$(cat)
text=$(printf '%s' "$payload" | /usr/bin/python3 -c \
  'import json,sys; d=json.load(sys.stdin).get("tool_input",{}); print(d.get("new_string", d.get("content","")), end="")' \
  2>/dev/null || true)

[ -z "$text" ] && exit 0

findings=$( { printf '%s\n' "$text" | scan_comment_rot || true; \
              printf '%s\n' "$text" | scan_code_magic || true; } \
            | grep -v '^[[:space:]]*$' || true)
[ -z "$findings" ] && exit 0

# printf with single-quoted literals, not a heredoc: an unquoted heredoc body
# runs command substitution, so a backtick in the prose (easy to reach for when
# naming a path) silently executes it.
printf '%s\n%s\n%s\n' >&2 \
  "[comment] This project bans narrative code comments — they rot, restate the code, or paper over names that should be clearer. The text you're about to write adds:" \
  "$findings" \
  'Default to NO comment: encode the intent in names/types/asserts (block comments are discouraged too). Keep one only if it is a non-obvious WHY you cannot encode, a citation to an external/vendor/protocol source, or a SAFETY justification (a magic literal wants a named const, not a comment). [citation pinned to a line number] is the one flag that does NOT mean delete: keep the citation, drop the ":NNN" and anchor on the symbol instead (the path plus the function/struct name) — a line number goes stale the next time that submodule advances, silently. [narrative/history] flags are prune-by-default: git log is the change history — describe the code as it IS, never how it changed. Doc comments (/// //!) are held to the same bar — tight and accurate, not rambling or stale. Ignore if a flag is a false positive.'
exit 0
