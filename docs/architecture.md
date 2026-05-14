# Architecture

Rebotica has three layers:

```text
root coordinator
  -> task envelope and policy
  -> local worker interface
  -> OpenAI-compatible endpoint
  -> advisory output or proposed diff
  -> root review and verification
```

## Rust Workspace

The Rust implementation lives in `crates/`.

Important boundaries:

- `rebotica-core`: config schema, task envelopes, model routing, and shared policy types.
- `rebotica-provider`: OpenAI-compatible HTTP calls, provider auth, and provider selection.
- `rebotica-git`: read-only git context and diff metrics.
- `rebotica-guard`: forbidden path and diff-size checks.
- `rebotica-runlog`: private run storage and scorecard bootstrap.
- `rebotica-cli`: user-facing command behavior.
- `rebotica-mcp`: future narrow MCP server.
- `skills/`: canonical skills that can be installed into root-agent adapters.

## Shell Bridge First

The first usable interface is `bin/rbtc`. It keeps the tool easy to call from Claude Code, terminals, and future roots.

## Provider And Model Routing

Rebotica supports simple aliases in `.rebotica.yml`.

Provider names keep URLs and auth details out of commands:

```yaml
providers:
  default: lmstudio
  lmstudio:
    kind: openai-compatible
    base_url: http://127.0.0.1:1234/v1
```

Model aliases make long local model ids stable and readable:

```yaml
models:
  default: local-coder
  review: local-coder
  aliases:
    local-coder: actual-model-id
```

This is intentionally a narrow provider framework. The contract is OpenAI-compatible `/models` and `/chat/completions`; provider-specific behavior should stay behind explicit config until a real need appears.

MCP comes later, after the shell bridge proves which tools are actually worth exposing.

## Narrow Future MCP Tools

The MCP server should expose only bounded tools:

- `local_model.review_diff`
- `local_model.explain_files`
- `local_model.propose_tests`
- `local_model.propose_patch`
- `local_model.health_check`
- `local_model.score_last_run`

It should not expose broad shell or filesystem mutation tools.
