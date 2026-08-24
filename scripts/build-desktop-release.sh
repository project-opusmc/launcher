#!/usr/bin/env bash
set -euo pipefail

opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# A superproject checkout keeps the verified Runtime output beside the
# launcher. Preserve the standalone override for CI and external artifact
# lanes, but do not silently fall back to a missing launcher-local directory.
runtime_artifact_dir="${OPUS_RUNTIME_ARTIFACT_DIR:-${opus_root}/../runtime/build/runtime}"
if [[ ! -f "${runtime_artifact_dir}/runtime-manifest.json" ]]; then
  runtime_artifact_dir="${opus_root}/runtime-artifacts"
fi
OPUS_RUNTIME_ARTIFACT_DIR="${runtime_artifact_dir}" \
  "${opus_root}/scripts/prepare-desktop-assets.sh"
cd "${opus_root}/desktop"
exec npm run build
