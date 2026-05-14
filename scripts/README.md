# Scripts

Rebotica's implementation boundaries live in Rust crates under `crates/`.

This directory keeps install and contributor helper scripts.

- `install.sh`: install a release shim into a user prefix.
- `local-install-smoke.sh`: install into a local prefix and verify the installed `rbtc` shim against a sandbox project without requiring a model provider.
