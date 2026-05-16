# Exit Codes

Rebotica JSON and quiet output include a stable `error.code` value when a migrated command fails. The process exit code is derived from that typed error code, not from matching error-message text.

Legacy commands that have not been migrated to envelopes yet keep their existing human-oriented behavior. This includes `health`, `smoke`, `guard-diff`, and `run ...`.

## For Consumers

| `error.code` | Exit | Meaning | Consumer action |
| --- | ---: | --- | --- |
| `internal` | 1 | An uncategorized Rebotica failure. | Fail the run and surface the message for investigation. |
| `usage` | 2 | Invalid CLI usage, missing arguments, or rejected user input. | Ask the caller to fix the command or input. |
| `config` | 3 | Project configuration, local state, or expected file setup is invalid or missing. | Ask for project setup or configuration repair. |
| `provider_unavailable` | 10 | The configured provider cannot be reached or cannot supply the requested model listing. | Retry after provider startup, endpoint, or network checks. |
| `provider_error` | 11 | The provider was reached but returned an unusable response for the requested operation. | Retry if transient; otherwise surface provider details. |
| `guard_rejected` | 20 | A policy or safety guard rejected the requested operation. | Do not retry unchanged; ask for review or narrower scope. |
| `patch_invalid` | 21 | A patch or patch-like output is malformed or cannot be accepted. | Ask for regeneration or manual repair before applying. |
| `over_limit` | 22 | The requested operation exceeds configured size or safety limits. | Ask for a smaller scope or explicit limit adjustment. |
| `cancelled` | 130 | The command was interrupted by cancellation, such as Ctrl-C. | Treat as user-initiated cancellation, not a system failure. |

## For Contributors

Use `rebotica_core::output::CodedCommandError` when a command can classify a failure. Use `ErrorCode::exit_code()` for process exits and `ErrorCode::all()` for generated help, manifests, or documentation tables.

Do not classify errors by matching message strings. Preserve existing `CodedCommandError` values when adding outer context so a specific producer code is not collapsed into a generic wrapper code.

The full taxonomy is intentionally available before every producer is migrated. Future command migrations should attach these existing codes rather than adding new variants unless the consumer behavior truly needs a new class.
