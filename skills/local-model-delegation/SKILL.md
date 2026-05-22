---
name: local-model-delegation
description: Use this skill whenever the user asks for a code review of a diff, a test proposal, or an explanation of unfamiliar files. Calls Rebotica's local apprentice via MCP so the local model does the grunt work and Prime keeps judgment.
---

# Local Model Delegation

You have access to a local-model apprentice via the Rebotica MCP server. Use it before you do the work yourself on any of these tasks:

- **Code review of a diff** → call `mcp__rebotica__review_diff` first.
- **Proposing missing tests for files** → call `mcp__rebotica__propose_tests` first.
- **Explaining unfamiliar files** → call `mcp__rebotica__explain_files` first.
- **Diagnosing why delegated calls are failing** → call `mcp__rebotica__health_check`.
- **Rebotica itself fell short** (bad apprentice output, confusing result, missing capability) → call `mcp__rebotica__submit_feedback` instead of silently working around it.

The apprentice is *advisory*. You retain full judgment: incorporate, override, or reject its findings. The point is to delegate the obvious so you spend cycles on the calls that need you.

## When to call which tool

| User intent (paraphrase)                                | Tool                                  | What it returns                                                              |
| ------------------------------------------------------- | ------------------------------------- | ---------------------------------------------------------------------------- |
| "Review my changes" / "what's wrong with this diff?"    | `mcp__rebotica__review_diff`          | `findings[]` with severity, file, line, summary, fix; plus a confidence 0-10 |
| "What tests am I missing?" / "test this file"           | `mcp__rebotica__propose_tests`        | `proposed_tests[]` with name, scenario, kind                                 |
| "Explain this file" / "what does this do?"              | `mcp__rebotica__explain_files`        | `analysis` (string) covering responsibilities, deps, risks                   |
| "Is the local model working?" / "why is rbtc failing?"  | `mcp__rebotica__health_check`         | `{ provider, base_url, ok, model_count, models }`                            |
| Rebotica produced wrong output / is confusing / lacks X | `mcp__rebotica__submit_feedback`      | `{ card_id, status }`; pass `run_id` when it's about a specific run          |

For `review_diff`, the `source` parameter selects the diff:
- `"working-tree"` (default) — unstaged changes vs HEAD
- `"staged"` — index vs HEAD
- `"range:BASE..HEAD"` — explicit ref range

## After acting on apprentice output

Record what you did with the apprentice's output so it can learn from real use:

```sh
rbtc score RUN_ID --disposition <accept|reject|edit_then_use>
```

Use:
- `accept` — you used the output substantively, no edits.
- `edit_then_use` — you used it with edits.
- `reject` — you discarded it (hallucination, off-topic, low quality).

The `RUN_ID` is the `run_id` field in the apprentice's tool response. If a `PostToolUse` hook is configured (see [hooks/](hooks/)), a placeholder `unscored` row is written automatically and you can upgrade it later with the explicit disposition.

## What the apprentice should NOT be used for

Even when these are technically in-scope for one of the tools above, do not delegate without explicit user instruction:

- Auth architecture and authorization behavior.
- Security-sensitive code paths (secrets handling, crypto, sandboxing).
- Database migration semantics.
- Dependency additions or version bumps.
- Large cross-cutting refactors.
- Generated files.
- Anything that would result in a commit, push, or merge.

The apprentice never commits, pushes, or merges. It returns advisory JSON. You apply.

## CLI fallback

If MCP tool calls fail (e.g., MCP server not configured or unreachable), the same modes are available via the CLI:

```sh
rbtc run review                    # working tree diff
rbtc run review --cached           # staged diff
rbtc run review --base origin/main # range
rbtc run explain path/to/file
rbtc run tests path/to/file
rbtc health
```

CLI output is verbose; the MCP variant returns clean JSON.

## Setup (one-time)

For Claude Code to discover the MCP server, register it once per project. Easiest:

```sh
claude mcp add --scope project rebotica rbtc -- mcp serve
```

That writes the entry to `.mcp.json` at the project root. To edit by hand instead, see [claude-settings-snippet.json](claude-settings-snippet.json) for the exact JSON.

Restart any existing Claude Code session in the project for the registration to take effect. Verify with `/mcp` — `rebotica` should appear with the five tools.

For Codex, MCP discovery is native once `rbtc mcp serve` is invokable; no equivalent file to edit.

## Codex parity

This skill works identically under Codex when installed via `rbtc install codex` to `.agents/skills/local-model-delegation/`. Codex's MCP support is native: no settings.json equivalent is needed. The same five `mcp__rebotica__*` tool names should appear automatically.
