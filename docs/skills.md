# Skills

Atelier can act as a skills multiplexer.

The near-term model is file-based:

```text
skills/
  local-model-delegation/
    SKILL.md
  local-worker-self-healing/
    SKILL.md
```

Root-agent adapters install those canonical skills into the places each tool expects:

```sh
atelier install claude
atelier install codex
```

Claude also gets slash-command files from `claude/commands`. Codex gets the same canonical skills under `.agents/skills`.

## Why Multiplex Skills

The useful invariant is that policy lives once:

- bounded delegation rules
- allowed and forbidden worker behavior
- retrospective/self-healing workflow
- patch acceptance rules
- provider and model routing vocabulary

Different root tools can consume that policy through different adapters without forking the actual rules.

## Skills Server

Atelier can become a skills server for larger root agents, but the first version should stay file-based.

A future server should expose narrow operations:

- list available skills
- read a skill by id and version
- install a skill into a target adapter
- report compatibility metadata
- expose skills through MCP resources

It should not expose broad filesystem mutation or arbitrary shell execution.

## GitHub

GitHub is not a skills host in the same way. For GitHub, Atelier installs repository governance assets:

```sh
atelier install github
```

Those assets can include workflows, pull request templates, issue templates, and release checklists. They should be copied into the repo so GitHub can run them without depending on local symlinks.

## Future Shape

Possible CLI surface:

```sh
atelier skills list
atelier skills show local-model-delegation
atelier skills install local-model-delegation --target claude
atelier skills install local-model-delegation --target codex
atelier skills serve --mcp
```

That should come after the file-based adapter path proves useful.
