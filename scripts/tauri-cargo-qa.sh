#!/usr/bin/env bash
set -euo pipefail

# Tauri's workspace-inherited dependency cannot rewrite Cargo features from
# the config file. This runner makes the offline QA flavor a compile-time
# property of the launcher binary rather than a bundle-name-only variant.
opus_args=()
for opus_arg in "$@"; do
  # Tauri's runner protocol uses --debug; Cargo's debug build is implicit.
  if [[ "$opus_arg" != "--debug" ]]; then
    opus_args+=("$opus_arg")
  fi
done
exec cargo "${opus_args[@]}" --features qa-edition
