# Release

Rebotica's first public distribution should stay boring: a tagged source release, a source-building Homebrew tap formula, and a local install smoke harness that proves the installed shim can find its runtime assets.

## Unreleased Notes

- JSON-mode state commands now emit the v1 Rebotica envelope; consumers should read `rebotica`, `kind`, `ok`, `data`, and `error` from the envelope.
- `--json` is now a global flag, so existing placements such as `rbtc doctor --json` continue to parse while `rbtc --json doctor` is also accepted. `--quiet` and `REBOTICA_QUIET` imply JSON mode for envelope output.
- `--help` and `--version` always print human-readable text and exit 0, even with `--json` or `--quiet` set. These are not errors and do not produce envelopes.
- Built-in `run review`, `run explain`, `run tests`, and `run patch` now route through the `run.*` plugin engine and emit schema-validated v1 envelopes.
- **Behavior change:** `rbtc run review --model X --model Y` (multi-model side-by-side invocation) is no longer supported. The v1 envelope contract is one envelope per invocation, which can't carry N model responses without a redesign that would significantly complicate the contract for a power-user feature. Run models separately via a shell loop: `for m in X Y; do rbtc run review --model $m --json; done`.
- `rbtc score RUN_ID` now accepts `--disposition <accept|reject|edit_then_use|unscored>` and writes the disposition back to the per-run `scorecard.yml`. The legacy `--accepted` / `--rejected` shorthands still work and map to `accept` / `reject`.
- The built-in `prompts/runs.d/<mode>/prompt.md` example block is now drift-checked by `cargo test`: a regression that breaks any built-in prompt's example against its schema fails the release gate. New built-in modes must include a fenced ` ```json ` example block whose payload validates against the mode's `schema.json`.
- **Apprentice ledger** at `~/.rebotica/ledger.db` (SQLite, bundled). `rbtc run *` writes `run_started` and `run_completed` events; `rbtc score` writes `prime_disposition`. The schema, payload shapes, and derived views (`v_per_model_stats`, `v_per_envelope_stats`, `v_disposition_breakdown`) are documented in [output-contract.md](output-contract.md#apprentice-ledger). Ledger writes are best-effort: a write failure logs a warning but does not abort the command. The `hallucination_rate` field on `run_completed` is populated by issue #51 in a follow-up.
- **`rbtc mcp serve`** — new subcommand that exposes apprentice tools over Model Context Protocol (stdio transport) for Prime agents. Tools: `review_diff`, `propose_tests`, `explain_files`, `health_check`. Each tool routes through the same `rebotica-run::dispatch` engine the CLI uses, so ledger persistence and schema validation are unchanged. The `command` field of envelopes persisted from MCP-initiated runs is `mcp.<tool>` (e.g. `mcp.review_diff`) so consumers can distinguish CLI-initiated from MCP-initiated runs.
- **`local-model-delegation` skill rewritten for MCP-first invocation.** Installed by `rbtc install claude` (into `.claude/skills/`) and `rbtc install codex` (into `.agents/skills/`). Prime is instructed to call `mcp__rebotica__review_diff` / `propose_tests` / `explain_files` / `health_check` before doing the equivalent work itself; the existing `rbtc run *` CLI verbs remain as a documented fallback. The simplest setup is `claude mcp add --scope project rebotica rbtc -- mcp serve` (writes to `.mcp.json` at the project root). The skill also ships a `hooks/claude-settings-snippet.json` users merge into `.claude/settings.json` to wire the included `hooks/post-tool-use.sh`, which records a placeholder `unscored` disposition after every `mcp__rebotica__*` tool call (Prime can later upgrade with explicit `rbtc score RUN_ID --disposition <value>`). The hook requires `jq` on PATH and soft-fails if anything is missing.
- **`scripts/mcp-eval.sh`** measures whether Claude Code invokes the right Rebotica MCP tool unprompted. Fires three seeded prompts × three runs against `claude --print --output-format stream-json --mcp-config ...` and reports `RESULT: N/9 passed`. Per Success Criterion 1 of the v0.3 milestone: ≥7/9 ships as planned, 5–6/9 ships with a known-issue note (tool-description iteration becomes the v0.3.x headline), ≤4/9 blocks the tag. The MCP server defaults to `REBOTICA_MCP_OFFLINE_PROBE=1` mode under the eval, so no provider tokens are spent on the apprentice side (Prime-side Claude tokens are still spent — one short prompt per session). Requires `claude`, `rbtc`, `jq`, and an `ANTHROPIC_API_KEY`. Tune via `REBOTICA_EVAL_MODEL`, `REBOTICA_EVAL_RUNS`, `REBOTICA_EVAL_LIVE`.
- **`REBOTICA_MCP_OFFLINE_PROBE`** environment variable: when set, the `rbtc mcp serve` tool handlers return a canned stub (with a fresh `run_id`) instead of calling the dispatch engine or the provider. Intended only for telemetry harnesses like `scripts/mcp-eval.sh`; do not set in production.
- **`rbtc runs list`** and **`rbtc runs show RUN_ID`** — new commands that query the apprentice ledger and render the per-run apprentice card. `list` supports `--limit`, `--kind`, `--model` filters; `show` renders a terminal-pretty card in human mode and a `runs.show` envelope in JSON mode with fields `apprentice_model`, `confidence`, `useful_finding`, `rejected_claim`, `recommended_next`. Pre-ledger runs (created before #44 landed) degrade gracefully with `source: "pre-ledger"`; the rejected-claim slot is populated by a placeholder until the hallucination-rate writer (#51) lands.

### Envelope contract

All state and diagnostic commands now emit the v1 envelope contract in JSON mode:

- `doctor`
- `providers`
- `models`
- `models configure`
- `init`
- `install`
- `skills list`
- `skills show`
- `score`
- `scorecards`
- `comment-card *`
- `retro`
- `health`
- `smoke`
- `guard-diff`
- `run review`
- `run explain`
- `run tests`
- `run patch`

The exit-code taxonomy is final and documented in [exit-codes.md](exit-codes.md).

## Local Install Harness

Run:

```sh
just install-smoke
```

This installs `rbtc` into `target/local-install/prefix`, creates a sandbox project under `target/local-install/`, and verifies:

- `rbtc --version`
- `rbtc help`
- `rbtc init`
- `rbtc providers --json`
- `rbtc models --configured-only`
- `rbtc doctor --json`
- `rbtc install codex --copy`
- `rbtc install claude --copy`
- `rbtc install github`

Use a custom prefix when needed:

```sh
just install-smoke /tmp/rebotica-prefix
```

The smoke harness does not require a running local model provider. Provider checks that hit `/models` remain a separate manual step.

## Release Gate

Before cutting a tag:

```sh
just release-check
```

Then run provider-backed checks against a real local provider:

```sh
rbtc health
rbtc smoke --model MODEL_ALIAS_OR_ID
```

Check the public conventions before release:

- CLI is `rbtc`.
- Version is `rbtc --version`, not a `version` subcommand.
- Config paths are `.rebotica.yml` or `.rebotica/project.yml`.
- Project state is `.rebotica/`.
- Private global state is `~/.rebotica/`.
- Environment variables use the `REBOTICA_` prefix.
- No public docs or prompts reintroduce old names.

## Tag Checklist

1. Confirm the working tree only contains intentional release changes.
2. Run `just release-check`.
3. Run provider-backed `rbtc health` and `rbtc smoke`.
4. Update release notes with CLI, config, prompt, safety, and migration changes.
5. Create an annotated tag, for example `v0.1.0`.
6. Push the tag.
7. Create a GitHub release from the tag.
8. Record the source tarball SHA-256 for the Homebrew formula.

## Homebrew Strategy

Start with a general-purpose personal tap:

```sh
catalandres/homebrew-tap
```

Users would install with:

```sh
brew install catalandres/tap/rebotica
```

Manual tap installation is also fine:

```sh
brew tap catalandres/tap
brew install rebotica
```

The formula should build from the tagged source archive, install the binary and runtime assets under `libexec`, and write a `bin/rbtc` shim that sets `REBOTICA_HOME` to that `libexec` directory.

Use [packaging/homebrew/rebotica.rb.template](../packaging/homebrew/rebotica.rb.template) as the starting point.

Formula update flow:

```sh
VERSION=v0.1.0
curl -L -o rebotica-$VERSION.tar.gz \
  https://github.com/catalandres/rebotica/archive/refs/tags/$VERSION.tar.gz
shasum -a 256 rebotica-$VERSION.tar.gz
```

Then replace the formula `url` and `sha256`, and test locally:

```sh
brew install --build-from-source --verbose ./Formula/rebotica.rb
brew test rebotica
```

After the tap is published, test the user-facing path:

```sh
brew install --build-from-source --verbose catalandres/tap/rebotica
brew test catalandres/tap/rebotica
```

## Bottles Later

Do not start with bottles. First prove:

- tagged source releases are repeatable
- the shim reliably sets `REBOTICA_HOME`
- runtime assets remain stable
- `rbtc install claude|codex|github` works from the brewed package

Once those are stable, add tap CI and bottles.
