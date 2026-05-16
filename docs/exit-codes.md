# Exit Codes

Rebotica JSON and quiet output include a stable `error.code` value when a migrated command fails. The process exit code is derived from that typed error code, not from matching error-message text.

Legacy commands that have not been migrated to envelopes yet keep their existing human-oriented behavior. This includes `run ...`.

## For Consumers

| `error.code` | Exit | Meaning | Consumer action |
| --- | ---: | --- | --- |
| `internal` | 1 | An uncategorized Rebotica failure. | Fail the run and surface the message for investigation. |
| `usage` | 2 | Invalid CLI usage, missing arguments, or rejected user input. | Ask the caller to fix the command or input. |
| `config` | 3 | Project configuration, local state, or expected file setup is invalid or missing. | Ask for project setup or configuration repair. |
| `provider_unavailable` | 10 | The configured provider cannot be reached or cannot supply the requested model listing. | Retry after provider startup, endpoint, or network checks. |
| `provider_server_error` | 11 | Provider was reached but returned a 5xx or an unparseable response. | Retry after backoff; surface provider details if it persists. |
| `provider_client_error` | 12 | Provider rejected the request with HTTP 4xx. | Do not retry unchanged; fix auth, model name, or malformed input. |
| `guard_rejected` | 20 | A policy or safety guard rejected the requested operation. | Do not retry unchanged; ask for review or narrower scope. |
| `output_invalid` | 21 | Worker output failed to validate against the mode's declared schema. | Ask for regeneration or schema-compliant worker output before consuming. |
| `over_limit` | 22 | The requested operation exceeds configured size or safety limits. | Ask for a smaller scope or explicit limit adjustment. |
| `cancelled` | 130 | The command was interrupted by cancellation, such as Ctrl-C. | Treat as user-initiated cancellation, not a system failure. |

## Error Details

Envelope failures may include `error.details` with code-specific context. Consumers should branch on `error.code` first, then read the relevant fields.

`provider_unavailable`:

```json
{ "endpoint": "models", "reason": "connection refused" }
```

`provider_server_error` with HTTP status:

```json
{ "endpoint": "models", "http_status": 503, "body": "service overloaded" }
```

`provider_server_error` with invalid response:

```json
{ "endpoint": "chat", "parse_error": "expected value at line 1 column 1" }
```

`provider_client_error`:

```json
{ "endpoint": "models", "http_status": 401, "body": "missing api key" }
```

`guard_rejected`:

```json
{ "rejected_paths": ["secrets/key.txt"], "forbidden_pattern": "secrets/" }
```

`over_limit`:

```json
{ "kind": "lines", "limit": 1000, "actual": 1542 }
```

## For Contributors

Use `rebotica_core::output::CodedCommandError` when a command can classify a failure. Use `ErrorCode::exit_code()` for process exits and `ErrorCode::all()` for generated help, manifests, or documentation tables.

Do not classify errors by matching message strings. Preserve existing `CodedCommandError` values when adding outer context so a specific producer code is not collapsed into a generic wrapper code.

The full taxonomy is intentionally available before every producer is migrated. Future command migrations should attach these existing codes rather than adding new variants unless the consumer behavior truly needs a new class.
