#!/usr/bin/env python3
"""Check that the roadmap and durable progress ledger stay in sync."""

from pathlib import Path
import json
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
PLAN = ROOT / "docs/superpowers/plans/2026-10-03-terminal-coding-agent.md"
PROGRESS = ROOT / "PROGRESS.md"
MAP_PATH = ROOT / ".github/roadmap-issues.json"


def main() -> int:
    plan_text = PLAN.read_text(encoding="utf-8")
    progress_text = PROGRESS.read_text(encoding="utf-8")
    task_ids = [int(n) for n in re.findall(r"^### Task (\d+):", plan_text, re.M)]
    ledger_ids = [int(n) for n in re.findall(r"^\| (\d+) \|", progress_text, re.M)]

    expected = list(range(1, 47))
    if task_ids != expected:
        print(f"plan task sequence is not 1–46: found {task_ids}", file=sys.stderr)
        return 1
    if ledger_ids != expected:
        print(f"progress ledger does not list tasks 1–46: found {ledger_ids}", file=sys.stderr)
        return 1
    if "Complete** only when its pull request is merged" not in progress_text:
        print("progress completion rule must require a merged PR", file=sys.stderr)
        return 1
    rows = re.findall(r"^\| (\d+) \| (.+?) \| (Open|In progress|Complete) \|[ \t]*(.*?)\|$", progress_text, re.M)
    if len(rows) != 46:
        print(f"expected 46 valid progress rows, found {len(rows)}", file=sys.stderr)
        return 1
    for number, ticket, status, evidence in rows:
        if status == "Complete" and not re.search(r"\bPR #\d+\b.*\bmerged\b", evidence, re.I):
            print(f"Task {number} is Complete without merged PR evidence", file=sys.stderr)
            return 1
        if ticket != "TBD" and not re.search(r"https://github\.com/paramcodes/ben/issues/\d+", ticket):
            print(f"Task {number} ticket link is malformed: {ticket}", file=sys.stderr)
            return 1
    if MAP_PATH.exists():
        issue_map = json.loads(MAP_PATH.read_text(encoding="utf-8"))
        if set(issue_map) != {str(n) for n in expected}:
            print("GitHub issue map does not contain tasks 1 through 46", file=sys.stderr)
            return 1
        for number, issue in issue_map.items():
            if issue.get("url") not in progress_text:
                print(f"Task {number} issue URL is missing from PROGRESS.md", file=sys.stderr)
                return 1
    print(f"OK: {len(task_ids)} plan tickets match {len(ledger_ids)} progress rows")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
