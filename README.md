# Atelier

Atelier is a reusable local-model delegation harness for governed collaborative craftsmanship.

It keeps a root coding agent, such as Claude Code, in charge while bounded workers exposed through OpenAI-compatible providers help with review, explanation, test proposals, documentation cleanup, and small patch drafts.

The core idea is simple:

```text
root coordinator
  -> explicit task envelope
  -> narrow local worker contract
  -> advisory output or bounded diff
  -> coordinator review, tests, and acceptance gates
```

Atelier is not an autonomous coding swarm. It is a set of contracts, prompts, scripts, guards, logs, and docs for delegating bounded work safely.

## Status

This repository starts with a Rust CLI and a project-agnostic file structure. The shell bridge comes first; MCP and Aider-style worktree patching are intentionally secondary.

Implemented in this first version:

- Provider health and smoke checks, with LM Studio as the default local provider.
- `atelier doctor` for config validation and environment diagnostics.
- `atelier models` and `atelier providers` for routing visibility.
- `atelier init` project onboarding.
- `atelier install claude|codex|github|all` for root-agent and repository assets.
- `atelier review` for current git diffs.
- `atelier explain <file...>` for file explanation.
- `atelier tests <file...>` for test proposals.
- `atelier patch <task-envelope.yml> --dry-run` guard flow.
- Run logging under `~/.atelier/runs`.
- Prompt contracts, templates, Claude commands, and Claude skills.
- MCP server source scaffold with narrow tool boundaries.

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
export ATELIER_PROVIDER=lmstudio
export ATELIER_BASE_URL=http://127.0.0.1:1234/v1
export ATELIER_MODEL=qwen-coder
```

## Quick Start

Install from a clone:

```sh
git clone https://github.com/catalandres/atelier.git ~/Developer/atelier
cd ~/Developer/atelier
scripts/install.sh
export PATH="$HOME/.local/bin:$PATH"
atelier doctor
atelier health
atelier smoke --model YOUR_MODEL_ID
```

From a target project:

```sh
atelier init
atelier install claude
atelier review
atelier explain src/main.rs
atelier tests src/main.rs
atelier patch .atelier/tasks/example.yml --dry-run
```

## Project Configuration

Each project opts in with `.atelier.yml` or `.atelier/project.yml`.

Start with:

```sh
atelier init
```

That creates:

```text
.atelier.yml
.atelier/
  .gitignore
  tasks/
  runs/
```

The project config describes commands, forbidden paths, sensitive paths, providers, model aliases, limits, and preferred model routes. See [templates/project.atelier.yml](templates/project.atelier.yml).

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
  default: qwen-worker
  review: qwen-worker
  tests: qwen-worker
  aliases:
    qwen-worker: huihui-qwen3.6-35b-a3b-claude-4.7-opus-abliterated-mlx
```

The CLI accepts either aliases or raw values:

```sh
atelier smoke --model qwen-worker
atelier health --provider lmstudio
atelier health --base-url http://127.0.0.1:1234/v1
```

## Philosophy

Atelier delegates bounded work, not ambiguity.

The root coordinator owns judgment: decomposition, scope, worker selection, patch acceptance, test execution, and final responsibility. Local models are useful precisely when their work is constrained, logged, reversible, and reviewed.

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

## Repository Layout

```text
crates/                      Rust workspace crates
bin/                         executable CLI entrypoints
scripts/                     install and contributor helper scripts
prompts/system/              role prompts
prompts/contracts/           worker output contracts
mcp/local-model-server/      future narrow MCP bridge
skills/                      canonical root-agent skills
claude/commands/             reusable Claude Code slash commands
codex/                       Codex adapter notes
github/                      GitHub repository assets
templates/                   project, task, and scorecard templates
docs/                        architecture and operating guidance
```

## Safety Defaults

Atelier defaults to advisory output. Patch mode starts as dry-run-first and must pass guard checks before a human or root coordinator chooses to apply anything.

Local workers must not push, commit, merge, add dependencies, edit forbidden paths, or claim checks passed unless the harness actually ran them.

## License

MIT.
