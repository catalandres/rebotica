# Rebotica

Rebotica is a workshop for local agents: a reusable delegation harness for governed collaborative craftsmanship.

It keeps Prime, the coordinating agent such as Claude Code, in charge while local models exposed through OpenAI-compatible providers help with review, explanation, test proposals, documentation cleanup, and small patch drafts.

The core idea is simple:

```text
Prime
  -> explicit task envelope
  -> scoped local-model contract
  -> advisory output or scoped diff
  -> Prime review, tests, and acceptance gates
```

Rebotica is not an autonomous coding swarm. It is a set of contracts, prompts, scripts, guards, logs, and docs for delegating scoped work safely.

## Name

*Rebotica* (reh-boh-TEE-kah · /reβoˈtika/) is Spanish for the *trastienda* — the back room — of a *farmacia* (pharmacy). It is where the pharmacist works behind the counter, out of public view: weighing ingredients on careful scales, compounding preparations from recorded recipes, consulting with apprentices over the open formulary. The customer at the front of the shop never sees the work; they trust the labeled bottle because the pharmacist's discipline — recipe, measure, log, signature — makes that trust safe. In the Spanish literary tradition the rebotica is also where the *tertulia* gathers: the back-room conversation among trusted hands, where ideas are weighed before they reach the street.

A fitting name for a workshop where local agents do scoped work behind Prime: task envelopes for the recipe, guards for the scale, run logs for the formulary, and Prime signing off on what reaches the user.

## Status

This repository starts with a Rust CLI and a project-agnostic file structure. The shell bridge comes first; MCP and Aider-style worktree patching are intentionally secondary.

Implemented in this first version:

- Setup and status: `rbtc init`, `rbtc doctor`, `rbtc providers`, `rbtc models`, `rbtc models configure`, `rbtc health`, `rbtc smoke`, and `rbtc install claude|codex|github|all`.
- Delegated work: `rbtc run review`, `rbtc run explain <file...>`, `rbtc run tests <file...>`, and `rbtc run patch <task-envelope.yml> --dry-run`.
- Policy and safety: `rbtc guard-diff` for forbidden-path and size-limit checks on selected git diffs.
- Skills and prompts: `rbtc skills list|show`, prompt contracts, templates, Prime-agent adapter assets, and Prime-selected skill context.
- Feedback and learning: `rbtc score`, `rbtc scorecards`, `rbtc comment-card`, run logging under `~/.rebotica/runs`, and local-first product feedback about Rebotica.
- Future bridge: MCP server source scaffold with narrow tool boundaries.

## Requirements

- Current stable Rust toolchain with Cargo.
- Git.
- LM Studio or another OpenAI-compatible provider when invoking model-backed commands.

Default local provider endpoint:

```sh
http://127.0.0.1:1234/v1
```

You can override the provider, endpoint, or model:

```sh
export REBOTICA_PROVIDER=lmstudio
export REBOTICA_BASE_URL=http://127.0.0.1:1234/v1
export REBOTICA_MODEL=qwen-coder
```

## Quick Start

Install from a clone:

```sh
git clone https://github.com/catalandres/rebotica.git ~/Developer/rebotica
cd ~/Developer/rebotica
scripts/install.sh
export PATH="$HOME/.local/bin:$PATH"
rbtc doctor
rbtc health
rbtc smoke --model YOUR_MODEL_ID
```

From a target project:

```sh
rbtc init
rbtc models configure --detect
rbtc install claude
rbtc skills list
rbtc guard-diff --base main
rbtc run review --base main --skill local-model-delegation
rbtc run review --base main --model gemma-review --model qwen-code
rbtc score RUN_ID --rating 4 --accepted --label useful-review
rbtc scorecards
rbtc comment-card new --from-run RUN_ID --kind ux --area review --source prime --title "review feedback"
rbtc run explain src/main.rs
rbtc run tests src/main.rs
rbtc run patch .rebotica/tasks/example.yml --dry-run
```

## Project Configuration

Each project opts in with `.rebotica.yml` or `.rebotica/project.yml`.

Start with:

```sh
rbtc init
```

That creates:

```text
.rebotica.yml
.rebotica/
  .gitignore
  tasks/
  runs/
```

The project config describes commands, forbidden paths, sensitive paths, providers, model aliases, limits, and preferred model routes. See [templates/project.rebotica.yml](templates/project.rebotica.yml).

`rbtc init` intentionally leaves model routes empty. Use `rbtc models configure --detect` when a provider such as LM Studio is running with exactly one loaded model, or use `rbtc models configure --model MODEL_ID` to configure the route manually while offline.

## Providers And Model Aliases

Aliases are useful because local model ids can be long and because different projects may route work to different OpenAI-compatible providers.

```yaml
providers:
  default: lmstudio
  lmstudio:
    kind: openai-compatible
    base_url: http://127.0.0.1:1234/v1
  openai:
    kind: openai-compatible
    base_url: https://api.openai.com/v1
    api_key_env: OPENAI_API_KEY

models:
  default: qwen-model
  review: qwen-model
  tests: qwen-model
  aliases:
    qwen-model: huihui-qwen3.6-35b-a3b-claude-4.7-opus-abliterated-mlx
```

The CLI accepts either aliases or raw values:

```sh
rbtc models configure --detect
rbtc models configure --model huihui-qwen3.6-35b-a3b-claude-4.7-opus-abliterated-mlx --alias qwen-model
rbtc smoke --model qwen-model
rbtc health --provider lmstudio
rbtc health --base-url http://127.0.0.1:1234/v1
```

## Philosophy

Rebotica delegates scoped work, not ambiguity.

Prime owns judgment: decomposition, scope, model selection, patch acceptance, test execution, and final responsibility. Local models are useful precisely when their work is constrained, logged, reversible, and reviewed.

Read more in [docs/philosophy.md](docs/philosophy.md).

## Documentation

- [Usage](docs/usage.md)
- [Installation](docs/install.md)
- [Architecture](docs/architecture.md)
- [Providers](docs/providers.md)
- [Skills](docs/skills.md)
- [Operating Model](docs/operating-model.md)
- [Governance](docs/governance.md)
- [Safety Model](docs/safety-model.md)
- [Self-Healing](docs/self-healing.md)
- [Roadmap](docs/roadmap.md)
- [Release](docs/release.md)

## Repository Layout

```text
crates/                      Rust workspace crates
bin/                         executable CLI entrypoints
scripts/                     install and contributor helper scripts
prompts/system/              role prompts
prompts/contracts/           local-model output contracts
mcp/rebotica-server/          future narrow MCP bridge
skills/                      canonical Prime-agent skills
claude/commands/             reusable Claude Code slash commands
codex/                       Codex adapter notes
github/                      GitHub repository assets
templates/                   project, task, and scorecard templates
docs/                        architecture and operating guidance
```

## Safety Defaults

Rebotica defaults to advisory output. Patch mode starts as dry-run-first and must pass guard checks before a human or Prime chooses to apply anything.

Local models must not push, commit, merge, add dependencies, edit forbidden paths, or claim checks passed unless the harness actually ran them.

## License

MIT.
