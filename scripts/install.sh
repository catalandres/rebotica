#!/usr/bin/env sh
set -eu

PREFIX="${1:-$HOME/.local}"
SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
ATELIER_HOME=$(CDPATH= cd -- "$SCRIPT_DIR/.." && pwd)
BIN_DIR="$PREFIX/bin"

cargo build --release --manifest-path "$ATELIER_HOME/Cargo.toml" -p atelier-cli >/dev/null
mkdir -p "$BIN_DIR"

cat > "$BIN_DIR/atelier" <<EOF
#!/usr/bin/env sh
set -eu
export ATELIER_HOME="$ATELIER_HOME"
exec "\$ATELIER_HOME/target/release/atelier" "\$@"
EOF

chmod +x "$BIN_DIR/atelier"

printf 'installed atelier to %s\n' "$BIN_DIR/atelier"
printf 'ensure %s is on PATH\n' "$BIN_DIR"
