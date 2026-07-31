# Harness configuration (`.agents/` canonical, `.claude/` = adapter)

Agent-facing config lives in **`.agents/`**, harness-neutral, mirroring the
`CLAUDE.md → AGENTS.md` symlink pattern:

- `.agents/skills/` — [agentskills.io](https://agentskills.io/specification)
  standard layout. pi discovers it natively at the project level; Claude
  Code follows the `.claude/skills` symlink.
- `.agents/agents/` — subagent definitions (Markdown + frontmatter),
  symlinked from `.claude/agents`.
- `.agents/hooks/` — standalone shell scripts speaking the Claude Code hook
  wire protocol: a JSON payload on stdin (`session_id`, `transcript_path`,
  `tool_name`, `stop_hook_active`, …), decisions via exit code / stdout
  JSON. Registration lives in `.claude/settings.json`, but the scripts
  themselves are harness-agnostic — any harness with an equivalent event
  surface (pi extensions expose `session_start`, `tool_call`,
  `agent_settled`) can bridge them. Each `stop.d/` check is independently
  testable: `echo "$payload" | .agents/hooks/stop.d/20-commit.sh; echo $?`
  (exit 0 = pass, exit 10 = fire with the reason on stdout).

`.claude/` keeps only Claude-specific wiring: `settings.json` (permissions
+ hook registration) and the two symlinks. Per-user/runtime state
(`settings.local.json`, `.bandwidth/`, `worktrees/`) stays gitignored
there.
