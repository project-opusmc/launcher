#!/usr/bin/env bash
set -euo pipefail

cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

if [[ "${OPUS_CHECK_OFFICIAL:-${RBW_CHECK_OFFICIAL:-0}}" == "1" ]]; then
  cargo test -p rbw-runtime --test official_metadata -- --ignored
fi

./game/gradlew -p game test prepareBootstrap --warning-mode all
