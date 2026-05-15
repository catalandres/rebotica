# Philosophy

Rebotica is built around governed collaborative craftsmanship: capable tools working under explicit boundaries, with a human or Prime preserving judgment.

The harness does not try to make local models autonomous. It makes them useful by narrowing their work.

## Do Not Delegate Ambiguity

Local models can be effective reviewers, explainers, test drafters, and mechanical patch assistants. They are much less reliable when asked to decide what matters, where risk lives, or how much scope is appropriate.

Rebotica therefore treats every local invocation as a contract:

- What is the goal?
- Which files are allowed?
- Which files are forbidden?
- What output shape is acceptable?
- What risks must be surfaced?
- What acceptance gates remain outside the delegated run?

## Public Vocabulary

Use neutral delegation language in user-facing text:

- `Prime`: the coordinating agent or human-guided harness that owns judgment.
- `local model`: the model selected for a scoped task.
- `delegated run`: one Rebotica invocation with a task envelope, prompt contract, and run log.
- `scoped task`: constrained work with explicit allowed files, forbidden files, limits, and output shape.

Avoid public wording that implies hierarchy or autonomy, such as broad references to local workers, worker ownership, or autonomous agents. Internal file names and Rust identifiers may keep older terms when renaming would add more churn than clarity.

## Prime

Prime may be Claude Code, OpenCode, Hermes, or another future harness. Prime owns decomposition, scope, review, test execution, and acceptance.

This keeps Rebotica project-agnostic. The durable interface is the task envelope and local-model output contract, not any single Prime-agent product.

## Why Rust

Rebotica is local-first today: local providers such as LM Studio, local files, git worktrees, shell commands, and private run logs. Rust is a good fit for that center of gravity because it gives the project a portable native binary, strong typed policy boundaries, a mature CLI ecosystem, and a clean release story for coworkers.

The CLI remains plain and scriptable. Future MCP or editor integrations can call the same executable instead of reimplementing policy.
