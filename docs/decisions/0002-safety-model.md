# Approval and safety model

## Decision

The agent must obtain explicit user approval before every side-effecting action. Approvals are bound to an immutable action fingerprint; a changed command, arguments, or path invalidates the earlier approval.

## Context

The plan requires that model-generated edits and commands require approval before side effects. The first release must not include a broad "always allow" mode.

## Consequences

- Every proposed action is displayed with its exact command, arguments, and target paths.
- The user approves once, rejects, or cancels each action individually.
- Approved edits are shown as a unified diff before application.
- Actions are confined to the selected workspace root; path traversal and symlink escapes are rejected.
- API keys are never persisted to session files or logs.
- Session data is stored locally with restrictive permissions and atomic writes.

## Alternatives considered

| Approach | Trade-off | Decision |
|---|---|---|
| Always-allow mode | Convenient but unsafe for a local-first tool | Rejected for initial release |
| Approval with TTL | Reduces friction but adds complexity | Deferred |
| Automatic approval for read-only tools | Read-only tools are already safe | Not needed; read-only tools execute without approval |