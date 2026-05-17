# Usage

Practical guide to running `rbtc`. The wire format is specified in [output-contract.md](output-contract.md); the `error.code` table and per-code `details` shapes live in [exit-codes.md](exit-codes.md).

## Five-minute path

A coordinating agent wiring `rbtc` as a subprocess needs four things: install, set quiet mode, parse the envelope, and branch on the exit code.

### Install

```sh
git clone https://github.com/catalandres/rebotica.git
cd rebotica
scripts/install.sh
export PATH="$HOME/.local/bin:$PATH"
```

Or with Cargo:

```sh
cargo install --path crates/rebotica-cli
```

### First call

```sh
rbtc doctor --json | jq
```

You should see a v1 envelope on stdout. The shape:

```json
{
  "rebotica": "v1",
  "kind": "doctor",
  "ok": true,
  "command": "doctor",
  "data": { /* per-kind payload */ },
  "error": null,
  "run_id": null,
  "started_at": "2026-05-15T22:00:00Z",
  "duration_ms": 42
}
```

`rebotica` is the schema version. `kind` is the discriminator for `data`. `ok` is `true` iff `error` is `null`. See [output-contract.md](output-contract.md) for every field.

### Quiet mode

For subprocess use, prefer `--quiet`:

```sh
rbtc doctor --quiet
```

`--quiet` implies `--json` and guarantees exactly one envelope on stdout with nothing on stderr. Equivalent via env:

```sh
REBOTICA_QUIET=1 rbtc doctor
```

### Exit codes

The process exit code is derived from `error.code`. To branch on it from a parent process:

```sh
rbtc guard-diff --quiet > out.json
exit_code=$?

if [ "$exit_code" -eq 0 ]; then
  : # success
elif [ "$exit_code" -eq 20 ]; then
  jq '.error.details.rejected_paths' out.json   # guard_rejected
elif [ "$exit_code" -eq 10 ]; then
  : # provider_unavailable; retry after provider startup
fi
```

See [exit-codes.md](exit-codes.md) for the full table and per-code `error.details` shapes.

## Global flags and environment

`--json` and `--quiet` are global; they are accepted before or after the subcommand.

| Flag | Env | Effect |
| --- | --- | --- |
| `--json` | `REBOTICA_JSON=1` | Emit the v1 envelope on stdout. Stderr remains available for progress. |
| `--quiet` | `REBOTICA_QUIET=1` | Imply `--json`; suppress non-error stderr. |

Env truthy values are `1`, `true`, `yes` (case-insensitive).

Provider-related env vars used by setup commands:

| Variable | Purpose |
| --- | --- |
| `REBOTICA_PROVIDER` | Provider name from config, or an OpenAI-compatible base URL. |
| `REBOTICA_BASE_URL` | Provider base URL override (e.g. `http://127.0.0.1:1234/v1`). |
| `REBOTICA_MODEL` | Model alias or raw provider model id. |

## Reading the output

Every state command emits the envelope shape documented in [output-contract.md](output-contract.md). The `kind` discriminator tells you what `data` to expect. Kinds emitted today:

| Group | Kinds |
| --- | --- |
| Setup and status | `doctor`, `providers`, `models`, `models.configure`, `health`, `smoke`, `init`, `install` |
| Skills | `skills.list`, `skills.show` |
| Policy and safety | `guard-diff` |
| Feedback and learning | `score`, `scorecards`, `comment-card.new`, `comment-card.list`, `comment-card.show`, `comment-card.dismiss`, `comment-card.consent`, `comment-card.submit`, `retro` |
| Delegated work | `run.review`, `run.explain`, `run.tests`, `run.patch` |

The literal `"error"` kind appears when an envelope must be emitted before a subcommand can be resolved (parse failure, missing subcommand, top-level cancellation). Branch on `error.code` only when you see it.

