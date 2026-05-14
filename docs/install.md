# Installation

Rebotica is a Rust command-line tool plus a repository of prompts, templates, Claude assets, and docs. The installed command should know where the cloned repo lives, so installation uses a small shim that sets `REBOTICA_HOME`.

## Install From Source

Install a current stable Rust toolchain first.

Clone the repo:

```sh
git clone https://github.com/catalandres/rebotica.git ~/Developer/rebotica
cd ~/Developer/rebotica
```

Install into `~/.local/bin`:

```sh
scripts/install.sh
```

Or choose a prefix:

```sh
scripts/install.sh /opt/homebrew
```

Make sure the install directory is on `PATH`:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Verify:

```sh
rbtc --version
rbtc health
```

## Make Targets

```sh
make build
make release
make install
make verify
```

`make install PREFIX=/opt/homebrew` installs the shim into `/opt/homebrew/bin`.

## Project Onboarding

In a project repo:

```sh
rbtc init
rbtc install claude
```

This creates:

```text
.rebotica.yml
.rebotica/
  .gitignore
  tasks/
  runs/
```

Commit `.rebotica.yml` when the team wants shared governance rules, provider routes, model aliases, and safety limits. Keep `.rebotica/runs/` private; the generated `.rebotica/.gitignore` ignores it.

Install other adapters as needed:

```sh
rbtc install codex
rbtc install github
rbtc install all
```

In restricted agent sandboxes, stage Codex skills under `.rebotica` instead of writing directly to `.agents`:

```sh
rbtc install all --target-dir .rebotica/adapters/codex/skills
```

## Upgrades

For now, upgrade by pulling a tagged release and reinstalling the shim:

```sh
cd ~/Developer/rebotica
git fetch --tags
git checkout v0.1.0
scripts/install.sh
```

Teams that want faster iteration can track `main`, but project work should prefer tagged versions once releases exist.

## Future Distribution

The intended order is:

1. Source install with a shim.
2. Tagged GitHub releases with checksums.
3. Homebrew tap once the command surface stabilizes.
4. Optional prebuilt binary artifacts after the runtime asset story is settled.
