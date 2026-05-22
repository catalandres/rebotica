# Output Contract (v1)

This document specifies the wire format `rbtc` emits in machine-readable mode. Practical examples live in [usage.md](usage.md); the `error.code` table and per-code `details` shapes live in [exit-codes.md](exit-codes.md).

## Scope

The v1 envelope is what `rbtc` writes to stdout in JSON or quiet mode. Every migrated command emits one envelope per invocation, success or failure. The shape is stable within `v1`; additive fields do not bump the version, semantic changes do.

The commands currently on the v1 envelope are listed in the [`kind` taxonomy](#kind-taxonomy) below.

## Channels

| Channel | Human mode | `--json` mode | `--quiet` mode |
| --- | --- | --- | --- |
| stdout | Rendered result (text, table, diff). No progress chatter. | One v1 envelope, pretty-printed. | One v1 envelope, pretty-printed. |
| stderr | Progress, prompts, warnings, prose preambles. | Progress, prompts, warnings, prose preambles. | Silent unless something failed before the envelope could be assembled. |

The discipline holds regardless of command. Diagnostic text never appears on stdout.

## Modes

- **Human** is the default. Stdout gets a human-rendered result; stderr gets progress and prompts.
- **`--json`** emits the v1 envelope on stdout. Prose still allowed on stderr.
- **`--quiet`** implies `--json` and suppresses non-error stderr. A `--quiet` invocation produces exactly one JSON envelope on stdout, and nothing on stderr, unless something failed before the envelope could be assembled (e.g. a panic before reporter setup).

`--help` and `--version` always print human text and exit `0`, regardless of mode. They are not errors and do not produce envelopes.

## Flags and environment

`--json` and `--quiet` are global flags on the root `Cli`. They are accepted before or after the subcommand:

```sh
rbtc --json doctor
rbtc doctor --json
```

Environment overrides:

| Variable | Equivalent to |
| --- | --- |
| `REBOTICA_JSON=1` | `--json` |
| `REBOTICA_QUIET=1` | `--quiet` (implies `--json`) |

Truthy values are `1`, `true`, `yes` (case-insensitive). Anything else is treated as unset. Useful for adapters that wrap `rbtc` without rewriting argv.

## Envelope shape

```json
{
  "rebotica": "v1",
  "kind": "doctor",
  "ok": true,
  "command": "doctor",
  "data": { },
  "error": null,
  "run_id": null,
  "started_at": "2026-05-15T22:00:00Z",
  "duration_ms": 42
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `rebotica` | string | Schema version. Currently `"v1"`. Additive changes do not bump it; renames, removals, or semantic changes do. |
| `kind` | string | Discriminator for `data`. Stable per command. See [`kind` taxonomy](#kind-taxonomy). |
| `ok` | bool | `true` iff `error` is `null`. Convenience for consumers that do not introspect `error`. |
| `command` | string | The resolved CLI subcommand path, space-separated. Mirrors what a human would type. |
| `data` | object | Payload object, shape determined by `kind`. `{}` for commands with no payload. |
| `error` | object\|null | `null` on success; an [error object](#error-variant) on failure. |
| `run_id` | string\|null | Set for commands that persist a run under `~/.rebotica/runs`. `null` otherwise. |
| `started_at` | string | RFC3339 UTC timestamp of command start. |
| `duration_ms` | integer | Wall-clock duration from start to envelope emission. |

JSON is pretty-printed. `jq` handles either form; pretty reads better when a human runs `rbtc ... --json` directly.

## Error variant

```json
{
  "code": "guard_rejected",
  "message": "patch touches forbidden path: secrets/key.txt",
  "details": {
    "rejected_paths": ["secrets/key.txt"],
    "forbidden_pattern": "secrets/"
  }
}
```

| Field | Type | Notes |
| --- | --- | --- |
| `code` | string | Symbolic name from the [`error.code` table](exit-codes.md#for-consumers). Snake-case. Drives the process exit code. |
| `message` | string | Single human-readable sentence. No trailing newline. |
| `details` | object\|absent | Code-specific structured context. Omitted when there is none. Per-code shapes are documented in [exit-codes.md](exit-codes.md#error-details). |

When a command fails, `ok` is `false`, `data` may be partial or absent (still an object), the envelope is still written to stdout, and the process exits with the code mapped from `error.code`.

## `kind` taxonomy

`kind` is the discriminator. Top-level commands use a single token (`doctor`); subcommands use a dotted form (`comment-card.new`).

These kinds are emitted today:

| `kind` | Command | `data` summary |
| --- | --- | --- |
| `doctor` | `rbtc doctor` | Resolved config, provider state, git state, installed adapters. |
| `providers` | `rbtc providers` | Configured providers, endpoints, auth env state. |
| `models` | `rbtc models` | Configured model routes; provider model list when reachable. |
| `models.configure` | `rbtc models configure` | Alias and route entries written or proposed. |
| `health` | `rbtc health` | `{ provider, base_url, model_count, models }`. |
| `smoke` | `rbtc smoke` | `{ provider, base_url, model, probe_prompt, response }`. |
| `init` | `rbtc init` | Files created and any next-step hints. |
| `install` | `rbtc install` | Adapter assets symlinked or copied. |
| `skills.list` | `rbtc skills list` | Canonical and project-local skills with source attribution. |
| `skills.show` | `rbtc skills show` | The selected skill's metadata and body. |
| `guard-diff` | `rbtc guard-diff` | `{ diff_source, changed_files, changed_lines, max_files, max_lines, effective_forbidden_paths }`. |
| `score` | `rbtc score` | The run id, the recorded disposition (`accept`, `reject`, `edit_then_use`, `unscored`), optional rating, labels, notes, and the on-disk paths the score event wrote to. |
| `scorecards` | `rbtc scorecards` | Aggregate model scorecard summary. |
| `comment-card.new` | `rbtc comment-card new` | The created card's id and path. |
| `comment-card.list` | `rbtc comment-card list` | Pending, submitted, and dismissed cards. |
| `comment-card.show` | `rbtc comment-card show` | The card body and metadata. |
| `comment-card.dismiss` | `rbtc comment-card dismiss` | The dismissed card's id and new path. |
| `comment-card.consent` | `rbtc comment-card consent` | The consent state and target repo. |
| `comment-card.submit` | `rbtc comment-card submit` | Submission result, including the GitHub issue url. |
| `retro` | `rbtc retro` | The created retrospective path and source run id. |
| `runs.list` | `rbtc runs list` | Recent runs from the ledger with `run_id`, `kind`, `model`, `started_at`, `ok`, `disposition`. |
| `runs.show` | `rbtc runs show` | Apprentice card for a single run: `apprentice_model`, `confidence`, `useful_finding`, `rejected_claim`, `recommended_next`, plus paths to persisted artifacts. |
| `run.review` | `rbtc run review` | Schema-validated review findings for a selected diff. |
| `run.explain` | `rbtc run explain` | Schema-validated analysis for selected files. |
| `run.tests` | `rbtc run tests` | Schema-validated proposed tests for selected files. |
| `run.patch` | `rbtc run patch` | Schema-validated patch proposal and touched files. |
| `compare.<mode>` | `rbtc compare <mode> --model A --model B` | Per-model summary rows aggregating an N-way run against the same input. See [Comparison runs](#comparison-runs). |

For the canonical `data` field shape of each kind, the source struct in `crates/rebotica-cli/src/main.rs` is authoritative. A consumer that wants to be forward-compatible should read only the fields it needs and ignore unknown ones.

The literal kind `"error"` is also possible: it is used when an envelope must be emitted before a subcommand can be resolved (clap parse failures, missing-subcommand, top-level cancellation). When you see `kind: "error"`, branch on `error.code` only; `command` will be `"rbtc"` or the best-effort subcommand path, and `data` will be `{}`.

## Persisted Run Artifacts

Model-backed `run.*` invocations allocate a `run_id` and create `~/.rebotica/runs/{run_id}/` before the provider chat request. The stdout envelope is the wire contract; the run directory is the audit trail.

Success writes:

- `model-response.md`: raw provider text, verbatim.
- `parsed-output.json`: the validated envelope `data`.
- `envelope.json`: the full emitted v1 envelope.

When the provider returns text but JSON extraction or schema validation fails, Rebotica writes:

- `model-response.md`: raw provider text, verbatim.
- `parse-failure.json`: the structured `error.details` object.
- `envelope.json`: the full failure envelope.

When the provider call fails before returning a response, Rebotica writes:

- `provider-failure.json`: typed provider diagnostic details.
- `envelope.json`: the full failure envelope.

`model-response.md` is present if and only if the provider returned a response body. Its absence means the model was never reached or returned no response body to persist.

Failure envelopes from after run allocation, including `output_invalid` and provider failures, carry the same non-null `run_id` so consumers can locate the persisted artifacts. Failures before run allocation, such as unknown modes or guard rejection, keep `run_id: null`.

### Scorecard write-back

Every run also gets a `scorecard.yml` written at allocation time with `disposition: unscored`. `rbtc score RUN_ID --disposition <accept|reject|edit_then_use|unscored>` updates that file in place and additionally writes:

- `feedback.yml` — the per-call score event with disposition, optional rating, labels, and notes.
- An appended row in `~/.rebotica/model-events.jsonl`.
- A rebuilt `~/.rebotica/model-scorecards.yml` aggregate.

The `--disposition` flag takes precedence over the legacy `--accepted` / `--rejected` shorthands; the shorthands continue to work and map to `accept` / `reject`.

## Apprentice Ledger

`~/.rebotica/ledger.duckdb` is a [DuckDB](https://duckdb.org) database that records the v0.3+ event log driving routing, retrospectives, and (eventually) the federated benchmark. It is created and migrated on first event write; consumers can open it read-only with the `duckdb` CLI. A ledger created before the v0.3 DuckDB switch (`~/.rebotica/ledger.db`, SQLite) is migrated forward automatically on first open via DuckDB's `sqlite` extension, and the original is preserved as `ledger.db.sqlite-backup`.

### Schema

Single append-only table, with ids from a sequence:

```sql
CREATE SEQUENCE ledger_events_id_seq START 1;
CREATE TABLE ledger_events (
    id           BIGINT  PRIMARY KEY DEFAULT nextval('ledger_events_id_seq'),
    ts           VARCHAR NOT NULL,           -- RFC 3339 UTC
    run_id       VARCHAR,                    -- nullable; null for non-run events
    event_type   VARCHAR NOT NULL,           -- run_started | run_completed | prime_disposition | score_recorded | run_rejected
    payload_json VARCHAR NOT NULL            -- typed payload, shape per event_type
);
```

The schema version lives in a `ledger_meta(key, value)` row (`key = 'schema_version'`), `1` for the v0.3 baseline. Future schema bumps land additive migrations and increment this number. Schema changes never alter existing rows. (Concurrency note: DuckDB takes an exclusive lock per read-write connection, so rbtc opens the ledger in short write-then-close bursts and retries with backoff on transient lock contention.)

### MCP-initiated runs

When the apprentice is invoked over MCP (via `rbtc mcp serve`), the `command` field on persisted envelopes is `mcp.<tool>` rather than `run <mode>`. Mapping:

| MCP tool         | `run.*` mode | `command` on persisted envelope | `envelope_shape` on ledger |
| ---------------- | ------------ | ------------------------------- | -------------------------- |
| `review_diff`     | `review`     | `mcp.review_diff`               | `run_review`               |
| `propose_tests`   | `tests`      | `mcp.propose_tests`             | `run_tests`                |
| `explain_files`   | `explain`    | `mcp.explain_files`             | `run_explain`              |
| `health_check`    | (no run)     | (no persisted envelope)         | (no ledger event)          |
| `submit_feedback` | (no run)     | (no persisted envelope)         | (no ledger event)          |

`health_check` does not invoke a model; it pings the configured provider's `/models` endpoint and returns a short status object. It does not persist a run directory or write to the ledger.

`submit_feedback` does not invoke a model either; it writes a product-feedback comment card under `~/.rebotica/comment-cards/pending/` and, when GitHub submission consent is enabled (`rbtc comment-card consent --allow-github`), files a labeled GitHub issue via `gh`. Returns the `card_id` and whether it was submitted. It shares the same core as the `rbtc comment-card` CLI verbs.

The ledger's `EnvelopeShape` enum also reserves `review_diff`, `propose_tests`, `explain_files`, and `health_check` variants for future MCP-specific instrumentation. The four `run_*` shapes are what's used today.

### Event payload shapes (v0.3)

`run_started`:

```json
{
  "kind": "run.review",
  "envelope_shape": "run_review",
  "model": "qwen-coder-32b",
  "provider": "lmstudio",
  "contract_version": 1
}
```

`run_completed`:

```json
{
  "kind": "run.review",
  "envelope_shape": "run_review",
  "model": "qwen-coder-32b",
  "ok": true,
  "error_code": null,
  "duration_ms": 4321,
  "output_bytes": 1024,
  "hallucination_rate": 0.2,
  "confidence": 7,
  "apprentice_prompt_tokens": 3421,
  "apprentice_completion_tokens": 612,
  "envelope_bytes": 1187
}
```

`error_code` (snake_case `ErrorCode` name) is populated when `ok` is `false`.

`hallucination_rate` is the **structural** review hallucination rate (issue #51): the fraction of `findings[]` whose citations don't ground against the reviewed code. A finding is *ungrounded* when its `file` is neither in the diff's changed set nor present in the working tree, or when its `line` falls outside that file (lines are 1-indexed). The denominator is `findings.len()`. A finding with no `file` citation is treated as grounded (the rate measures *false* citations, not missing ones). It is:

- a number in `[0.0, 1.0]` for successful `run.review` runs that returned at least one finding;
- `null` for findings-free reviews (no denominator), for freeform (`--no-schema`) runs, for non-review modes (`tests` / `explain` / `patch`), and for any failed run.

This is the structural half of the design definition; semantic claim-grounding (behaviour asserted but not supported by the diff hunks) is deferred to a later iteration. Aggregate it via `v_per_model_stats.avg_hallucination_rate`.

`apprentice_prompt_tokens` and `apprentice_completion_tokens` come from the provider's `usage` block (LM Studio reports them; some upstream proxies strip them). Both are omitted when the provider does not report `usage`, or when the run failed before any provider response arrived. `envelope_bytes` is the byte length of the serialized `data` field returned to Prime — a proxy for Prime's roundtrip context cost (≈ bytes/4 tokens). It is populated whenever a parsed envelope was produced, including the success path and post-validation failure paths; it is `null` only on pre-chat failures. Together, these three fields are the minimum needed to compute net-token-saved estimates against the apprentice corpus; see [measurement.md](measurement.md) for the formula and its limits.

`prime_disposition`:

```json
{
  "disposition": "accept",
  "rating": 4,
  "labels": ["useful_finding"],
  "notes": "Tweaked one wording."
}
```

`score_recorded`:

```json
{ "axis": "diff_review", "score": 4 }
```

`run_rejected` (added in #59) — emitted when a dispatch attempt fails before
allocating a persisted run (e.g. `over_limit`, `guard_rejected`, missing model,
config errors). It has no matching `run_started` / `run_completed` pair, but it
does carry a `run_id` so callers can pivot to `rbtc runs show <id>`:

```json
{
  "kind": "run.review",
  "error_code": "over_limit",
  "message": "diff exceeds 50000 lines",
  "details": { "lines": 60000 }
}
```

`kind` falls back to `"run"` when the rejection happened before plugin
resolution. `error_code` is the snake_case `ErrorCode` name. `details` is
optional and shape varies by failure mode.

### Derived views

The schema ships three SQL views that aggregate `ledger_events` for ad-hoc inspection and for `rbtc runs show` (#18):

- `v_per_model_stats` — per `(model, envelope_shape)`: `completed_runs`, `ok_runs`, `avg_confidence`, `avg_hallucination_rate`, `latest_ts`.
- `v_per_envelope_stats` — per `envelope_shape`: `completed_runs`, `ok_runs`, `avg_confidence`, `avg_hallucination_rate`.
- `v_disposition_breakdown` — per `disposition`: `rows_count`.

These are SQL views, not materialized tables; they always reflect the current contents of `ledger_events`.

## Comparison runs

`rbtc compare <mode> --model A --model B [-- adapter args]` dispatches the
same `run.<mode>` work against each model sequentially and emits a single
`compare.<mode>` envelope that aggregates the per-model outcomes. Each model
call goes through the normal `dispatch` path, so each one persists its own
run directory, ledger rows, and `run_id` — `compare` adds the aggregation on
top, it does not replace per-run persistence.

Default is sequential. Parallel execution (`--parallel`/`--jobs`) is the
follow-up tracked in [#4](https://github.com/catalandres/rebotica/issues/4).

`data` shape:

```json
{
  "mode": "review",
  "models": [
    {
      "model": "qwen3-coder-next",
      "run_id": "20260518-201500-abcd",
      "ok": true,
      "error_code": null,
      "confidence": 8,
      "n_findings": 3,
      "duration_ms": 12345
    },
    {
      "model": "gemma-4-26b-a4b",
      "run_id": "20260518-201600-efgh",
      "ok": false,
      "error_code": "output_invalid",
      "confidence": null,
      "n_findings": null,
      "duration_ms": 9876
    }
  ]
}
```

`n_findings` counts the natural list field for each mode (`findings` for
`review`, `proposed_tests` for `tests`) and is `null` for modes that have
no such list (`explain`, `patch`). `run_id` is `null` only if both
persistence and the rejection ledger write failed catastrophically.

`compare` does not auto-score the per-model runs. Prime calls
`rbtc score <run_id> --disposition ...` against each row, same as for
single-model runs.

## Reserved kinds

The following are reserved by the v1 spec but not yet emitted. Do not parse anything from these commands today.

| `kind` | Command | Status |
| --- | --- | --- |
| `capabilities` | `rbtc capabilities` | Awaiting issue #17. Command does not yet exist. |

## Failure-mode guarantees

- The envelope is emitted on stdout for every recognized invocation, success or failure. The exit code is the primary failure signal; the envelope is the structured one.
- `error.code` is set from a typed `ErrorCode` value at the producer site. It is not derived from matching message strings. Consumers should branch on `error.code`, never on `error.message`.
- `error.details` is populated for every code that documents a `details` shape in [exit-codes.md](exit-codes.md#error-details). Branch on `error.code` first, then read details.
- SIGINT yields a `cancelled` envelope (kind `"error"` if no subcommand was running) and exit code `130`.
- `--help` and `--version` bypass the envelope path entirely.

## `--quiet` guarantee

A `--quiet` invocation that reaches reporter setup writes exactly one envelope on stdout and nothing on stderr. The only way to see stderr text under `--quiet` is if `rbtc` failed before it could construct the reporter — for example, a hard panic in startup. Treat any stderr output under `--quiet` as a bug to file.

## Schema versioning

`v1` is the current schema. The contract is:

- Adding optional fields to the envelope, to a `data` payload, or to `error.details` is **additive** and does not change the version.
- Renaming a field, changing the meaning of an existing field, removing a field, or changing a type is **breaking** and requires `v2`.
- New `kind` values may be added under `v1`. Consumers should treat unknown kinds as opaque rather than failing hard.

Producers within the workspace should use `rebotica_core::output::Envelope` and the `Reporter` wrapper from the same module. Do not construct envelopes by hand or write to stdout directly outside of the reporter.
