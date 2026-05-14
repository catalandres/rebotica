#!/usr/bin/env sh
set -eu

PREFIX="${1:-$HOME/.local}"
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
REBOTICA_HOME=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
BIN_DIR="$PREFIX/bin"

cargo build --release --manifest-path "$REBOTICA_HOME/Cargo.toml" -p rebotica-cli >/dev/null
mkdir -p "$BIN_DIR"

cat > "$BIN_DIR/rbtc" <<EOF
#!/usr/bin/env sh
set -eu
export REBOTICA_HOME="$REBOTICA_HOME"
exec "\$REBOTICA_HOME/target/release/rbtc" "\$@"
EOF

chmod +x "$BIN_DIR/rbtc"

printf 'installed rbtc to %s\n' "$BIN_DIR/rbtc"
printf 'ensure %s is on PATH\n' "$BIN_DIR"
