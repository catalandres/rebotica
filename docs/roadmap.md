# Roadmap

Rebotica should grow through proof, not ambition.

## Phase 1: Shell Bridge

Current focus:

- Rust CLI.
- Provider health and smoke checks, with LM Studio as the default local provider.
- Review a selected git diff.
- Guard a selected git diff.
- Inspect and attach Prime-selected skills.
- Score model runs and queue Rebotica comment cards.
- Explain files.
- Propose tests.
- Dry-run patch proposal.
- Run logging.
- Claude commands and skills.

## Phase 2: Better Guardrails

Next:

- Stronger task-envelope validation.
- Better parsing of model output.
- Patch diff validation before display.
- Optional configured test/check execution.
- Project-level context packing rules.
- Provider fixture tests for auth, aliases, and error handling.

## Phase 3: Worktree Patch Drafting

Patch drafting should happen in isolated git worktrees before direct application is considered.

Expected pattern:

```sh
git worktree add ../project-rebotica-draft-1 -b ai/rebotica-draft-1 main
cd ../project-rebotica-draft-1
rbtc run patch .rebotica/tasks/task.yml --dry-run
```

Prime reviews the diff and runs checks.

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

Make canonical Rebotica skills installable across Prime-agent adapters:

- per-invocation skill selection first
- file-based Claude/Codex adapters first
- GitHub repository governance assets
- MCP resources for skill discovery
- versioned skill metadata

## Non-Goals

- Autonomous coding swarms.
- Automatic merge systems.
- Persistent background agents.
- Broad filesystem mutation.
- Project-specific hardcoding.
- Complex UI.
- Database storage before files become inadequate.
