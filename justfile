prefix := env_var_or_default("PREFIX", env_var("HOME") + "/.local")
local_prefix := env_var_or_default("REBOTICA_LOCAL_PREFIX", justfile_directory() + "/target/local-install/prefix")

default:
    @just --list

build:
    cargo build -p rebotica-cli

release:
    cargo build --release -p rebotica-cli

install prefix=prefix:
    scripts/install.sh "{{prefix}}"

install-smoke prefix=local_prefix:
    scripts/local-install-smoke.sh "{{prefix}}"

verify:
    cargo fmt --all -- --check
    cargo test --workspace
    cargo build --workspace
    bin/rbtc help >/dev/null

release-check:
    just verify
    just coverage
    just install-smoke

coverage:
    #!/usr/bin/env sh
    set -eu
    command -v llvm-profdata >/dev/null || {
      printf 'llvm-profdata is required for coverage\n' >&2
      exit 1
    }
    command -v llvm-cov >/dev/null || {
      printf 'llvm-cov is required for coverage\n' >&2
      exit 1
    }

    coverage_dir="${COVERAGE_DIR:-target/coverage}"
    case "$coverage_dir" in
      /*) ;;
      *) coverage_dir="$(pwd)/$coverage_dir" ;;
    esac
    run_dir="$coverage_dir/run-$(date +%Y%m%d-%H%M%S)-$$"
    mkdir -p "$run_dir/profraw"

    export CARGO_TARGET_DIR="$run_dir/target"
    export RUSTFLAGS="${RUSTFLAGS:-} -Cinstrument-coverage"
    export LLVM_PROFILE_FILE="$run_dir/profraw/rebotica-%p-%m.profraw"
    cargo test --workspace

    llvm-profdata merge -sparse "$run_dir"/profraw/*.profraw -o "$run_dir/rebotica.profdata"

    objects_tmp="$run_dir/objects.tmp"
    objects="$run_dir/objects"
    find "$CARGO_TARGET_DIR/debug/deps" -maxdepth 1 -type f \
      \( -name 'rbtc-*' \
      -o -name 'rebotica_core-*' \
      -o -name 'rebotica_git-*' \
      -o -name 'rebotica_guard-*' \
      -o -name 'rebotica_mcp-*' \
      -o -name 'rebotica_provider-*' \
      -o -name 'rebotica_runlog-*' \) \
      -print > "$objects_tmp"
    while IFS= read -r candidate; do
      if [ -x "$candidate" ]; then
        printf '%s\n' "$candidate"
      fi
    done < "$objects_tmp" > "$objects"

    first="$(sed -n '1p' "$objects")"
    if [ -z "$first" ]; then
      printf 'no instrumented test binaries found under %s\n' "$CARGO_TARGET_DIR/debug/deps" >&2
      exit 1
    fi

    set -- "$first"
    rest="$run_dir/objects.rest"
    sed '1d' "$objects" > "$rest"
    while IFS= read -r object; do
      set -- "$@" --object "$object"
    done < "$rest"

    llvm-cov report \
      --instr-profile="$run_dir/rebotica.profdata" \
      --ignore-filename-regex='/.cargo/registry|/rustc/|rustlib/src/rust|target/' \
      "$@" > "$coverage_dir/coverage.txt"
    cat "$coverage_dir/coverage.txt"
    printf '\ncoverage report: %s\n' "$coverage_dir/coverage.txt"

clean:
    rm -rf target
