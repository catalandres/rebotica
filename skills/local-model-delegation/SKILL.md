# Local Model Delegation

Use this skill when Prime wants help from local models through Rebotica for bounded review, explanation, test proposal, documentation cleanup, or small patch drafting.

Rebotica is advisory by default. Local workers do not own architecture, commits, pushes, merges, or final acceptance.

## Allowed Work

- Review a selected git diff.
- Explain selected files.
- Propose missing tests.
- Draft small bounded patches.
- Identify documentation drift.
- Perform mechanical cleanup inside an explicit allowlist.

## Forbidden Unless Explicitly Approved

- Auth architecture.
- Authorization behavior.
- Security-sensitive code.
- Database migration semantics.
- Dependency additions.
- Large cross-cutting refactors.
- Generated files.
- Direct commits.
- Direct pushes.

## Workflow

1. Read `.rebotica.yml` or `.rebotica/project.yml`.
2. Define a narrow task.
3. Create or inspect the task envelope.
4. Validate allowed, forbidden, and sensitive files.
5. Invoke Rebotica with `rbtc`.
6. Treat local output as advisory.
7. Apply a proposed patch only after reviewing the diff.
8. Run project checks.
9. Summarize the result and record a retrospective if useful.

## Commands

Use:

```sh
rbtc review
rbtc review --base origin/main
rbtc explain path/to/file
rbtc tests path/to/file
rbtc patch .rebotica/tasks/task.yml --dry-run
rbtc guard-diff
rbtc guard-diff --base origin/main
```

If `rbtc` is not on `PATH`, use the harness wrapper path directly.

## Acceptance

Never accept local worker output solely because it was generated. Prime must review the content, verify scope, and run appropriate checks.
