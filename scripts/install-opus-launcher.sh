#!/usr/bin/env bash
set -euo pipefail

opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
opus_guard="${opus_root}/scripts/opus-process-guard.sh"
opus_bundle="${opus_root}/target/release/bundle/macos/Opus Launcher.app"
opus_destination='/Applications/Opus Launcher.app'

"${opus_guard}" assert-idle
test -f "${opus_bundle}/Contents/MacOS/opus-launcher"
test -f "${opus_bundle}/Contents/Info.plist"

opus_stage="$(mktemp -d /tmp/opus-launcher-install.XXXXXX)"
trap 'rm -rf "${opus_stage}"' EXIT
cp -R "${opus_bundle}" "${opus_stage}/Opus Launcher.app"
test -f "${opus_stage}/Opus Launcher.app/Contents/MacOS/opus-launcher"
test -f "${opus_stage}/Opus Launcher.app/Contents/Info.plist"

codesign --force --deep --sign - "${opus_stage}/Opus Launcher.app"
codesign --verify --deep --strict "${opus_stage}/Opus Launcher.app"

if [[ -d "${opus_destination}" ]]; then
  opus_trash="${HOME}/.Trash/Opus Launcher.app.$(date +%Y%m%d-%H%M%S)"
  mv "${opus_destination}" "${opus_trash}"
  echo "Previous Opus Launcher moved to: ${opus_trash}"
fi

mv "${opus_stage}/Opus Launcher.app" "${opus_destination}"
rmdir "${opus_stage}"
trap - EXIT
echo "Installed: ${opus_destination}"
