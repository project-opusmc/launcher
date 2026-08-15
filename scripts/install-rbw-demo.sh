#!/usr/bin/env bash
set -euo pipefail

# Install only the isolated Opus Launcher QA bundle. The existing app moves to
# Trash only after the newly built bundle has been staged and validated.
rbw_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rbw_guard="${rbw_root}/scripts/rbw-process-guard.sh"
rbw_bundle="${rbw_root}/target/debug/bundle/macos/Opus Launcher QA.app"
rbw_destination='/Applications/Opus Launcher QA.app'

"${rbw_guard}" assert-idle
test -f "${rbw_bundle}/Contents/MacOS/opus-launcher"
test -f "${rbw_bundle}/Contents/Info.plist"

rbw_stage="$(mktemp -d /tmp/opus-qa-install.XXXXXX)"
trap 'rm -rf "${rbw_stage}"' EXIT
cp -R "${rbw_bundle}" "${rbw_stage}/Opus Launcher QA.app"
test -f "${rbw_stage}/Opus Launcher QA.app/Contents/MacOS/opus-launcher"
test -f "${rbw_stage}/Opus Launcher QA.app/Contents/Info.plist"

codesign --force --deep --sign - "${rbw_stage}/Opus Launcher QA.app"
codesign --verify --deep --strict "${rbw_stage}/Opus Launcher QA.app"

if [[ -d "${rbw_destination}" ]]; then
  rbw_trash="${HOME}/.Trash/Opus Launcher QA.app.$(date +%Y%m%d-%H%M%S)"
  mv "${rbw_destination}" "${rbw_trash}"
  echo "Previous Demo moved to: ${rbw_trash}"
fi

mv "${rbw_stage}/Opus Launcher QA.app" "${rbw_destination}"
rmdir "${rbw_stage}"
trap - EXIT
echo "Installed: ${rbw_destination}"
