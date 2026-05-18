# Authoring a `run.*` plugin

A `run.*` plugin is a directory containing a manifest, a prompt, and an output schema. Drop one in the right place and `rbtc run <mode>` works — no rebuild. This guide walks through building one end-to-end, then catalogs the references you'll need.

The audience is someone who already understands the v1 envelope contract (see [output-contract.md](output-contract.md)) and the exit-code taxonomy (see [exit-codes.md](exit-codes.md)).

## What a plugin is

A directory with three files (defaults shown):

```
my-mode/
  manifest.yml      # what this mode is, what inputs it consumes, what error codes it can produce
  prompt.md         # the system prompt the worker receives
  schema.json       # JSON Schema for the worker's output payload
```

When you run `rbtc run my-mode`, the engine:

1. Resolves the plugin from one of the registry layers (see below).
2. Parses `adapter_args` according to the inputs the manifest declares.
3. Renders the full prompt (project config + per-adapter blocks + `prompt.md`).
4. Sends it to your configured provider.
5. Extracts the JSON payload from the response.
6. Validates the payload against `schema.json` (which must extend the common schema).
7. Emits a v1 envelope of kind `run.my-mode` with `data` = the validated payload.

If anything fails, the envelope carries an `output_invalid` error with structured `error.details` and the raw response is persisted under `~/.rebotica/runs/{run_id}/model-response.md` so you can diagnose.

## The three-layer registry

Plugins are resolved from three directories, in precedence order:

| Layer | Path | When to use |
|-------|------|-------------|
| Project | `.rebotica/runs.d/<mode>/` | This repo only |
| User | `~/.rebotica/runs.d/<mode>/` | Cross-project, just you |
| Built-in | `prompts/runs.d/<mode>/` | Ships with the binary; covers `review`, `explain`, `tests`, `patch` |

A plugin at a higher-precedence layer shadows lower layers completely — no field-by-field merge. If a layer's directory exists but is incomplete (missing files, invalid manifest, schema doesn't extend the common base), the engine falls through to the next layer and emits a one-time stderr warning (suppressed under `--quiet`). `rbtc doctor` lists every broken layer with its reason.

## Walkthrough: a `commit-message` mode

We'll write a mode that reads a diff and proposes a commit message. The prompt should accept any standard `diff` adapter flags (`--base`, `--range`, `--cached`).

### Manifest

`./.rebotica/runs.d/commit-message/manifest.yml`:

```yaml
kind: run.commit-message
display_name: Commit Message
description: Propose a conventional commit message from a diff.
schema_version: 1
inputs:
  - diff
  - guard
exit_codes:
  - output_invalid
  - guard_rejected
  - over_limit
```

