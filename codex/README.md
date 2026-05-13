# Codex Adapter

Atelier installs canonical skills from `skills/` into `.agents/skills`.

Use:

```sh
atelier install codex
```

In restricted environments, stage skills in an explicit directory:

```sh
atelier install codex --target-dir .atelier/adapters/codex/skills
```

This is intentionally file-based for now. A future skills server can expose the same canonical skills through MCP or another root-agent adapter.
