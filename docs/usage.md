# Usage

## Install Locally

Install from a clone:

```sh
git clone https://github.com/catalandres/atelier.git ~/Developer/atelier
cd ~/Developer/atelier
scripts/install.sh
```

Add the install directory to your shell path:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

For contributor builds inside this repo, `bin/atelier` builds and runs the debug executable directly with Cargo.

## Check The Provider

Start the configured provider, then run:

```sh
atelier doctor
atelier providers
atelier models
atelier health
atelier smoke --model MODEL_ID
```

The default endpoint is:

```text
http://127.0.0.1:1234/v1
```

Override the provider, endpoint, or model when needed:

```sh
export ATELIER_PROVIDER=lmstudio
export ATELIER_BASE_URL=http://127.0.0.1:1234/v1
export ATELIER_MODEL=MODEL_ID
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
atelier smoke --model local-coder
atelier health --provider lmstudio
```

## Onboard a Project

From a target repository:

```sh
atelier init
```

This creates `.atelier.yml` plus `.atelier/tasks`, `.atelier/runs`, and `.atelier/.gitignore`.

Edit `.atelier.yml` to set:

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
atelier install claude
atelier install codex
atelier install github
```

By default this symlinks `.claude/commands` and `.claude/skills` entries back to the central harness. Use `--copy` if a project needs local copies.

`atelier install codex` symlinks canonical Atelier skills into `.agents/skills`.

Restricted environments can stage the same skills elsewhere:

```sh
atelier install codex --target-dir .atelier/adapters/codex/skills
```

`atelier install github` copies GitHub repository assets into `.github/`. GitHub assets are copied rather than symlinked so they can be committed and work in GitHub's hosted environment.

## Review Current Diff

```sh
atelier review
```

The command sends git status, git diff stat, git diff, project config, and nearby repo instructions to the configured local model. Output is advisory.

## Explain Files

```sh
atelier explain crates/atelier-cli/src/main.rs
```

This is read-only and logs the run.

## Propose Tests

```sh
atelier tests crates/atelier-git/src/lib.rs
```

The worker proposes tests and gaps. It does not write files.

## Patch Drafting

Patch mode is dry-run-first:

```sh
atelier patch .atelier/tasks/example.yml --dry-run
```

The worker returns a proposed unified diff. The root coordinator reviews it before any application.
