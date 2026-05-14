#!/usr/bin/env sh
set -eu

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REBOTICA_HOME=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
PREFIX="${1:-${REBOTICA_LOCAL_PREFIX:-$REBOTICA_HOME/target/local-install/prefix}}"
STAMP=$(date +%Y%m%d-%H%M%S)
SANDBOX="${REBOTICA_LOCAL_SANDBOX:-$REBOTICA_HOME/target/local-install/sandbox-$STAMP-$$}"
RBTC="$PREFIX/bin/rbtc"

printf 'installing local Rebotica shim into %s\n' "$PREFIX"
"$REBOTICA_HOME/scripts/install.sh" "$PREFIX"

if [ ! -x "$RBTC" ]; then
  printf 'expected executable not found: %s\n' "$RBTC" >&2
  exit 1
fi

mkdir -p "$SANDBOX/project"

printf 'running installed CLI checks\n'
"$RBTC" --version >/dev/null
"$RBTC" help >/dev/null

(
  cd "$SANDBOX/project"
  "$RBTC" init >/dev/null
  test -f .rebotica.yml
  test -d .rebotica/tasks
  test -d .rebotica/runs
  test -f .rebotica/.gitignore

  "$RBTC" providers --json >/dev/null
  "$RBTC" models --configured-only >/dev/null
  "$RBTC" doctor --json >/dev/null

  "$RBTC" install codex --copy --target-dir .rebotica/adapters/codex/skills >/dev/null
  "$RBTC" install claude --copy >/dev/null
  "$RBTC" install github >/dev/null

  test -d .rebotica/adapters/codex/skills
  test -d .claude/commands
  test -d .claude/skills
  test -d .github
)

cat <<EOF
local install smoke passed

installed:
  $RBTC

sandbox project:
  $SANDBOX/project

try it:
  export PATH="$PREFIX/bin:\$PATH"
  cd "$SANDBOX/project"
  rbtc doctor
EOF
