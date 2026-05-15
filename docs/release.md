# Release

Rebotica's first public distribution should stay boring: a tagged source release, a source-building Homebrew tap formula, and a local install smoke harness that proves the installed shim can find its runtime assets.

## Local Install Harness

Run:

```sh
just install-smoke
```

This installs `rbtc` into `target/local-install/prefix`, creates a sandbox project under `target/local-install/`, and verifies:

- `rbtc --version`
- `rbtc help`
- `rbtc init`
- `rbtc providers --json`
- `rbtc models --configured-only`
- `rbtc doctor --json`
- `rbtc install codex --copy`
- `rbtc install claude --copy`
- `rbtc install github`

Use a custom prefix when needed:

```sh
just install-smoke /tmp/rebotica-prefix
```

The smoke harness does not require a running local model provider. Provider checks that hit `/models` remain a separate manual step.

## Release Gate

Before cutting a tag:

```sh
just release-check
```

Then run provider-backed checks against a real local provider:

```sh
rbtc health
rbtc smoke --model MODEL_ALIAS_OR_ID
```

Check the public conventions before release:

- CLI is `rbtc`.
- Version is `rbtc --version`, not a `version` subcommand.
- Config paths are `.rebotica.yml` or `.rebotica/project.yml`.
- Project state is `.rebotica/`.
- Private global state is `~/.rebotica/`.
- Environment variables use the `REBOTICA_` prefix.
- No public docs or prompts reintroduce old names.

## Tag Checklist

1. Confirm the working tree only contains intentional release changes.
2. Run `just release-check`.
3. Run provider-backed `rbtc health` and `rbtc smoke`.
4. Update release notes with CLI, config, prompt, safety, and migration changes.
5. Create an annotated tag, for example `v0.2.0`.
6. Push the tag.
7. Create a GitHub release from the tag.
8. Record the source tarball SHA-256 for the Homebrew formula.

## Homebrew Strategy

Start with a general-purpose personal tap:

```sh
catalandres/homebrew-tap
```

Users would install with:

```sh
brew install catalandres/tap/rebotica
```

Manual tap installation is also fine:

```sh
brew tap catalandres/tap
brew install rebotica
```

The formula should build from the tagged source archive, install the binary and runtime assets under `libexec`, and write a `bin/rbtc` shim that sets `REBOTICA_HOME` to that `libexec` directory.

Use [packaging/homebrew/rebotica.rb.template](../packaging/homebrew/rebotica.rb.template) as the starting point.

Formula update flow:

```sh
VERSION=v0.2.0
curl -L -o rebotica-$VERSION.tar.gz \
  https://github.com/catalandres/rebotica/archive/refs/tags/$VERSION.tar.gz
shasum -a 256 rebotica-$VERSION.tar.gz
```

Then replace the formula `url` and `sha256`, and test locally:

```sh
brew install --build-from-source --verbose ./Formula/rebotica.rb
brew test rebotica
```

After the tap is published, test the user-facing path:

```sh
brew install --build-from-source --verbose catalandres/tap/rebotica
brew test catalandres/tap/rebotica
```

## Bottles Later

Do not start with bottles. First prove:

- tagged source releases are repeatable
- the shim reliably sets `REBOTICA_HOME`
- runtime assets remain stable
- `rbtc install claude|codex|github` works from the brewed package

Once those are stable, add tap CI and bottles.
