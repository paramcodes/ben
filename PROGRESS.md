# Project progress

Last updated: 2026-10-03

## Project records

- Design spec: [terminal coding agent design](docs/superpowers/specs/2026-10-03-terminal-coding-agent-design.md) — committed as `c00353b`.
- Implementation plan: [46-ticket plan](docs/superpowers/plans/2026-10-03-terminal-coding-agent.md) — approved; tickets are tracked as GitHub issues.
- Repository: [paramcodes/ben](https://github.com/paramcodes/ben) — public.

## Completion rule

A ticket is **Complete** only when its pull request is merged. Record the merged PR number and link in the Evidence column. Work in progress belongs on a ticket branch and is marked **In progress** while its PR is open. An open issue, local commit, or passing local checks alone does not count as complete.

## Milestone status

| Milestone | Tickets | Status |
|---|---:|---|
| 0 — Rust project and terminal lifecycle | 1–4 | In progress |
| 1 — TUI shell and state | 5–8 | Not started |
| 2 — Configuration and provider boundary | 9–13 | Not started |
| 3 — Streaming and agent loop | 14–20 | Not started |
| 4 — Workspace context and read-only tools | 21–26 | Not started |
| 5 — Approval and side-effecting tools | 27–32 | Not started |
| 6 — Sessions and robust recovery | 33–36 | Not started |
| 7 — Product usability and release | 37–40 | Not started |
| 8 — Performance evidence and stack comparison | 41–46 | Not started |

## Ticket ledger

Status values: **Open**, **In progress**, **Complete**. Keep issue and PR links in this table. Add a short note only when useful; the PR is the detailed change record.

| Task | Ticket | Status | Evidence / note |
|---:|---|---|---|
| 1 | [#1](https://github.com/paramcodes/ben/issues/1) | Complete | [PR #47](https://github.com/paramcodes/ben/pull/47) — merged 2026-10-03 |
| 2 | [#2](https://github.com/paramcodes/ben/issues/2) | Complete | [PR #48](https://github.com/paramcodes/ben/pull/48) — merged 2026-10-03 |
| 3 | [#3](https://github.com/paramcodes/ben/issues/3) | In progress | [PR #49](https://github.com/paramcodes/ben/pull/49) |
| 4 | [#4](https://github.com/paramcodes/ben/issues/4) | Open | |
| 5 | [#5](https://github.com/paramcodes/ben/issues/5) | Open | |
| 6 | [#6](https://github.com/paramcodes/ben/issues/6) | Open | |
| 7 | [#7](https://github.com/paramcodes/ben/issues/7) | Open | |
| 8 | [#8](https://github.com/paramcodes/ben/issues/8) | Open | |
| 9 | [#9](https://github.com/paramcodes/ben/issues/9) | Open | |
| 10 | [#10](https://github.com/paramcodes/ben/issues/10) | Open | |
| 11 | [#11](https://github.com/paramcodes/ben/issues/11) | Open | |
| 12 | [#12](https://github.com/paramcodes/ben/issues/12) | Open | |
| 13 | [#13](https://github.com/paramcodes/ben/issues/13) | Open | |
| 14 | [#14](https://github.com/paramcodes/ben/issues/14) | Open | |
| 15 | [#15](https://github.com/paramcodes/ben/issues/15) | Open | |
| 16 | [#16](https://github.com/paramcodes/ben/issues/16) | Open | |
| 17 | [#17](https://github.com/paramcodes/ben/issues/17) | Open | |
| 18 | [#18](https://github.com/paramcodes/ben/issues/18) | Open | |
| 19 | [#19](https://github.com/paramcodes/ben/issues/19) | Open | |
| 20 | [#20](https://github.com/paramcodes/ben/issues/20) | Open | |
| 21 | [#21](https://github.com/paramcodes/ben/issues/21) | Open | |
| 22 | [#22](https://github.com/paramcodes/ben/issues/22) | Open | |
| 23 | [#23](https://github.com/paramcodes/ben/issues/23) | Open | |
| 24 | [#24](https://github.com/paramcodes/ben/issues/24) | Open | |
| 25 | [#25](https://github.com/paramcodes/ben/issues/25) | Open | |
| 26 | [#26](https://github.com/paramcodes/ben/issues/26) | Open | |
| 27 | [#27](https://github.com/paramcodes/ben/issues/27) | Open | |
| 28 | [#28](https://github.com/paramcodes/ben/issues/28) | Open | |
| 29 | [#29](https://github.com/paramcodes/ben/issues/29) | Open | |
| 30 | [#30](https://github.com/paramcodes/ben/issues/30) | Open | |
| 31 | [#31](https://github.com/paramcodes/ben/issues/31) | Open | |
| 32 | [#32](https://github.com/paramcodes/ben/issues/32) | Open | |
| 33 | [#33](https://github.com/paramcodes/ben/issues/33) | Open | |
| 34 | [#34](https://github.com/paramcodes/ben/issues/34) | Open | |
| 35 | [#35](https://github.com/paramcodes/ben/issues/35) | Open | |
| 36 | [#36](https://github.com/paramcodes/ben/issues/36) | Open | |
| 37 | [#37](https://github.com/paramcodes/ben/issues/37) | Open | |
| 38 | [#38](https://github.com/paramcodes/ben/issues/38) | Open | |
| 39 | [#39](https://github.com/paramcodes/ben/issues/39) | Open | |
| 40 | [#40](https://github.com/paramcodes/ben/issues/40) | Open | |
| 41 | [#41](https://github.com/paramcodes/ben/issues/41) | Open | |
| 42 | [#42](https://github.com/paramcodes/ben/issues/42) | Open | |
| 43 | [#43](https://github.com/paramcodes/ben/issues/43) | Open | |
| 44 | [#44](https://github.com/paramcodes/ben/issues/44) | Open | |
| 45 | [#45](https://github.com/paramcodes/ben/issues/45) | Open | |
| 46 | [#46](https://github.com/paramcodes/ben/issues/46) | Open | |

## PR history

| PR | Ticket | Merged | Summary |
|---|---:|---|---|
| [PR #47](https://github.com/paramcodes/ben/pull/47) | 1 | Merged 2026-10-03 | chore: scaffold terminal agent |
| [PR #48](https://github.com/paramcodes/ben/pull/48) | 2 | Merged 2026-10-03 | chore: pin initial Rust dependencies |
