# Usage

## Install Locally

Install from a clone:

```sh
git clone https://github.com/catalandres/rebotica.git ~/Developer/rebotica
cd ~/Developer/rebotica
scripts/install.sh
```

Add the install directory to your shell path:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

For contributor builds inside this repo, `bin/rbtc` builds and runs the debug executable directly with Cargo.

## Check The Provider

Start the configured provider, then run:

```sh
rbtc doctor
rbtc providers
rbtc models
rbtc models configure --detect
rbtc health
rbtc smoke --model MODEL_ID
```

The default endpoint is:

```text
http://127.0.0.1:1234/v1
```

Override the provider, endpoint, or model when needed:

```sh
export REBOTICA_PROVIDER=lmstudio
export REBOTICA_BASE_URL=http://127.0.0.1:1234/v1
export REBOTICA_MODEL=MODEL_ID
```

You can also use config aliases:

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

Then run:

```sh
rbtc smoke --model local-coder
rbtc health --provider lmstudio
```

## Configure Model Routes

`rbtc init` works without a running provider and leaves model routes empty. Model-backed commands can still run with `--model` or `REBOTICA_MODEL`, but repeated use is easier after you configure routes explicitly.

Configure manually when you already know the provider model id:

```sh
rbtc models configure --model huihui-qwen3.6-35b-a3b-claude-4.7-opus-abliterated-mlx --alias local-coder
```

This writes `models.aliases.local-coder` and fills empty `default`, `review`, `explain`, `tests`, and `patch` routes. Existing route values are preserved unless you pass `--force`.

Or opt into provider detection:

```sh
rbtc models configure --detect
```

Detection is conservative. If the provider is unavailable, returns no models, or returns multiple candidates, the command prints actionable next steps and writes nothing. If exactly one model is returned, it writes the same alias and route entries as manual configuration.

## Onboard a Project

From a target repository:

```sh
rbtc init
```

This creates `.rebotica.yml` plus `.rebotica/tasks`, `.rebotica/runs`, and `.rebotica/.gitignore`.

If the generated config has empty model routes, init prints the route setup commands:

```sh
rbtc models configure --detect
rbtc models configure --model MODEL_ID
```

Edit `.rebotica.yml` to set:

- project type
- test and check commands
- forbidden paths
- sensitive paths
- provider settings
- model aliases
- model routing
- size limits

## Install Assets

From a target repository:

```sh
rbtc install claude
rbtc install codex
rbtc install github
```

By default this symlinks `.claude/commands` and `.claude/skills` entries back to the central harness. Use `--copy` if a project needs local copies.

`rbtc install codex` symlinks canonical Rebotica skills into `.agents/skills`.

Restricted environments can stage the same skills elsewhere:

```sh
rbtc install codex --target-dir .rebotica/adapters/codex/skills
```

`rbtc install github` copies GitHub repository assets into `.github/`. GitHub assets are copied rather than symlinked so they can be committed and work in GitHub's hosted environment.

## Review A Git Diff

```sh
rbtc run review
```

By default this reviews unstaged working-tree changes from `git diff`.

For committed feature branch work, review the branch against a base ref:

```sh
rbtc run review --base main
rbtc run review --base origin/main
```

Use an explicit range when the coordinator has already chosen exact refs:

```sh
rbtc run review --range main..HEAD
rbtc run review --range main...HEAD
```

Use staged changes when reviewing an index-only patch:

```sh
rbtc run review --cached
```

If a legitimate review is larger than the project default limits, override the limits recorded in the task envelope:

```sh
rbtc run review --base origin/main --max-files 10 --max-lines 600
```

Run multiple models side by side by repeating `--model`:

```sh
rbtc run review --base origin/main --model gemma-review --model qwen-code
```

Each model gets the same rendered prompt and its own run log.

The command sends git status, diff source, diff stat, diff, project config, and nearby repo instructions to the configured local model. Output is advisory.

## Attach Skills

Prime can attach canonical or project-local skills to a worker invocation:

```sh
rbtc skills list
rbtc skills show local-model-delegation
rbtc run review --base origin/main --skill local-model-delegation
rbtc run tests crates/rebotica-cli/src/main.rs --skill local-model-delegation
```

Project-local skills live under `.rebotica/skills/`:

```text
.rebotica/skills/frontend-review.md
.rebotica/skills/hfx-emitter/SKILL.md
```

If a project skill intentionally shares an id with a canonical skill, disambiguate the source:

```sh
rbtc run review --base origin/main --skill canonical:local-model-delegation
rbtc run review --base origin/main --skill project:frontend-review
```

Selected skills are included in the prompt and logged as `skills.json` under the run directory. They are context only; they cannot override Rebotica contracts, forbidden paths, sensitive paths, or task limits.

## Score Worker Output

After a model-backed run, Rebotica prints the run id to stderr and suggests follow-up commands for Prime.

Record model performance feedback locally:

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

The event log is append-only. The scorecards file is a summary that can help Prime route future review, test, explanation, and patch tasks to models that have performed well.

## Comment Cards

Comment cards are product feedback about Rebotica itself. They are separate from model scorecards.

Create a local card:

```sh
rbtc comment-card new \
  --from-run RUN_ID \
  --kind ux \
  --area review \
  --source prime \
  --title "review should explain empty diffs better" \
  --body "Observed behavior, expected behavior, workaround, suggested fix."
```

Cards live locally until submitted or dismissed:

```text
~/.rebotica/comment-cards/pending/
~/.rebotica/comment-cards/submitted/
~/.rebotica/comment-cards/dismissed/
```

Manage cards:

```sh
rbtc comment-card list
rbtc comment-card show CARD_ID
rbtc comment-card dismiss CARD_ID
```

GitHub submission requires explicit consent:

```sh
rbtc comment-card consent --allow-github --repo catalandres/rebotica
rbtc comment-card submit CARD_ID
```

Submission uses `gh issue create` and attempts to create the comment-card labels first.

`rbtc doctor` reports whether submission consent is enabled and how many pending cards are queued.

## Pre-Flight Triage

Use local review as a cheap first pass before a stronger Prime-agent review:

```sh
rbtc health
rbtc guard-diff --base origin/main
rbtc run review --base origin/main --model LOCAL_MODEL_ID --skill local-model-delegation
rbtc run tests crates/rebotica-cli/src/main.rs --skill local-model-delegation
```

Treat local-model output as triage. It can catch obvious missing tests, dangling references, and documentation drift, but Prime still owns final review and acceptance.

## Guard A Diff

Use `guard-diff` to enforce forbidden paths and size limits before review:

```sh
rbtc guard-diff
rbtc guard-diff --cached
rbtc guard-diff --base origin/main
rbtc guard-diff --range main..HEAD --max-files 10 --max-lines 600
```

This command does not call a model. It reports JSON and is suitable for local pre-commit checks or CI jobs that can provide the correct base or range.

## Explain Files

```sh
rbtc run explain crates/rebotica-cli/src/main.rs
```

This is read-only and logs the run.

## Propose Tests

```sh
rbtc run tests crates/rebotica-git/src/lib.rs
```

The worker proposes tests and gaps. It does not write files.

## Patch Drafting

Patch mode is dry-run-first:

```sh
rbtc run patch .rebotica/tasks/example.yml --dry-run
```

The worker returns a proposed unified diff. Prime reviews it before any application.
