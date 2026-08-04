# Harness configuration (`.agents/` canonical, `.claude/` = adapter)

Agent-facing config lives in **`.agents/`**, harness-neutral, mirroring the
`CLAUDE.md → AGENTS.md` symlink pattern. `.claude/` holds only what a
Claude-specific path must hold.

**The rule: how a harness *discovers* a thing decides how it gets wired.**

| Kind | Discovery | Wiring | Example |
| --- | --- | --- | --- |
| Skills, subagents | Harness scans a well-known directory | Real content in `.agents/`, symlink at the well-known path | `.claude/skills → ../.agents/skills`, `.claude/agents → ../.agents/agents` |
| Hooks | Harness takes an explicit path from a config file | Real scripts in `.agents/hooks/`, path pointer in the config — **no directory to symlink, so `.claude/hooks/` does not exist** | `${CLAUDE_PROJECT_DIR}/.agents/hooks/…` in `.claude/settings.json` |
| Root instruction file | Harness reads a fixed filename | `AGENTS.md` is canonical; the harness-specific name is a symlink | `CLAUDE.md → AGENTS.md` |
| Per-user / runtime state | Harness writes it | Real file under `.claude/`, gitignored, never mirrored | `settings.local.json`, `.bandwidth/`, `worktrees/` |

Git tracks exactly three things under `.claude/`: the two symlinks and
`settings.json`. Anything with content belongs in `.agents/`.

`scripts/checks.sh harness` enforces the table — broken symlink, symlink
escaping `.agents/`, missing/non-executable hook path, a reappeared
`.claude/hooks/`, or a doc citing a `.claude/` path that isn't tracked all
fail the stage. It runs first in `.githooks/pre-push` and as its own CI step.

## Layout

- `.agents/skills/` — [agentskills.io](https://agentskills.io/specification)
  standard layout. pi discovers it natively at the project level; Claude
  Code follows the `.claude/skills` symlink.
- `.agents/agents/` — subagent definitions (Markdown + frontmatter),
  symlinked from `.claude/agents`.
- `.agents/hooks/` — standalone shell scripts speaking the Claude Code hook
  wire protocol: a JSON payload on stdin (`session_id`, `transcript_path`,
  `tool_name`, `stop_hook_active`, …), decisions via exit code / stdout
  JSON. The scripts are the portable asset; registration is the adapter.
  Any harness with an equivalent event surface (pi extensions expose
  `session_start`, `tool_call`, `agent_settled`) can bridge them. Each
  `stop.d/` check is independently testable:
  `echo "$payload" | .agents/hooks/stop.d/20-commit.sh; echo $?`
  (exit 0 = pass, exit 10 = fire with the reason on stdout).

## `ffxi-agent/` is a separate, intentional exception

`ffxi-agent/` ships its own `.claude/{hooks,agents,skills,settings.json}`
as *real* directories, plus an `opencode.json` that reads a `.claude/`
path. That is not drift: it is the runtime playbook for an LLM agent
*playing the game*, shipped as a unit, with a different audience from this
repo's dev harness. It is out of scope for the table above, and
`scripts/checks.sh harness` skips it.
