#!/usr/bin/env bash
set -euo pipefail

opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runtime_artifact_dir="${OPUS_RUNTIME_ARTIFACT_DIR:-${opus_root}/runtime-artifacts}"
bootstrap_destination="${opus_root}/desktop/src-tauri/resources/bootstrap"
node "${opus_root}/scripts/stage-runtime-artifacts.mjs" \
  "${runtime_artifact_dir}" \
  "${bootstrap_destination}"

brand_destination="${opus_root}/desktop/src-tauri/resources/brand"
mkdir -p "${brand_destination}"
cp "${opus_root}/desktop/public/brand/opus-wordmark-transparent.png" \
  "${brand_destination}/opus-wordmark-transparent.png"
