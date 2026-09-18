#!/usr/bin/env bash
# Stop sub-check (priority 45, last): the LSB dev stack outlives the work
# that needed it. If this session is settling and nothing is attached to
# the stack, stop the containers instead of leaving them resident.
#
# This check never blocks — it acts and passes. Blocking would spend a
# turn nagging about a chore the script can just do, and a Stop can land
# mid-task. The guards are what make silent action safe: a live client or
# MCP bridge (scripts/lsb-stack.sh in_use) keeps everything up, and the
# colima VM is left running so the next bring-up is a docker start.
#
# Escape hatch: LSB_STACK_AUTOSTOP=off.
#
# Contract: see stop-lib.sh. Exit 0 = pass; this check only ever passes.

set -uo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
# shellcheck source=../stop-lib.sh
. "$here/../stop-lib.sh"
load_payload

[ "${LSB_STACK_AUTOSTOP:-on}" = "off" ] && exit 0

stack="$CWD/scripts/lsb-stack.sh"
[ -x "$stack" ] || exit 0
command -v docker >/dev/null 2>&1 || exit 0

# Nothing running costs nothing to leave alone.
state=$("$stack" status 2>/dev/null)
printf '%s' "$state" | grep -q 'Up ' || exit 0
printf '%s' "$state" | grep -q 'held open by a live client' && exit 0

"$stack" down >/dev/null 2>&1
exit 0
