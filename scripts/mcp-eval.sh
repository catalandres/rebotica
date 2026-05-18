#!/bin/sh
# scripts/mcp-eval.sh
#
# Measures whether Claude Code (or Codex, with --client codex) invokes
# the right Rebotica MCP tool unprompted when given a natural-language
# prompt. This is the harness for Success Criterion 1 of epic #42.
#
# Methodology
# -----------
# Three seeded prompts (one per tool class) are each fired three times
# against `claude --print` with the Rebotica MCP server registered via
# --mcp-config (no .claude/settings.json modification needed). The
# stream-json output is scanned for `tool_use` events; a session
# "passes" if the expected tool was invoked at least once.
#
# The MCP server runs in REBOTICA_MCP_OFFLINE_PROBE=1 mode by default,
# so no real provider tokens are spent on the apprentice side. Real
# Claude tokens ARE spent on the Prime side (one short prompt per
# session).
#
# Output
# ------
# Per-session pass/fail breakdown to stderr; final tally as the last
# line on stdout: `RESULT: N/9 passed`.
#
# Fallback rule per epic #42 Success Criterion 1:
#   - ≥7/9: ship v0.3 as planned
#   - 5–6/9: ship with known-issue note, iterate tool descriptions in v0.3.x
#   - ≤4/9: block tag and reconsider the bet
#
# Requirements
# ------------
#   - `claude` CLI on PATH (Claude Code, v2.x or newer with --print).
#   - `rbtc` CLI on PATH (or REBOTICA_BIN env var set to its absolute path).
#   - `jq` on PATH for stream-json parsing.
#   - ANTHROPIC_API_KEY (or whatever auth `claude` is configured with).
#
# Environment knobs
# -----------------
#   REBOTICA_BIN          : Path to rbtc (default: looked up via `command -v rbtc`).
#   REBOTICA_EVAL_MODEL   : Claude model to pass via --model (default: claude default).
#   REBOTICA_EVAL_RUNS    : Runs per prompt (default: 3, total 3 × prompts).
#   REBOTICA_EVAL_LIVE    : Set to 1 to disable offline probe (uses real LM Studio).
#   REBOTICA_EVAL_VERBOSE : Set to 1 to keep per-session stream-json logs in /tmp.

set -u

# --- Prerequisites --------------------------------------------------------

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    printf 'mcp-eval: required command not found on PATH: %s\n' "$1" >&2
    exit 2
  fi
}

require claude
require jq

RBTC=${REBOTICA_BIN:-$(command -v rbtc || true)}
if [ -z "$RBTC" ] || [ ! -x "$RBTC" ]; then
  printf 'mcp-eval: rbtc not found. Set REBOTICA_BIN or add rbtc to PATH.\n' >&2
  exit 2
fi

# --- Configuration --------------------------------------------------------

RUNS=${REBOTICA_EVAL_RUNS:-3}
MODEL_ARG=""
if [ -n "${REBOTICA_EVAL_MODEL:-}" ]; then
  MODEL_ARG="--model ${REBOTICA_EVAL_MODEL}"
fi

OFFLINE_MODE=1
if [ "${REBOTICA_EVAL_LIVE:-0}" = "1" ]; then
  OFFLINE_MODE=0
fi

# Build the --mcp-config JSON inline. The MCP server inherits the
# parent process's env, so REBOTICA_MCP_OFFLINE_PROBE flows through
# when we set it before invoking claude.
MCP_CONFIG=$(cat <<JSON
{"mcpServers":{"rebotica":{"command":"$RBTC","args":["mcp","serve"]}}}
JSON
)

# Limit Claude to only invoking our MCP tools so the eval measures the
# right thing. Built-in tools (Read/Bash/...) stay enabled by default,
# but the matcher here is what counts as a "tool used" pass.
ALLOWED_TOOLS="mcp__rebotica__review_diff mcp__rebotica__propose_tests mcp__rebotica__explain_files mcp__rebotica__health_check"

