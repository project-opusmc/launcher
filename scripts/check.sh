#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo clippy -p opus-launcher --all-targets --features qa-edition,ui-preview -- -D warnings
cargo test --workspace
cargo test -p opus-launcher --features qa-edition
npm --prefix desktop run check
npm --prefix desktop run build

if [[ "${OPUS_CHECK_OFFICIAL:-0}" == "1" ]]; then
  cargo test -p opus-engine --test official_metadata -- --ignored
fi
