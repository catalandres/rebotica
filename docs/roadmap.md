# Roadmap

Atelier should grow through proof, not ambition.

## Phase 1: Shell Bridge

Current focus:

- Rust CLI.
- Provider health and smoke checks, with LM Studio as the default local provider.
- Review current diff.
- Explain files.
- Propose tests.
- Dry-run patch proposal.
- Run logging.
- Claude commands and skills.

## Phase 2: Better Guardrails

Next:

- Stronger task-envelope validation.
- Better parsing of worker output.
- Patch diff validation before display.
- Optional configured test/check execution.
- Project-level context packing rules.
- Provider fixture tests for auth, aliases, and error handling.

## Phase 3: Worktree Patch Workers

Patch drafting should happen in isolated git worktrees before direct application is considered.

Expected pattern:

```sh
git worktree add ../project-atelier-worker-1 -b ai/atelier-worker-1 main
cd ../project-atelier-worker-1
atelier patch .atelier/tasks/task.yml --dry-run
```

The root coordinator reviews the diff and runs checks.

## Phase 4: MCP Bridge

Only after the CLI proves useful, add the MCP server with narrow tools:

- review diff
- explain files
- propose tests
- propose patch
- health check
- score last run

No broad shell or write-file tools.

## Phase 5: Model Routing

Use scorecards to decide which local models are good for which work. Route empirically.

## Phase 6: Distribution

Make coworker installation boring:

- tagged releases
- checksums
- Homebrew tap
- migration notes for config changes

## Phase 7: Skills Broker

Make canonical Atelier skills installable across root-agent adapters:

- file-based Claude/Codex adapters first
- GitHub repository governance assets
- MCP resources for skill discovery
- versioned skill metadata

## Non-Goals

- Autonomous coding swarms.
- Automatic merge systems.
- Persistent background workers.
- Broad filesystem mutation.
- Project-specific hardcoding.
- Complex UI.
- Database storage before files become inadequate.
