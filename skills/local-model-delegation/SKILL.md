# Local Model Delegation

Use this skill when the root coordinator wants help from local models through Atelier for bounded review, explanation, test proposal, documentation cleanup, or small patch drafting.

Atelier is advisory by default. Local workers do not own architecture, commits, pushes, merges, or final acceptance.

## Allowed Work

- Review current diff.
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

1. Read `.atelier.yml` or `.atelier/project.yml`.
2. Define a narrow task.
3. Create or inspect the task envelope.
4. Validate allowed, forbidden, and sensitive files.
5. Invoke Atelier with `atelier`.
6. Treat local output as advisory.
7. Apply a proposed patch only after reviewing the diff.
8. Run project checks.
9. Summarize the result and record a retrospective if useful.

## Commands

Use:

```sh
atelier review
atelier explain path/to/file
atelier tests path/to/file
atelier patch .atelier/tasks/task.yml --dry-run
atelier guard-diff
```

If `atelier` is not on `PATH`, use the harness wrapper path directly.

## Acceptance

Never accept local worker output solely because it was generated. The root coordinator must review the content, verify scope, and run appropriate checks.
