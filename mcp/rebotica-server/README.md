# Rebotica MCP Server

This is a future Rust MCP bridge for Rebotica.

The MCP layer intentionally comes after the shell bridge. The first working contract is `rbtc`, because it is easy to inspect, easy to log, and usable by any root coordinator.

When implemented, expose only narrow tools:

- `local_model.review_diff`
- `local_model.explain_files`
- `local_model.propose_tests`
- `local_model.propose_patch`
- `local_model.health_check`
- `local_model.score_last_run`

Do not expose broad tools such as:

- `local_model.run_shell`
- `local_model.write_file`
- `local_model.edit_repo`

The MCP server should call the same Rust core policies as the CLI, including provider and model routing.

The current scaffold lives in:

```text
crates/rebotica-mcp
```
