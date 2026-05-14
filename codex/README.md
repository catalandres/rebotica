# Codex Adapter

Rebotica installs canonical skills from `skills/` into `.agents/skills`.

Use:

```sh
rbtc install codex
```

In restricted environments, stage skills in an explicit directory:

```sh
rbtc install codex --target-dir .rebotica/adapters/codex/skills
```

This is intentionally file-based for now. A future skills server can expose the same canonical skills through MCP or another root-agent adapter.
