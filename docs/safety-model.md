# Safety Model

Atelier relies on layered safety rather than model obedience alone.

## Scope Controls

- File allowlists.
- Forbidden paths.
- Sensitive path flags.
- Maximum changed lines.
- Maximum changed files.
- No dependency changes by default.
- No generated files by default.

## Git Controls

- Review and explain commands are read-only.
- Patch mode is dry-run-first.
- Worktree isolation is preferred for future patch workers.
- No auto-push.
- No auto-merge.
- No auto-commit by default.

## Verification Controls

The root coordinator runs configured project commands:

- tests
- checks
- lint
- format checks

The local worker must not claim those commands passed unless the harness actually ran them.

## Output Controls

Worker output must separate:

- analysis
- assumptions
- risks
- patch output
- commands actually run
- commands still needed

## Guardrails Today

The current CLI enforces forbidden path checks and diff-size checks through `atelier guard-diff`. Patch application is intentionally disabled in v0.1.
