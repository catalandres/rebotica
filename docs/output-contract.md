# Output Contract (v1)

This document specifies the wire format `rbtc` emits in machine-readable mode. Practical examples live in [usage.md](usage.md); the `error.code` table and per-code `details` shapes live in [exit-codes.md](exit-codes.md).

## Scope

The v1 envelope is what `rbtc` writes to stdout in JSON or quiet mode. Every migrated command emits one envelope per invocation, success or failure. The shape is stable within `v1`; additive fields do not bump the version, semantic changes do.

The commands currently on the v1 envelope are listed in the [`kind` taxonomy](#kind-taxonomy) below. `run review`, `run explain`, `run tests`, and `run patch` are not yet on the envelope and their output should not be parsed from another tool; see [Reserved kinds](#reserved-kinds).

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
| `score` | `rbtc score` | The run id, the recorded score, and where it was written. |
| `scorecards` | `rbtc scorecards` | Aggregate model scorecard summary. |
| `comment-card.new` | `rbtc comment-card new` | The created card's id and path. |
| `comment-card.list` | `rbtc comment-card list` | Pending, submitted, and dismissed cards. |
| `comment-card.show` | `rbtc comment-card show` | The card body and metadata. |
| `comment-card.dismiss` | `rbtc comment-card dismiss` | The dismissed card's id and new path. |
| `comment-card.consent` | `rbtc comment-card consent` | The consent state and target repo. |
| `comment-card.submit` | `rbtc comment-card submit` | Submission result, including the GitHub issue url. |
| `retro` | `rbtc retro` | The created retrospective path and source run id. |

For the canonical `data` field shape of each kind, the source struct in `crates/rebotica-cli/src/main.rs` is authoritative. A consumer that wants to be forward-compatible should read only the fields it needs and ignore unknown ones.

The literal kind `"error"` is also possible: it is used when an envelope must be emitted before a subcommand can be resolved (clap parse failures, missing-subcommand, top-level cancellation). When you see `kind: "error"`, branch on `error.code` only; `command` will be `"rbtc"` or the best-effort subcommand path, and `data` will be `{}`.

## Reserved kinds

The following are reserved by the v1 spec but not yet emitted. Do not parse anything from these commands today.

| `kind` | Command | Status |
| --- | --- | --- |
| `run.review` | `rbtc run review` | Awaiting epic [#5](https://github.com/catalandres/rebotica/issues/5). |
| `run.explain` | `rbtc run explain` | Awaiting epic #5. |
| `run.tests` | `rbtc run tests` | Awaiting epic #5. |
| `run.patch` | `rbtc run patch` | Awaiting epic #5. |
| `runs.list` | `rbtc runs list` | Awaiting issue #18. Command does not yet exist. |
| `runs.show` | `rbtc runs show` | Awaiting issue #18. Command does not yet exist. |
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
