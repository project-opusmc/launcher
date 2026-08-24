#!/usr/bin/env bash
set -euo pipefail

opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
opus_guard="${opus_root}/scripts/opus-process-guard.sh"
opus_bundle="${opus_root}/target/release/bundle/macos/Opus Launcher.app"
opus_destination='/Applications/Opus Launcher.app'
file_only_root="$(cd "${opus_root}/.." && pwd)"

# Ad-hoc signing must never be attempted while macOS is locked.
if [[ -x "${file_only_root}/tools/no-keychain/check-console-unlocked.sh" ]] \
  && ! "${file_only_root}/tools/no-keychain/check-console-unlocked.sh"; then
  echo "Refusing install: macOS console is locked; no password is requested." >&2
  exit 78
fi

"${opus_guard}" assert-idle
test -f "${opus_bundle}/Contents/MacOS/opus-launcher"
test -f "${opus_bundle}/Contents/Info.plist"
"${opus_root}/scripts/assert-file-only-bundle.sh" "${opus_bundle}"
opus_game_stub="${opus_bundle}/Contents/Resources/Minecraft.app/Contents/MacOS/Minecraft"
if [[ ! -f "${opus_game_stub}" ]] \
  || ! strings "${opus_game_stub}" | rg -Fq 'macOS console is locked; unlock before launching Minecraft'; then
  echo "Refusing install: bundled macOS game stub is stale; rebuild the release bundle first." >&2
  exit 1
fi

opus_stage="$(mktemp -d /tmp/opus-launcher-install.XXXXXX)"
trap 'rm -rf "${opus_stage}"' EXIT
cp -R "${opus_bundle}" "${opus_stage}/Opus Launcher.app"
test -f "${opus_stage}/Opus Launcher.app/Contents/MacOS/opus-launcher"
test -f "${opus_stage}/Opus Launcher.app/Contents/Info.plist"
"${opus_root}/scripts/assert-file-only-bundle.sh" "${opus_stage}/Opus Launcher.app"
opus_staged_game_stub="${opus_stage}/Opus Launcher.app/Contents/Resources/Minecraft.app/Contents/MacOS/Minecraft"
if [[ ! -f "${opus_staged_game_stub}" ]] \
  || ! strings "${opus_staged_game_stub}" | rg -Fq 'macOS console is locked; unlock before launching Minecraft'; then
  echo "Refusing install: staged macOS game stub is stale." >&2
  exit 1
fi

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