- `kind` must equal `run.<directory-name>`. Mismatch makes the layer broken.
- `display_name` and `description` show up in `rbtc run --help`.
- `schema_version` is yours to manage — bump it when you make breaking changes to `schema.json`.
- `inputs` is the ordered list of adapters this mode uses. See [Input adapters](#input-adapters).
- `exit_codes` is advisory for `#17` (capabilities), not enforced at runtime. Engine validates each entry against the known `ErrorCode` set, so typos surface at registry build.

### Schema

`./.rebotica/runs.d/commit-message/schema.json`:

```json
{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "allOf": [
    { "$ref": "https://rebotica/runs-common.schema.json" },
    {
      "type": "object",
      "required": ["subject", "type"],
      "properties": {
        "subject": {
          "type": "string",
          "minLength": 1,
          "maxLength": 72,
          "description": "First line of the commit message; conventional-commit format."
        },
        "type": {
          "enum": ["feat", "fix", "chore", "docs", "refactor", "test", "perf"],
          "description": "Conventional commit type prefix."
        },
        "scope": {
          "type": "string",
          "description": "Optional scope (e.g. crate name)."
        },
        "body": {
          "type": "string",
          "description": "Optional longer description below the subject."
        }
      }
    }
  ]
}
```

The `allOf` + `$ref` is required. Without it the engine rejects the plugin at registry-build time. The common schema (`runs-common.schema.json`) contributes `assumptions`, `confidence`, `risks`, `next_action` — all required, all present in every successful envelope.

Use whatever JSON Schema features you need beyond `allOf`: `enum`, `pattern`, nested objects, arrays of objects, conditional shapes via `oneOf`/`anyOf`. The validator is `jsonschema` (Rust) pinned to draft 2020-12. Network-fetching `$ref`s does not work — only the common schema is resolved (locally). Use inline definitions for anything else.

### Prompt

`./.rebotica/runs.d/commit-message/prompt.md`:

```markdown
# Commit Message Mode

You are a local model assisting with commit-message authoring. Propose a single commit message that matches what the supplied diff actually does. Your output is advisory to Prime, the coordinating agent.

Match the conventional-commits format. Pick the `type` from the diff's intent (a new feature is `feat`, a bug correction is `fix`, a documentation change is `docs`, etc.). Use a `scope` only when the diff is clearly scoped to one crate, package, or subsystem. Keep `subject` under 72 characters and in imperative mood ("add X" not "added X"). Use `body` only when the change is non-obvious and worth explaining — most commits don't need it.

Do not reference files that were not provided in the supplied context; if you suspect a relevant file is missing, surface that in `risks`.

Return exactly one fenced JSON block. Do not include prose outside the block.

Required fields:

- `assumptions`: array of strings.
- `confidence`: integer from 0 to 10.
- `risks`: array of strings.
- `next_action`: string.
- `subject`: string, max 72 chars, imperative mood.
- `type`: one of `feat`, `fix`, `chore`, `docs`, `refactor`, `test`, `perf`.
- `scope`: optional string.
- `body`: optional string.

```json
{
  "assumptions": [],
  "confidence": 8,
  "risks": [],
  "next_action": "Prime should review the proposed message before committing.",
  "subject": "add user-defined commit-message mode",
  "type": "feat",
  "scope": "run",
  "body": "Introduces a project-local run.* plugin that proposes a conventional commit message from the current diff."
}
```
```

The fenced ` ```json ` block at the end is the example output. The engine's JSON extractor prefers a fenced block; falling back to the last balanced `{...}` works but emits a stderr warning. Asking for the fence in the prompt is the cleanest way to keep extraction reliable.

### Try it

```sh
# from a git repo with some changes
rbtc doctor --json | jq '.data[] | select(.id == "run.plugins")'
# should report the new plugin as healthy

rbtc run commit-message --base main --json | jq '.data'
# emits the proposed commit message
```

If the model produces output that doesn't validate, you'll see an `output_invalid` envelope with structured `error.details` showing exactly which fields failed and why. The raw response is at `~/.rebotica/runs/{run_id}/model-response.md` — read that to figure out what to tighten in the prompt.

## Manifest reference

| Field | Required | Notes |
|-------|----------|-------|
| `kind` | yes | Must equal `run.<directory-name>` (lowercase). |
| `display_name` | yes | Shown in help output. |
| `description` | yes | One-sentence description; shown in help output. |
| `schema_version` | yes | Integer; you own this. Bump on breaking schema changes. |
| `inputs` | yes | Ordered list of adapter names. See [Input adapters](#input-adapters). |
| `prompt_file` | no | Defaults to `prompt.md`. Relative path within the plugin directory; `..` traversal rejected. |
| `schema_file` | no | Defaults to `schema.json`. Same path rules. |
| `exit_codes` | no | Advisory list of `error.code` values this mode may produce. Validated against `ErrorCode::all()` at registry build (typos = broken plugin). |

Unknown top-level keys make the layer broken (warn + fall through, not a hard command failure). This is strict on purpose — typoed keys would otherwise silently do nothing.

The mode-name regex `^[a-z0-9][a-z0-9_-]*$` applies to both the directory name and the part of `kind` after `run.`. `Review` or `run patch` won't load.

## Schema design

### Common fields (inherited)

Every mode inherits four required fields from `runs-common.schema.json`:

| Field | Type | Purpose |
|-------|------|---------|
| `assumptions` | `array<string>` | What the model assumed about the inputs. Read this when validating output. |
| `confidence` | `integer` 0-10 | Model self-rating. Calibration varies wildly between models; treat as relative, not absolute. |
| `risks` | `array<string>` | What the model thinks could go wrong with its output. |
| `next_action` | `string` | What Prime (or a human) should do next. |

You can't drop these; the engine enforces them via the `allOf` + `$ref` rule.

### Mode-specific fields

Anything beyond the common four lives in the second branch of your `allOf`. Best practices:

- **Be explicit about types.** `{ "type": "string" }` beats omitting type.
- **Use `enum` for closed sets.** `severity`, `kind`, `category` fields with bounded values should enumerate them. Models comply with enums more reliably than free-text constraints.
- **Use `required` for fields you actually need.** Optional fields are a license for the model to omit them.
- **Bound strings with `maxLength` when the field has a semantic limit** (commit message subject, finding summary, etc.). Catches the model when it tries to write essays.
- **Bound arrays with `maxItems` when blast radius matters.** Helps catch models that try to flood you with low-quality findings.
- **Use `description` fields.** They're not enforced but they're visible to humans reading the schema, and some validators surface them in error messages.

### Features that work

JSON Schema draft 2020-12 in full. `allOf`, `anyOf`, `oneOf`, `not`, `if`/`then`/`else`, `pattern`, `enum`, `const`, nested objects, arrays-of-objects, `additionalProperties: false` for strictness.

### Features that don't

- **External `$ref`s.** Only `runs-common.schema.json` is resolved (locally, no network). Other `$ref`s fail. Use inline definitions or `$defs` within the same schema file.
- **Schema-version migration.** Bumping `schema_version` in the manifest does not migrate persisted runs. Old runs in `~/.rebotica/runs/` were validated against whatever schema was active when they ran.

## Prompt design

### The output contract

Two rules:

1. **Ask for a fenced ` ```json ` block.** The extractor prefers it. Without a fence, the extractor falls back to the last balanced `{...}` in the response, which works but emits a stderr warning and can grab the wrong object if your model emits multiple JSON blobs in its reasoning.
2. **List the required fields with types.** Even though the schema enforces this, restating it in the prompt reduces the rate at which the model omits required fields.

The prompt is the model's contract. The schema is the validator. They should agree.

### Inheriting input context

The engine prepends the project config and each adapter's context block before your `prompt.md`. The full prompt the worker sees is:

```
## Project Config
{rendered .rebotica.yml or placeholder}

## Diff
{diff body, when `inputs` includes `diff`}

## Files
{file blocks, when `inputs` includes `files`}

## Skills
{selected skills, when `inputs` includes `skills` and at least one was passed}

{your prompt.md, verbatim}
```

You can reference these sections from your prompt ("review the Diff section above for ...") and the model will see them in that order.

### Common pitfalls

- **Don't ask for fields not in your schema.** The model might emit them; the validator will accept extras by default (unless you set `additionalProperties: false`); the consumer might rely on them; then the next prompt-author changes their mind and the consumer breaks.
- **Don't ask for free-form prose outside the JSON block.** It either gets thrown away (best case) or confuses the extractor (worst case).
- **Don't ask the model to "be honest about confidence."** Models are unreliable at confidence calibration. The field exists for the model's self-report; treat it as one signal among many, not as truth.
- **Don't include scope-restriction language as an afterthought.** Put it near the top of the prompt where it primes the model's response, not at the end where it's the last thing read but the first thing forgotten.

## Input adapters

The engine ships six built-in adapters. v1 doesn't support user-defined adapters; you compose from this fixed set.

| Adapter | Reads | Produces in prompt | Notes |
|---------|-------|---------------------|-------|
| `diff` | `--base STR`, `--range STR`, `--cached`, else working tree | Diff source description, stat, body | Mutually exclusive flags validated. Modes for code review or commit messaging want this. |
| `files` | Positional file paths after flags | Per-file fenced blocks, truncated by existing limits | Used by `explain` and `tests`. |
| `task_envelope` | First positional arg (a `.yml` path) | Envelope YAML rendered + allowed/forbidden paths extracted | Used by `patch`. |
| `skills` | `--skill ID` (repeatable, accepts `canonical:` / `project:` prefix) | Selected skill bodies; persisted to `skills.json` under the run dir | Available to any mode that declares it. |
| `project_config` | (implicit, always available) | Renders project config | Modes don't list this; engine always includes. |
| `guard` | (consumes whatever the producer adapters touched) | No prompt block; pre-check that fails fast with `GuardRejected` if a forbidden path is touched | Should be last in `inputs:`. |

### Ordering rules

The engine processes `inputs:` in declaration order. Recommended pattern:

```yaml
inputs:
  - <one or more producer adapters>     # diff / files / task_envelope, etc.
  - skills                              # if your mode benefits from skill context
  - guard                               # always last, after producers and skills
```

`project_config` is implicit and always first; you don't list it. `guard` produces no prompt block — it's purely a safety pre-check that runs after producers have populated the file set.

### Strict arg consumption

After all producer adapters have consumed their flags, any unconsumed token in `adapter_args` fails with `Usage`:

```sh
$ rbtc run commit-message --base main --typo-flag
error: unknown argument for run commit-message: --typo-flag
```

This catches typos immediately rather than silently ignoring them. If your mode wants flag-like positional arguments, document them in your prompt and remember the engine sees them as unknown.

## Testing your plugin

### Quick check

```sh
rbtc doctor --json | jq '.data[] | select(.id == "run.plugins")'
```

The `run.plugins` doctor check lists every broken plugin layer with its path and reason. If you see your plugin listed, fix it before invoking.

### Manual invocation

```sh
rbtc run my-mode <adapter args> --json | jq '.error // .data'
```

`jq '.error // .data'` shows the error if there is one, otherwise the validated payload. Add `--quiet` if you want exactly one envelope on stdout with no extras.

### Reading the failure

`output_invalid` is the most common authoring failure. The error has structured details:

```json
{
  "error": {
    "code": "output_invalid",
    "message": "model output failed schema validation",
    "details": {
      "mode": "commit-message",
      "extraction": "fence",
      "validation_errors": [
        {
          "instance_path": "/subject",
          "schema_path": "/allOf/1/properties/subject/maxLength",
          "keyword": "maxLength",
          "message": "is longer than 72 characters"
        }
      ]
    }
  }
}
```

`extraction: "fallback"` means your prompt didn't elicit a fenced block — tighten it. `validation_errors` entries point at the specific schema rule the output violated; `keyword` is what to address (`required`, `enum`, `maxLength`, `type`, etc.).

The raw model response is at `~/.rebotica/runs/{run_id}/model-response.md` and the structured failure details are at `~/.rebotica/runs/{run_id}/parse-failure.json`. Read both when diagnosing.

## Promoting: project → user → built-in

A plugin in `.rebotica/runs.d/` works only in the repo that contains it. A plugin in `~/.rebotica/runs.d/` works across every repo for that user. A plugin in `prompts/runs.d/` ships with the binary.

The natural path:

1. Prototype in `.rebotica/runs.d/<mode>/` in whichever repo you're scratching the itch in.
2. When you find yourself wanting it elsewhere, move it to `~/.rebotica/runs.d/<mode>/`.
3. If it'd benefit everyone using rebotica, contribute it to `prompts/runs.d/<mode>/` via a PR.

No code path differs by layer — the engine treats all three identically once a plugin is complete. Promotion is purely about visibility.

## v1 limits

What the plugin model deliberately doesn't do in this version:

- **No custom input adapters.** You use the six built-in adapters. Adding a new one means an engine change.
- **No hot reload.** The registry is built at process start. Edit a manifest, re-run.
- **No schema-version migration.** Bumping `schema_version` doesn't retroactively re-validate persisted runs.
- **No prompt templating.** The prompt is sent verbatim — no variable substitution from CLI args. If you need dynamic content, the adapter system is the supported path.
- **No multi-model in a single invocation.** `--model X --model Y` is rejected. Use a shell loop: `for m in X Y; do rbtc run my-mode --model $m --json; done`.
- **No async or streaming.** The engine is request-response.
- **No sandboxing of user prompts.** Single-user CLI; you trust your own plugins.

If you need any of these and a workaround isn't acceptable, file an issue.

## Related

- [output-contract.md](output-contract.md) — the v1 envelope your plugin will emit.
- [exit-codes.md](exit-codes.md) — the `ErrorCode` taxonomy your `manifest.exit_codes` validates against.
- [usage.md](usage.md) — how `rbtc` is invoked end-to-end.
