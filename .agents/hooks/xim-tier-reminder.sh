#!/usr/bin/env bash
# PostToolUse hook (Read|Grep|Glob): when the agent reads research/xim, remind
# it that XIM is the lowest-ranked reference and to confirm against XIClient
# before acting on anything it says.
#
# Why this exists: XIM's author states it reproduces what is *observable*, not
# what the client computes, so it is silently wrong wherever an effect is subtle
# in-game — not just on bit widths. It cost us a wrong reading of the chase
# camera's triangle-skip rule (kuluu-eg5g): XIM had the shape right and the
# predicate wrong. XIM is still the fastest way to find *where* to look, hence a
# nudge rather than a block.
#
# Throttle: once per session — the point is to set the frame, not to nag.

set -euo pipefail

payload=$(cat)
session_id=$(printf '%s' "$payload" | jq -r '.session_id // empty')

# Read uses tool_input.file_path; Grep/Glob use tool_input.path. Grep can also
# be scoped by glob/pattern alone, so fall back to the whole tool_input.
target=$(printf '%s' "$payload" | jq -r '
  .tool_input.file_path // .tool_input.path // (.tool_input | tostring) // empty')

[ -z "$session_id" ] && exit 0
[ -z "$target" ] && exit 0

case "$target" in
*research/xim*) ;;
*) exit 0 ;;
esac

throttle_dir="${TMPDIR:-/tmp}/claude-xim-tier"
mkdir -p "$throttle_dir"
marker="$throttle_dir/${session_id}"
[ -f "$marker" ] && exit 0
touch "$marker"

msg='research/xim is tier 6 — the lowest-ranked reference (research/AGENTS.md).

Its author states XIM reproduces what is observable, not what the client
computes, so it is silently wrong wherever an effect is subtle in-game. That is
wider than bit widths: a detail XIM omits is weak evidence that retail omits it.

Use it to find WHERE to look, then confirm the answer in research/XIClient
(tier 2, disassembly-grounded) before citing it or writing code against it.
XIClient carries retail runtime policies named and intact, not just structs.'

jq -n --arg m "$msg" '{
  systemMessage: $m,
  hookSpecificOutput: {hookEventName: "PostToolUse", additionalContext: $m}
}'
