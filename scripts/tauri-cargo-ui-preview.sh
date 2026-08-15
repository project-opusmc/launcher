#!/usr/bin/env bash
set -euo pipefail

# The preview JAR lock and offline launcher must be compiled together. Tauri
# otherwise warns that workspace dependencies prevent its feature injection.
opus_args=()
for opus_arg in "$@"; do
  # Tauri's runner protocol uses --debug; Cargo's debug build is implicit.
  if [[ "$opus_arg" != "--debug" ]]; then
    opus_args+=("$opus_arg")
  fi
done
exec cargo "${opus_args[@]}" --features ui-preview
