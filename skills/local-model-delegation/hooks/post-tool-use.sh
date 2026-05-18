#!/bin/sh
# Claude Code PostToolUse hook for Rebotica MCP tool calls.
#
# When matched against `mcp__rebotica__.*`, this hook reads the tool
# response from stdin, extracts the apprentice's run_id, and records a
# placeholder `unscored` disposition in the ledger so Prime can upgrade
# it later with an explicit `accept` / `reject` / `edit_then_use`.
#
# Soft-fails: a missing run_id, an unparseable response, or a missing
# rbtc binary should not block the calling tool. Logs go to stderr.
#
# Requires: jq, rbtc (on PATH).

set -u

payload=$(cat)

if ! command -v jq >/dev/null 2>&1; then
  echo "rebotica hook: jq not found on PATH; skipping disposition recording" >&2
  exit 0
fi

# The MCP tool response wraps the apprentice's JSON in
# `tool_response.content[0].text` as a stringified payload.
inner=$(printf '%s' "$payload" | jq -r '.tool_response.content[0].text // ""' 2>/dev/null)
[ -z "$inner" ] && exit 0

run_id=$(printf '%s' "$inner" | jq -r 'fromjson? | .run_id // empty' 2>/dev/null)
[ -z "$run_id" ] && exit 0

if ! command -v rbtc >/dev/null 2>&1; then
  echo "rebotica hook: rbtc not found on PATH; cannot record disposition for $run_id" >&2
  exit 0
fi

rbtc score "$run_id" --disposition unscored >/dev/null 2>&1 || true
