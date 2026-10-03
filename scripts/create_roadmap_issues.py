#!/usr/bin/env python3
"""Create one well-structured GitHub issue per implementation-plan task."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
PLAN_PATH = ROOT / "docs/superpowers/plans/2026-10-03-terminal-coding-agent.md"
MAP_PATH = ROOT / ".github/roadmap-issues.json"


def parse_tasks() -> list[dict[str, object]]:
    lines = PLAN_PATH.read_text(encoding="utf-8").splitlines()
    milestone = "Roadmap"
    tasks: list[dict[str, object]] = []
    current: dict[str, object] | None = None
    for line in lines:
        if line.startswith("## Milestone "):
            milestone = line.removeprefix("## ")
        match = re.match(r"### Task (\d+): (.+)", line)
        if match:
            if current:
                tasks.append(current)
            current = {
                "number": int(match.group(1)),
                "name": match.group(2),
                "milestone": milestone,
                "learning": "",
                "files": "",
                "deliverable": "",
                "steps": [],
            }
            continue
        if current is None:
            continue
        if line.startswith("**Learning goal:**"):
            current["learning"] = line.removeprefix("**Learning goal:**").strip()
        elif line.startswith("**Files:**"):
            current["files"] = line.removeprefix("**Files:**").strip()
        elif line.startswith("**Produces:**"):
            current["deliverable"] = line.removeprefix("**Produces:**").strip()
        elif line.startswith("- [ ] "):
            steps = current["steps"]
            assert isinstance(steps, list)
            steps.append(line.removeprefix("- [ ] ").strip())
    if current:
        tasks.append(current)
    return tasks


def task_dependencies(number: int) -> list[int]:
    if number == 41:
        return []
    if number in (43, 44):
        return [41]
    if number == 45:
        return [42, 43, 44]
    if number == 46:
        return [10, 18]
    if number in (42,):
        return [41]
    if number in range(9, 41):
        return [number - 1]
    if number in range(5, 9):
        return [number - 1]
    if number in range(2, 5):
        return [number - 1]
    return []


def issue_body(task: dict[str, object]) -> str:
    number = int(task["number"])
    name = str(task["name"])
    learning = str(task["learning"])
    files = str(task["files"])
    deliverable = str(task["deliverable"])
    steps = task["steps"]
    assert isinstance(steps, list)
    deps = task_dependencies(number)
    dep_text = ", ".join(f"Task {item}" for item in deps) if deps else "None"
    plan_anchor = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    how = "\n".join(f"- {step}" for step in steps)
    return f"""## What

{deliverable}

## Why

{learning} This ticket delivers one runnable, reviewable increment in the {task['milestone']} roadmap.

## How

**Files:** {files}

Follow the ticket steps below in order. Keep the change limited to this deliverable and use local deterministic tests; do not require a live model API key.

{how}

## Acceptance criteria

- The deliverable above works as described.
- Focused automated checks pass and the PR records exact CLI commands and results.
- The PR updates `PROGRESS.md`; this ticket is marked Complete only after its PR merges.

## Dependencies

{dep_text}

## Source

Task {number} in [`docs/superpowers/plans/2026-10-03-terminal-coding-agent.md`](https://github.com/paramcodes/ben/blob/main/docs/superpowers/plans/2026-10-03-terminal-coding-agent.md#task-{number}-{plan_anchor}).
"""


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--repo", default="paramcodes/ben", help="GitHub owner/repository")
    parser.add_argument("--create", action="store_true", help="Create missing issues with gh")
    parser.add_argument("--map-file", type=Path, default=MAP_PATH)
    args = parser.parse_args()

    tasks = parse_tasks()
    if len(tasks) != 46 or [int(task["number"]) for task in tasks] != list(range(1, 47)):
        print("Expected exactly plan tasks 1 through 46", file=sys.stderr)
        return 1
    if not args.create:
        for task in tasks:
            print(f"[Task {int(task['number']):02}] {task['name']}")
        print(f"Validated {len(tasks)} issues. Pass --create to publish them.")
        return 0

    check = subprocess.run(["gh", "auth", "status"], cwd=ROOT, check=False)
    if check.returncode:
        return check.returncode
    existing_run = subprocess.run(
        ["gh", "issue", "list", "--repo", args.repo, "--state", "all", "--limit", "300", "--json", "number,title,url"],
        cwd=ROOT,
        check=True,
        capture_output=True,
        text=True,
    )
    existing = {issue["title"]: issue for issue in json.loads(existing_run.stdout)}
    mapping: dict[str, dict[str, object]] = {}
    for task in tasks:
        number = int(task["number"])
        title = f"[Task {number:02}] {task['name']}"
        issue = existing.get(title)
        if issue is None:
            created = subprocess.run(
                ["gh", "issue", "create", "--repo", args.repo, "--title", title, "--body", issue_body(task)],
                cwd=ROOT,
                check=True,
                capture_output=True,
                text=True,
            )
            url = created.stdout.strip().splitlines()[-1]
            issue = {"title": title, "url": url, "number": int(url.rstrip("/").split("/")[-1])}
            print(f"Created {title}: {url}", flush=True)
        else:
            print(f"Exists  {title}: {issue['url']}", flush=True)
        mapping[str(number)] = {"number": issue["number"], "url": issue["url"], "title": title}
    args.map_file.parent.mkdir(parents=True, exist_ok=True)
    args.map_file.write_text(json.dumps(mapping, indent=2) + "\n", encoding="utf-8")
    print(f"Wrote issue map to {args.map_file.relative_to(ROOT)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
