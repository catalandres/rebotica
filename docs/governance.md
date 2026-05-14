# Governance

Rebotica governance has two audiences: people using the tool in project repos, and maintainers evolving the harness.

## User Experience

The normal path should be:

```sh
rbtc init
rbtc health
rbtc review
```

Good defaults matter more than many switches. Project configuration should capture team policy once, so day-to-day use stays short.

## Versioning

Use semantic versioning once releases start:

- Patch: bug fixes, prompt clarifications, docs updates, compatible guard improvements.
- Minor: new commands, new provider options, new config keys with safe defaults.
- Major: breaking CLI flags, config shape changes, run log format changes, or safety model changes.

Until `1.0`, breaking changes are allowed but should be documented in release notes.

## Upgrade Policy

Coworkers should install a tagged version for regular project work. The repo can move faster on `main`, but team adoption should point at known tags.

Each release should include:

- CLI version.
- Config changes.
- Prompt changes that affect behavior.
- Safety model changes.
- Migration notes.
- Verification commands.

## Config Stability

Project `.rebotica.yml` files should remain readable and reviewable. Prefer additive config keys over rewrites.

Secrets must stay in environment variables, not config files.

## Safety Changes

Changes that broaden what local workers can do require explicit review:

- patch application
- filesystem mutation
- shell execution
- worktree automation
- dependency changes
- provider auth behavior

Default behavior should remain advisory unless a command name and task envelope make mutation explicit.

## Release Checklist

Before a tag:

```sh
just verify
just coverage
rbtc health
rbtc smoke --model MODEL_ALIAS_OR_ID
```

Then check:

- README quick start still works.
- `docs/install.md` is current.
- `templates/project.rebotica.yml` matches documented config.
- Prompt contracts match command behavior.
- Run logs do not store secrets.
