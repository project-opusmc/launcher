#!/usr/bin/env bash
set -euo pipefail

opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
runtime_artifact_dir="${OPUS_RUNTIME_ARTIFACT_DIR:-${opus_root}/../runtime/build/runtime}"
if [[ ! -f "${runtime_artifact_dir}/runtime-manifest.json" ]]; then
  runtime_artifact_dir="${opus_root}/runtime-artifacts"
fi
bootstrap_destination="${opus_root}/desktop/src-tauri/resources/bootstrap"
node "${opus_root}/scripts/stage-runtime-artifacts.mjs" \
  "${runtime_artifact_dir}" \
  "${bootstrap_destination}"
# The launcher validates the Forge artifacts by SHA-1 and byte size at launch.
# Verify that the staged Runtime bytes and those compile-time expectations are
# the same before a desktop bundle can capture a mismatched pair.
node "${opus_root}/scripts/verify-runtime-launch-contract.mjs" \
  "${bootstrap_destination}"

# The sole launcher shell must stay non-persistent so WebKit cannot register
# with macOS Keychain despite file-only credential storage. Historical UI
# preview source is deliberately not part of staging or verification.
node -e '
  const fs = require("fs");
  const file = process.argv[1];
  const config = JSON.parse(fs.readFileSync(file, "utf8"));
  const window = config.app?.windows?.[0];
  if (window?.incognito !== true) {
    console.error(`Tauri window must be incognito to keep WebKit out of Keychain: ${file}`);
    process.exit(1);
  }
' "${opus_root}/desktop/src-tauri/tauri.conf.json"
