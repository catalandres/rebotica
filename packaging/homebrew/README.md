# Homebrew Packaging

Rebotica needs more than a single binary. The CLI reads prompts, templates, skills, and adapter assets from `REBOTICA_HOME`, so the Homebrew formula installs runtime assets under `libexec` and exposes a small `rbtc` shim that sets `REBOTICA_HOME`.

Use `rebotica.rb.template` as the starting point for a tap formula.

Expected tap:

```sh
brew tap catalandres/rebotica
brew install rebotica
```

During early releases, prefer source builds from signed tags. Bottles can come after the CLI and runtime asset layout have settled.
