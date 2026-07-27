---
name: beads-github-sync
description: How beads issues are projected to GitHub Issues and (optionally) imported back. Use when publishing beads to GitHub, debugging why a bead's issue is stale or missing, editing scripts/beads-github-publish.py or scripts/beads-github-sync.sh, or touching .github/workflows/beads-github-publish.yml.
---

# Beads ↔ GitHub Issues

GitHub Issues are a **generated projection of beads** for contributors, not a second source of truth. Beads (`.beads/`) is authoritative; never treat a GitHub issue edit as the durable record.

## Outbound: beads → GitHub

`scripts/beads-github-publish.py` is the publisher. Each in-scope bead maps to one issue keyed by a `<!-- beads-id: <id> -->` body marker. It keeps in sync:

- title and body
- managed labels: `vanilla-parity` / `enhanced` / `area:*` / `status:*`
- open/closed state — closing a bead closes its issue

`.github/workflows/beads-github-publish.yml` runs it automatically on every push to `main` that touches `.beads/issues.jsonl`, publishing **all** beads (not just `roadmap`-labelled ones). `workflow_dispatch` remains available for manual runs, where `dry_run` defaults to true.

**Beads edits only reach GitHub once the auto-exported `.beads/issues.jsonl` is committed and pushed.** A bead changed locally but not exported/pushed will look stale on GitHub — that's the usual cause of "my issue didn't update".

## Inbound: GitHub → beads

`scripts/beads-github-sync.sh` is independent and opt-in. It imports GitHub issues *into* beads, keyed on `external_ref: gh-<number>`.

The two directions are **not** a loop — don't round-trip the same issues through both.
