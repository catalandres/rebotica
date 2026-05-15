# Skills

Rebotica can act as a skills multiplexer.

The near-term model is file-based:

```text
skills/
  local-model-delegation/
    SKILL.md
  local-worker-self-healing/
    SKILL.md
```

The CLI can inspect those canonical skills:

```sh
rbtc skills list
rbtc skills show local-model-delegation
```

Prime-agent adapters also install canonical skills into the places each tool expects:

```sh
rbtc install claude
rbtc install codex
```

Claude also gets slash-command files from `claude/commands`. Codex gets the same canonical skills under `.agents/skills`.

## Worker Context

Prime can attach skills to individual local-worker invocations:

```sh
rbtc run review --base origin/main --skill local-model-delegation
rbtc run tests crates/rebotica-cli/src/main.rs --skill local-model-delegation
```

Selected skills are included in the worker prompt after the Rebotica system prompt and task envelope, and after the mode contract or project config when that command includes them. They are context only. They cannot override forbidden paths, sensitive paths, task limits, or any Rebotica safety contract.

Project-local skills can live under:

```text
.rebotica/skills/frontend-review.md
.rebotica/skills/hfx-emitter/SKILL.md
```

If a project skill shares an id with a canonical skill, qualify the source:

```sh
rbtc skills show canonical:local-model-delegation
rbtc skills show project:frontend-review
```

Run logs record selected skill metadata in `skills.json`, while `prompt.md` preserves the exact rendered skill text sent to the worker. After the run, Prime can score model performance with `rbtc score` or create a Rebotica product feedback comment card with `rbtc comment-card`.

## Why Multiplex Skills

The useful invariant is that policy lives once:

- bounded delegation rules
- allowed and forbidden worker behavior
- retrospective/self-healing workflow
- patch acceptance rules
- provider and model routing vocabulary

Different Prime tools can consume that policy through different adapters without forking the actual rules.

## Skills Server

Rebotica can become a skills server for larger Prime agents, but the first version should stay file-based.

A future server should expose narrow operations:

- list available skills
- read a skill by id and version
- install a skill into a target adapter
- report compatibility metadata
- expose skills through MCP resources

It should not expose broad filesystem mutation or arbitrary shell execution.

## GitHub

GitHub is not a skills host in the same way. For GitHub, Rebotica installs repository governance assets:

```sh
rbtc install github
```

Those assets can include workflows, pull request templates, issue templates, and release checklists. They should be copied into the repo so GitHub can run them without depending on local symlinks.

## Future Shape

Future CLI surface:

```sh
rbtc skills install local-model-delegation --target claude
rbtc skills install local-model-delegation --target codex
rbtc skills serve --mcp
```

That should come after the file-based adapter path proves useful.
