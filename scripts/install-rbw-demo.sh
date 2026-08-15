#!/usr/bin/env bash
set -euo pipefail

# Install only the official offline Demo bundle. The existing app moves to
# Trash only after the newly built bundle has been staged and validated.
rbw_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
rbw_guard="${rbw_root}/scripts/rbw-process-guard.sh"
rbw_bundle="${rbw_root}/target/debug/bundle/macos/RBW Client Demo.app"
rbw_destination='/Applications/RBW Client Demo.app'

"${rbw_guard}" assert-idle
test -f "${rbw_bundle}/Contents/MacOS/rbw-desktop"
test -f "${rbw_bundle}/Contents/Info.plist"

rbw_stage="$(mktemp -d /tmp/rbw-demo-install.XXXXXX)"
trap 'rm -rf "${rbw_stage}"' EXIT
cp -R "${rbw_bundle}" "${rbw_stage}/RBW Client Demo.app"
test -f "${rbw_stage}/RBW Client Demo.app/Contents/MacOS/rbw-desktop"
test -f "${rbw_stage}/RBW Client Demo.app/Contents/Info.plist"

if [[ -d "${rbw_destination}" ]]; then
  rbw_trash="${HOME}/.Trash/RBW Client Demo.app.$(date +%Y%m%d-%H%M%S)"
  mv "${rbw_destination}" "${rbw_trash}"
  echo "Previous Demo moved to: ${rbw_trash}"
fi

mv "${rbw_stage}/RBW Client Demo.app" "${rbw_destination}"
rmdir "${rbw_stage}"
trap - EXIT
echo "Installed: ${rbw_destination}"