For the full taxonomy including reserved-but-not-yet-emitted kinds, see [output-contract.md](output-contract.md#kind-taxonomy).

## Working with jq

A few patterns that come up. All assume `--json` or `--quiet`.

Extract the payload:

```sh
rbtc models --quiet | jq '.data'
```

Branch on success:

```sh
rbtc health --quiet | jq 'if .ok then .data.models else .error end'
```

Switch on the error code:

```sh
rbtc guard-diff --quiet | jq -r '
  if .ok then "pass"
  elif .error.code == "guard_rejected" then "rejected: \(.error.details.rejected_paths | join(", "))"
  elif .error.code == "over_limit" then "too large: \(.error.details.kind) \(.error.details.actual) > \(.error.details.limit)"
  else "fail: \(.error.code) - \(.error.message)" end
'
```

Read the run id when present:

```sh
rbtc score RUN_ID --rating 4 --accepted --quiet | jq -r '.run_id'
```

## Setup commands

Start the configured provider, then check that the environment is healthy:

```sh
rbtc doctor
rbtc providers
rbtc models
rbtc models configure --detect
rbtc health
rbtc smoke --model MODEL_ID
```

The default endpoint is `http://127.0.0.1:1234/v1`. Override via env or config:

```sh
export REBOTICA_BASE_URL=http://127.0.0.1:1234/v1
export REBOTICA_MODEL=MODEL_ID
```

Or with config aliases in `.rebotica.yml`:

```yaml
providers:
  default: lmstudio
  lmstudio:
    kind: openai-compatible
    base_url: http://127.0.0.1:1234/v1

models:
  default: local-coder
  review: local-coder
  explain: local-coder
  tests: local-coder
  patch: local-coder
  aliases:
    local-coder: huihui-qwen3.6-35b-a3b-claude-4.7-opus-abliterated-mlx
```

Then:

```sh
rbtc smoke --model local-coder
rbtc health --provider lmstudio
```

### Configure model routes

`rbtc init` works without a running provider and leaves model routes empty. Model-backed commands can still run with `--model` or `REBOTICA_MODEL`, but repeated use is easier after explicit routing.

Manual configuration when the provider model id is known:

```sh
rbtc models configure --model huihui-qwen3.6-35b-a3b-claude-4.7-opus-abliterated-mlx --alias local-coder
```

This writes `models.aliases.local-coder` and fills empty `default`, `review`, `explain`, `tests`, and `patch` routes. Existing route values are preserved unless `--force` is passed.

Or opt into provider detection:

```sh
rbtc models configure --detect
```

Detection is conservative. If the provider is unavailable, returns no models, or returns multiple candidates, the command writes nothing and the envelope's `data` describes the next step. If exactly one model is returned, it writes the same alias and route entries as manual configuration.

## Project onboarding

From a target repository:

```sh
rbtc init
```

This creates `.rebotica.yml` plus `.rebotica/tasks`, `.rebotica/runs`, and `.rebotica/.gitignore`. The project config describes commands, forbidden paths, sensitive paths, providers, model aliases, limits, and preferred model routes. See [templates/project.rebotica.yml](../templates/project.rebotica.yml).

### Install adapter assets

```sh
rbtc install claude
rbtc install codex
rbtc install github
rbtc install all
```

By default this symlinks `.claude/commands` and `.claude/skills` entries back to the central harness. Use `--copy` if a project needs local copies.

`rbtc install codex` symlinks canonical skills into `.agents/skills`. Restricted environments can stage them elsewhere:

```sh
rbtc install codex --target-dir .rebotica/adapters/codex/skills
```

`rbtc install github` copies GitHub repository assets into `.github/` rather than symlinking, so they can be committed and work in GitHub's hosted environment.

## Skills

Skills are reusable prompt context that a coordinating agent can attach to a run.

```sh
rbtc skills list
rbtc skills show local-model-delegation
```

Project-local skills live under `.rebotica/skills/`:

```text
.rebotica/skills/frontend-review.md
.rebotica/skills/hfx-emitter/SKILL.md
```

When a project skill intentionally shares an id with a canonical skill, disambiguate the source with a `canonical:` or `project:` prefix on the id. Skills are context only; they cannot override Rebotica contracts, forbidden paths, sensitive paths, or task limits.

## Policy and safety

`guard-diff` enforces forbidden paths and size limits on a selected git diff:

```sh
rbtc guard-diff
rbtc guard-diff --cached
rbtc guard-diff --base origin/main
rbtc guard-diff --range main..HEAD --max-files 10 --max-lines 600
```

This command does not call a model. It is suitable for local pre-commit checks or CI jobs that can provide the correct base or range.

Failure modes attach typed `error.code` values:

- `guard_rejected` (exit 20): a forbidden path was touched. `error.details` includes `rejected_paths` and `forbidden_pattern`.
- `over_limit` (exit 22): file or line limit exceeded. `error.details` includes `kind`, `limit`, `actual`.

## Feedback and learning

After a model-backed run, the run id is printed to stderr. Record feedback locally:

```sh
rbtc score RUN_ID --rating 4 --accepted --label useful-review --notes "Caught a missing test"
rbtc score RUN_ID --rating 2 --rejected --label false-positive
rbtc scorecards
```

Scoring writes:

```text
~/.rebotica/runs/RUN_ID/feedback.yml
~/.rebotica/model-events.jsonl
~/.rebotica/model-scorecards.yml
```

The event log is append-only. Scorecards summarize how a model has performed and can inform future routing decisions.

### Comment cards

Comment cards are product feedback about Rebotica itself; they are separate from model scorecards.

```sh
rbtc comment-card new \
  --from-run RUN_ID \
  --kind ux \
  --area review \
  --source prime \
  --title "review should explain empty diffs better" \
  --body "Observed behavior, expected behavior, workaround, suggested fix."

rbtc comment-card list
rbtc comment-card show CARD_ID
rbtc comment-card dismiss CARD_ID
```

Cards live locally until submitted or dismissed:

```text
~/.rebotica/comment-cards/pending/
~/.rebotica/comment-cards/submitted/
~/.rebotica/comment-cards/dismissed/
```

GitHub submission requires explicit consent and uses `gh issue create`:

```sh
rbtc comment-card consent --allow-github --repo catalandres/rebotica
rbtc comment-card submit CARD_ID
```

`rbtc doctor` reports whether submission consent is enabled and how many cards are pending.

### Retrospectives

`rbtc retro --from-run RUN_ID` creates a retrospective template from a saved run.

## `run.*`

`run review`, `run explain`, `run tests`, and `run patch` are model-backed plugin modes resolved from `runs.d/` registries. In `--json` or `--quiet` mode they emit the v1 envelope kinds `run.review`, `run.explain`, `run.tests`, and `run.patch`.

Each mode validates the model response against its declared JSON Schema. `output_invalid` means the provider returned text but Rebotica could not extract JSON or the parsed JSON did not match the schema. When a run reaches the provider call, the envelope carries `run_id` and the audit artifacts are written under `~/.rebotica/runs/{run_id}/`.

Dynamic mode help comes from the registry:

```sh
rbtc run review --help
rbtc run patch --help
```
