#!/usr/bin/env bash
set -euo pipefail

# Tauri's workspace-inherited dependency cannot rewrite Cargo features from
# the config file. This runner makes the offline QA flavor a compile-time
# property of the launcher binary rather than a bundle-name-only variant.
rbw_args=()
for rbw_arg in "$@"; do
  # Tauri's runner protocol uses --debug; Cargo's debug build is implicit.
  if [[ "$rbw_arg" != "--debug" ]]; then
    rbw_args+=("$rbw_arg")
  fi
done
exec cargo "${rbw_args[@]}" --features qa-edition
