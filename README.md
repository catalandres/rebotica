# Rebotica

A Rust CLI (`rbtc`) that runs local-model work as a subprocess of a coordinating agent.

A coordinating agent — call it Prime — drives the work: it decides scope, picks the model, reads the output, runs the tests, decides what to keep. `rbtc` is the subprocess Prime calls when it wants a local model to do something specific: check a provider, list configured skills, guard a diff, score a run, draft a patch. Output is a structured envelope on stdout; prose stays on stderr.

## Name

*Rebotica* (reh-boh-TEE-kah · /reβoˈtika/) is Spanish for the *trastienda* — the back room — of a *farmacia* (pharmacy). It is where the pharmacist works behind the counter, out of public view: weighing ingredients on careful scales, compounding preparations from recorded recipes, consulting with apprentices over the open formulary. The customer at the front of the shop never sees the work; they trust the labeled bottle because the pharmacist's discipline — recipe, measure, log, signature — makes that trust safe. In the Spanish literary tradition the rebotica is also where the *tertulia* gathers: the back-room conversation among trusted hands, where ideas are weighed before they reach the street.

A fitting name for a workshop where local agents do scoped work behind Prime: task envelopes for the recipe, guards for the scale, run logs for the formulary, and Prime signing off on what reaches the user.

## Status

Version `0.2.0`. Single-user project; the only consumer is the author's coordinating agent.

What's stable:

- The v1 JSON envelope contract for every state command (`doctor`, `providers`, `models`, `models configure`, `health`, `smoke`, `init`, `install`, `skills list/show`, `guard-diff`, `score`, `scorecards`, `comment-card *`, `retro`).
- The v1 JSON envelope contract for built-in model-backed run modes (`run review`, `run explain`, `run tests`, `run patch`).
- The typed `ErrorCode` taxonomy and exit-code mapping.
- The CLI surface for the commands above (flags, env vars, output channels).

What's in flux:

- The mode-author walkthrough for custom `run.*` plugins is still pending under epic [#5](https://github.com/catalandres/rebotica/issues/5).

## Requirements

- Rust toolchain (current stable) with Cargo.
- Git.
- An OpenAI-compatible provider when invoking model-backed commands. LM Studio works out of the box at `http://127.0.0.1:1234/v1`.

## Install

Clone and run the install script:

```sh
git clone https://github.com/catalandres/rebotica.git
cd rebotica
scripts/install.sh
export PATH="$HOME/.local/bin:$PATH"
```

The script installs `rbtc` into `$HOME/.local/bin`. Alternatively, install with Cargo directly:

```sh
cargo install --path crates/rebotica-cli
```

Then check that the environment is healthy:

```sh
rbtc doctor
```

## Provider setup

Configure the provider via environment variables:

```sh
export REBOTICA_PROVIDER=lmstudio
export REBOTICA_BASE_URL=http://127.0.0.1:1234/v1
export REBOTICA_MODEL=qwen-coder
```

Or set them in `.rebotica.yml` after running `rbtc init` in a target project. See [docs/usage.md](docs/usage.md) for the full setup walkthrough.

## Output contract (one screen)

Every state command emits a v1 envelope on stdout when `--json` or `--quiet` is set. Prose goes to stderr. The exit code derives from `error.code`, not from matching message text.

```sh
$ rbtc doctor --json
{
  "rebotica": "v1",
  "kind": "doctor",
  "ok": true,
  "command": "doctor",
  "data": { /* ... per-kind payload ... */ },
  "error": null,
  "run_id": null,
  "started_at": "2026-05-15T22:00:00Z",
  "duration_ms": 42
}
```

`--quiet` (or `REBOTICA_QUIET=1`) implies `--json` and guarantees exactly one envelope on stdout with nothing on stderr. Use it from a parent process.

See [docs/output-contract.md](docs/output-contract.md) for the full envelope spec, [docs/usage.md](docs/usage.md) for the practical guide, and [docs/exit-codes.md](docs/exit-codes.md) for the `error.code` table.

## Command groups

- **Setup and status:** `init`, `doctor`, `providers`, `models`, `models configure`, `health`, `smoke`, `install`.
- **Skills:** `skills list`, `skills show`.
- **Policy and safety:** `guard-diff`.
- **Feedback and learning:** `score`, `scorecards`, `comment-card *`, `retro`.
- **Delegated work:** `run review`, `run explain`, `run tests`, `run patch`.

## State on disk

- Per-project: `.rebotica.yml` and `.rebotica/` (created by `rbtc init`). Contains task envelopes, run logs, optional project-local skills.
- Global: `~/.rebotica/`. Contains run logs (`runs/`), model scorecards, and pending/submitted/dismissed comment cards.

## Documentation

- [Usage](docs/usage.md) — practical guide and integration walkthrough.
- [Output contract](docs/output-contract.md) — v1 envelope wire format.
- [Exit codes](docs/exit-codes.md) — `error.code` table and per-code `error.details` shapes.
- [Release](docs/release.md) — release notes and tag checklist.

The following pre-date the v1 envelope work and may not reflect current behavior. Treat as background:

- [Architecture](docs/architecture.md), [Providers](docs/providers.md), [Skills](docs/skills.md), [Operating Model](docs/operating-model.md), [Governance](docs/governance.md), [Safety Model](docs/safety-model.md), [Self-Healing](docs/self-healing.md), [Roadmap](docs/roadmap.md), [Philosophy](docs/philosophy.md), [Install](docs/install.md).

## License

MIT.