# --- Seeded prompts -------------------------------------------------------
#
# Each prompt has: label, expected tool name, prompt text.

PROMPT_REVIEW_LABEL="diff-review"
PROMPT_REVIEW_TOOL="mcp__rebotica__review_diff"
PROMPT_REVIEW_TEXT="Take a look at my latest changes and tell me what to clean up."

PROMPT_TESTS_LABEL="test-proposal"
PROMPT_TESTS_TOOL="mcp__rebotica__propose_tests"
PROMPT_TESTS_TEXT="I just refactored crates/rebotica-core/src/output/envelope.rs. What tests would catch regressions here?"

PROMPT_EXPLAIN_LABEL="file-explain"
PROMPT_EXPLAIN_TOOL="mcp__rebotica__explain_files"
PROMPT_EXPLAIN_TEXT="I'm new to this code. What does crates/rebotica-cli/src/main.rs do at a high level?"

# --- Per-session runner ---------------------------------------------------

# Run one session, return 0 on pass (expected tool fired), 1 on fail.
# Streams claude output to a temp file so we can grep for tool_use events.
run_session() {
  label=$1
  expected_tool=$2
  prompt_text=$3
  log=$(mktemp -t mcp-eval-${label}-XXXXXX.jsonl)

  env_args=""
  if [ "$OFFLINE_MODE" = "1" ]; then
    env_args="REBOTICA_MCP_OFFLINE_PROBE=1"
  fi

  # shellcheck disable=SC2086
  env $env_args claude \
    --print \
    --output-format stream-json \
    --include-partial-messages \
    --verbose \
    --mcp-config "$MCP_CONFIG" \
    $MODEL_ARG \
    --dangerously-skip-permissions \
    "$prompt_text" \
    > "$log" 2>/dev/null

  # Each stream-json line is a JSON object. Tool uses appear inside
  # assistant messages: .message.content[] with .type == "tool_use".
  invoked=$(jq -r '
    select(.type == "assistant")
    | .message.content[]?
    | select(.type == "tool_use")
    | .name
  ' "$log" 2>/dev/null | sort -u)

  result=1
  matched_tool=""
  for tool in $invoked; do
    if [ "$tool" = "$expected_tool" ]; then
      result=0
      matched_tool=$tool
      break
    fi
  done

  if [ "$result" = "0" ]; then
    printf '  ✓ %s → %s\n' "$label" "$matched_tool" >&2
  else
    if [ -z "$invoked" ]; then
      printf '  ✗ %s (no MCP tools invoked)\n' "$label" >&2
    else
      printf '  ✗ %s (invoked: %s; expected: %s)\n' \
        "$label" "$(echo "$invoked" | tr '\n' ',' | sed 's/,$//')" "$expected_tool" >&2
    fi
  fi

  if [ "${REBOTICA_EVAL_VERBOSE:-0}" = "1" ]; then
    printf '    log: %s\n' "$log" >&2
  else
    rm -f "$log"
  fi
  return $result
}

# --- Drive 3 × N sessions -------------------------------------------------

pass=0
total=0

printf 'mcp-eval: %d runs per prompt; offline_probe=%s\n' "$RUNS" "$OFFLINE_MODE" >&2

for prompt in REVIEW TESTS EXPLAIN; do
  eval "label=\$PROMPT_${prompt}_LABEL"
  eval "tool=\$PROMPT_${prompt}_TOOL"
  eval "text=\$PROMPT_${prompt}_TEXT"
  printf '\nprompt: %s (expected: %s)\n' "$label" "$tool" >&2

  i=0
  while [ "$i" -lt "$RUNS" ]; do
    i=$((i + 1))
    total=$((total + 1))
    if run_session "$label#$i" "$tool" "$text"; then
      pass=$((pass + 1))
    fi
  done
done

printf '\n' >&2
printf 'RESULT: %d/%d passed\n' "$pass" "$total"

if [ "$pass" -ge 7 ]; then
  exit 0
elif [ "$pass" -ge 5 ]; then
  exit 1
else
  exit 2
fi
