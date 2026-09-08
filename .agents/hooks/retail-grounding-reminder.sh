#!/usr/bin/env bash
# Advisory source reminder, throttled per session; never a parity/approval gate.
set -uo pipefail

payload=$(cat)
session_id=$(printf '%s' "$payload" | jq -er '.session_id | strings | select(length > 0)' 2>/dev/null) || exit 0
target=$(printf '%s' "$payload" | jq -r '
  .tool_input | .file_path // .path // .glob // empty | strings' 2>/dev/null) || exit 0

case "$target" in
  *research/XIClient*|*research/xim*|*research/cexi-docs*|*research/cexi-viewer*) ;;
  *) exit 0 ;;
esac

session_key=$(printf '%s' "$session_id" | shasum -a 256 | cut -d ' ' -f 1) || exit 0
throttle_dir="${TMPDIR:-/tmp}/kuluu-retail-grounding"
mkdir -p "$throttle_dir" 2>/dev/null || exit 0
mkdir "$throttle_dir/$session_key" 2>/dev/null || exit 0

msg='For vanilla client behavior, use .agents/skills/retail-grounding/SKILL.md and the source ranking in research/AGENTS.md. XIClient is the strongest community map; XIM and cexi can suggest where to look. Check available retail observation or binary/DAT evidence for the actual caller, predicates and transforms before adopting a community approximation. Reuse applicable verified findings. If retail evidence is unavailable, continue with the best source and label the inference; this is a nudge, not a blocking gate. Pure tooling or intentional enhancements do not require a retail comparison.'

jq -n --arg m "$msg" '{
  hookSpecificOutput: {hookEventName: "PostToolUse", additionalContext: $m}
}' || true
exit 0
