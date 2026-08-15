#!/usr/bin/env bash
set -euo pipefail

# The preview JAR lock and offline launcher must be compiled together. Tauri
# otherwise warns that workspace dependencies prevent its feature injection.
rbw_args=()
for rbw_arg in "$@"; do
  # Tauri's runner protocol uses --debug; Cargo's debug build is implicit.
  if [[ "$rbw_arg" != "--debug" ]]; then
    rbw_args+=("$rbw_arg")
  fi
done
exec cargo "${rbw_args[@]}" --features ui-preview
