#!/usr/bin/env bash
set -euo pipefail

opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

target_roots=()
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
  case "${CARGO_TARGET_DIR}" in
    /*) target_roots=("${CARGO_TARGET_DIR}") ;;
    *)
      # Cargo resolves a relative CARGO_TARGET_DIR from the manifest process's
      # working directory. Tauri invokes Cargo from src-tauri, while the
      # default target directory is resolved from the workspace root. Accept
      # both locations so a caller can select an isolated target without
      # making the bundle gate point at a stale or nonexistent path.
      target_roots=(
        "${opus_root}/${CARGO_TARGET_DIR}"
        "${opus_root}/desktop/src-tauri/${CARGO_TARGET_DIR}"
      )
      ;;
  esac
else
  target_roots=("${opus_root}/target")
fi

cd "${opus_root}/desktop"
npm exec -- tauri build --bundles app

bundle=""
for target_root in "${target_roots[@]}"; do
  candidate="${target_root}/release/bundle/macos/Opus Launcher.app"
  if [[ -d "${candidate}" ]]; then
    bundle="${candidate}"
    break
  fi
done
if [[ -z "${bundle}" ]]; then
  echo "Unable to locate the Tauri bundle in any configured target directory:" >&2
  printf '  %s\n' "${target_roots[@]}" >&2
  exit 1
fi

# Tauri signs its executable before the staged CEF/resources tree is finalized.
# Re-seal the complete local development bundle so the artifact that passes the
# release gate is also the exact app that can be installed into /Applications.
if [[ "$(uname -s)" == "Darwin" ]]; then
  /usr/bin/codesign --force --deep --sign - "${bundle}"
  /usr/bin/codesign --verify --deep --strict "${bundle}"
fi

"${opus_root}/scripts/assert-file-only-bundle.sh" "${bundle}"

# Local release builds install the verified app automatically. CI and callers
# that only want a bundle can opt out without changing the default workflow.
if [[ "$(uname -s)" == "Darwin" && "${CI:-}" != "true" && "${OPUS_AUTO_INSTALL:-1}" == "1" ]]; then
  "${opus_root}/scripts/install-opus-launcher.sh" "${bundle}"
fi
