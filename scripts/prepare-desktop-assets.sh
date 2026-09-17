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

runtime_root="${opus_root}/../runtime"
cef_helper="${OPUS_CEF_HELPER_APP:-${runtime_root}/legacy/1.8.9/client/native/opus-cef-helper/build/opus-webview-helper.app}"
if [[ ! -x "${cef_helper}/Contents/MacOS/opus-webview-helper" ]]; then
  if [[ -x "${runtime_root}/web-surface-v2/scripts/build-w1-helper.sh" ]]; then
    cef_helper="$("${runtime_root}/web-surface-v2/scripts/build-w1-helper.sh" | tail -1)"
  fi
fi
if [[ ! -x "${cef_helper}/Contents/MacOS/opus-webview-helper" ]]; then
  echo "Missing Opus CEF helper app: ${cef_helper}" >&2
  exit 1
fi
helper_destination="${bootstrap_destination}/opus-webview-helper.app"
rm -rf "${helper_destination}"
/usr/bin/ditto "${cef_helper}" "${helper_destination}"
if [[ ! -f "${helper_destination}/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework" ]]; then
  echo "Staged Opus CEF helper is missing Chromium Embedded Framework" >&2
  exit 1
fi
/usr/bin/codesign --verify --deep --strict "${helper_destination}" >/dev/null 2>&1 || {
  echo "Staged Opus CEF helper failed codesign verification" >&2
  exit 1
}
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
