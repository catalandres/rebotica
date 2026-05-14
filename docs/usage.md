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

## Onboard a Project

From a target repository:

```sh
rbtc init
```

This creates `.rebotica.yml` plus `.rebotica/tasks`, `.rebotica/runs`, and `.rebotica/.gitignore`.

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

## Review Current Diff

```sh
rbtc review
```

The command sends git status, git diff stat, git diff, project config, and nearby repo instructions to the configured local model. Output is advisory.

## Explain Files

```sh
rbtc explain crates/rebotica-cli/src/main.rs
```

This is read-only and logs the run.

## Propose Tests

```sh
rbtc tests crates/rebotica-git/src/lib.rs
```

The worker proposes tests and gaps. It does not write files.

## Patch Drafting

Patch mode is dry-run-first:

```sh
rbtc patch .rebotica/tasks/example.yml --dry-run
```

The worker returns a proposed unified diff. The root coordinator reviews it before any application.
