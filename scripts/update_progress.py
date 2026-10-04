#!/usr/bin/env python3
"""Synchronize one roadmap row with a pull request's live GitHub state."""

from __future__ import annotations

import argparse
from datetime import date
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
PROGRESS = ROOT / "PROGRESS.md"
ISSUE_MAP = ROOT / ".github/roadmap-issues.json"
REPO = "paramcodes/ben"


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--task", type=int, required=True, choices=range(1, 47))
    parser.add_argument("--pr", type=int, required=True)
    args = parser.parse_args()

    issue_map = json.loads(ISSUE_MAP.read_text(encoding="utf-8"))
    issue_number = int(issue_map[str(args.task)]["number"])
    result = subprocess.run(
        ["gh", "pr", "view", str(args.pr), "--repo", REPO, "--json", "number,title,url,state,mergedAt,body"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    pr = json.loads(result.stdout)
    body = pr.get("body") or ""
    if pr["number"] != args.pr or not re.search(rf"\bCloses\s+#{issue_number}\b", body, re.I):
        print(f"PR #{args.pr} must close roadmap issue #{issue_number}", file=sys.stderr)
        return 1

    if pr.get("mergedAt"):
        status = "Complete"
        merged_on = pr["mergedAt"][:10]
        evidence = f"[PR #{args.pr}]({pr['url']}) — merged {merged_on}"
        history_state = f"Merged {merged_on}"
    elif pr.get("state") == "OPEN":
        status = "In progress"
        evidence = f"[PR #{args.pr}]({pr['url']}) — open"
        history_state = "Open"
    else:
        print(f"PR #{args.pr} is closed without a merge; reopen the ticket before recording progress", file=sys.stderr)
        return 1

    text = PROGRESS.read_text(encoding="utf-8")
    pattern = re.compile(rf"^\| {args.task} \| (.*?) \| (Open|In progress|Complete) \| (.*?)\s*\|$", re.M)
    replacement = f"| {args.task} | [#{issue_number}](https://github.com/{REPO}/issues/{issue_number}) | {status} | {evidence} |"
    text, count = pattern.subn(replacement, text)
    if count != 1:
        print(f"Expected one progress row for Task {args.task}, found {count}", file=sys.stderr)
        return 1

    history_row = f"| [PR #{args.pr}]({pr['url']}) | {args.task} | {history_state} | {pr['title']} |"
    history_pattern = re.compile(rf"^\| \[PR #{args.pr}\].*\| {args.task} \|.*$", re.M)
    if history_pattern.search(text):
        text = history_pattern.sub(history_row, text)
    else:
        text = text.replace("| — | — | — | No implementation PRs merged yet. |", history_row)
        if history_row not in text:
            text = text.rstrip() + "\n" + history_row + "\n"

    text = re.sub(r"(?m)^Last updated: .*?$", f"Last updated: {date.today().isoformat()}", text, count=1)
    text = update_milestone_status(text, args.task, status)
    PROGRESS.write_text(text, encoding="utf-8")
    print(f"Task {args.task}: {status}; PR #{args.pr} {history_state.lower()}")
    return 0


def update_milestone_status(text: str, changed_task: int, changed_status: str) -> str:
    ranges = [(0, 4), (4, 8), (8, 13), (13, 20), (20, 26), (26, 32), (32, 36), (36, 40), (40, 46)]
    for milestone_index, (start, end) in enumerate(ranges):
        if start < changed_task <= end:
            states = []
            for task in range(start + 1, end + 1):
                match = re.search(rf"(?m)^\| {task} \| .*? \| (Open|In progress|Complete) \|", text)
                states.append(match.group(1) if match else "Open")
            milestone_status = "Complete" if all(state == "Complete" for state in states) else (
                "In progress" if any(state != "Open" for state in states) else "Not started"
            )
            row_pattern = re.compile(
                rf"(?m)^(\| {milestone_index} — .*? \| \d+–\d+ \| )(Not started|In progress|Complete)( \|)$"
            )
            return row_pattern.sub(rf"\g<1>{milestone_status}\g<3>", text, count=1)
    return text


if __name__ == "__main__":
    raise SystemExit(main())
