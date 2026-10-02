import contextlib
import importlib.util
import io
import json
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch


SCRIPT = Path(__file__).resolve().parents[1] / "beads-github-publish.py"
SPEC = importlib.util.spec_from_file_location("beads_github_publish", SCRIPT)
publisher = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(publisher)


class DryRunTest(unittest.TestCase):
    def test_existing_projections_are_planned_without_mutations(self):
        beads = [
            {"id": "changed", "title": "Updated title", "status": "closed"},
            {"id": "reopened", "title": "Reopened", "status": "open"},
            {"id": "new", "title": "New", "status": "open"},
            {"id": "finished", "title": "Finished", "status": "closed"},
            {"id": "unchanged", "title": "Unchanged", "status": "open"},
        ]
        repo = "test/repo"
        issues = []
        for number, bead in enumerate((beads[0], beads[1], beads[4]), start=1):
            issues.append({
                "number": number,
                "title": bead["title"],
                "body": publisher.project_body(bead, repo),
                "state": "CLOSED" if bead["id"] == "reopened" else "OPEN",
                "labels": [
                    {"name": label} for label in publisher.bead_labels_to_gh(bead)
                ],
            })
        issues[0]["title"] = "Old title"
        issues[0]["body"] = "Old description\n" + publisher.MARKER.format(
            id="changed"
        )
        issues[0]["labels"] = [
            {"name": "status:missing"}, {"name": "hand-applied"}
        ]

        def read_only_gh(command, **kwargs):
            self.assertEqual(command[:3], ["gh", "issue", "list"])
            self.assertIn("all", command)
            self.assertTrue(kwargs["capture_output"])
            return subprocess.CompletedProcess(command, 0, json.dumps(issues))

        for argv, environment in [
            ([str(SCRIPT), "--repo", repo, "--all", "--dry-run"], {}),
            ([str(SCRIPT), "--repo", repo, "--all"], {"DRY_RUN": "1"}),
        ]:
            with (
                self.subTest(argv=argv, environment=environment),
                tempfile.TemporaryDirectory() as directory,
            ):
                export = Path(directory) / "issues.jsonl"
                export.write_text("\n".join(json.dumps(bead) for bead in beads))
                output = io.StringIO()
                with (
                    patch.object(publisher, "JSONL", export),
                    patch.object(publisher.sys, "argv", argv),
                    patch.dict(publisher.os.environ, environment, clear=True),
                    patch.object(
                        publisher.subprocess, "run", side_effect=read_only_gh
                    ) as executed,
                    contextlib.redirect_stdout(output),
                ):
                    self.assertEqual(publisher.main(), 0)

                executed.assert_called_once()
                plan = output.getvalue()
                self.assertIn("update: #1 changed Updated title", plan)
                self.assertIn("close:  #1 changed", plan)
                self.assertIn("reopen: #2 reopened", plan)
                self.assertIn("create: [status:missing] new New", plan)
                self.assertNotIn("create: [] changed", plan)
                self.assertNotIn("issue edit 3", plan)
                self.assertNotIn("--remove-label hand-applied", plan)
                self.assertIn(
                    "created=1 updated=1 closed=1 reopened=1 pruned=0 "
                    "skipped(closed,unpublished)=1",
                    plan,
                )
                self.assertIn("GitHub was not modified", plan)


if __name__ == "__main__":
    unittest.main()
