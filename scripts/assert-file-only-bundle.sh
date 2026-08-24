#!/usr/bin/env bash
set -euo pipefail

# Reject a stale desktop binary that was built while the launcher still used
# the macOS keyring. The source gate alone cannot detect an old installed app.
# This is also the installed-artifact gate for the Core-Mod-first game contract:
# no retired UI payload, custom game title, or extra client JAR may slip into a
# bundle even if the source-level checks passed.
bundle_root="${1:?usage: assert-file-only-bundle.sh <bundle.app>}"
binary="${bundle_root}/Contents/MacOS/opus-launcher"
bootstrap_root="${bundle_root}/Contents/Resources/bootstrap"
game_host_info="${bundle_root}/Contents/Resources/Minecraft.app/Contents/Info.plist"
game_host_binary="${bundle_root}/Contents/Resources/Minecraft.app/Contents/MacOS/Minecraft"

if [[ ! -x "${binary}" ]]; then
  echo "file-only bundle check: missing executable: ${binary}" >&2
  exit 1
fi

bundle_strings="$(strings "${binary}" 2>/dev/null || true)"
if rg -i -q \
  'apple-native-keyring-store|keyring[_-]core|keyring-4\.|macOS Keychain Store|keychainservice' \
  <<<"${bundle_strings}"; then
  echo "file-only bundle check: stale keyring code is present: ${bundle_root}" >&2
  exit 1
fi

linked_frameworks="$(otool -L "${binary}" 2>/dev/null || true)"
if rg -q '/System/Library/Frameworks/Security\.framework' <<<"${linked_frameworks}"; then
  echo "file-only bundle check: launcher executable links Security.framework: ${bundle_root}" >&2
  exit 1
fi

if ! rg -q 'Application Support/opus/auth' <<<"${bundle_strings}"; then
  echo "file-only bundle check: Opus file vault marker is missing: ${bundle_root}" >&2
  exit 1
fi

if ! rg -F -q -- '-Xdock:name=Minecraft' <<<"${bundle_strings}"; then
  echo "file-only bundle check: Minecraft Dock label is missing: ${bundle_root}" >&2
  exit 1
fi

if rg -F -q -- '-Xdock:name=Opus Client' <<<"${bundle_strings}"; then
  echo "file-only bundle check: retired Opus Client Dock label is present: ${bundle_root}" >&2
  exit 1
fi

if [[ ! -d "${bootstrap_root}" ]]; then
  echo "file-only bundle check: missing runtime bootstrap directory: ${bootstrap_root}" >&2
  exit 1
fi

jar_count="$(find "${bootstrap_root}" -maxdepth 1 -type f -name '*.jar' | wc -l | tr -d '[:space:]')"
if [[ "${jar_count}" != "2" ]]; then
  echo "file-only bundle check: expected exactly bootstrap + Core Mod JARs, found ${jar_count}: ${bootstrap_root}" >&2
  exit 1
fi

shopt -s nullglob
bootstrap_candidates=("${bootstrap_root}"/opus-bootstrap-*.jar)
coremod_candidates=("${bootstrap_root}"/opus-runtime-legacy-1.8.9-*.jar)
shopt -u nullglob
if [[ "${#bootstrap_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one Forge bootstrap JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#coremod_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one OPUS Core Mod JAR: ${bootstrap_root}" >&2
  exit 1
fi
bootstrap="${bootstrap_candidates[0]}"
coremod="${coremod_candidates[0]}"

legacy_payload="$(find "${bundle_root}" -type f \( -iname '*cef*' -o -iname '*webview*' -o -iname '*opus-client*' -o -iname '*opus client*' \) -print -quit)"
if [[ -n "${legacy_payload}" ]]; then
  echo "file-only bundle check: retired UI payload is present: ${legacy_payload}" >&2
  exit 1
fi

if ! unzip -tqq "${coremod}" >/dev/null; then
  echo "file-only bundle check: Core Mod is not a readable JAR: ${coremod}" >&2
  exit 1
fi
forbidden_ui_symbols="$(
  unzip -p "${coremod}" '*.class' \
    | strings \
    | rg -i 'WindowTitleTransformer|opus\\.window\\.title|displayGuiScreen|currentScreen|onGuiOpening|registerKeyBinding|keyTyped|mouseClicked|handleMouseInput|org/cef|com/cef' \
    || true
)"
if [[ -n "${forbidden_ui_symbols}" ]]; then
  echo "file-only bundle check: Core Mod contains a retired UI/title/input symbol: ${coremod}" >&2
  exit 1
fi

if ! unzip -tqq "${bootstrap}" >/dev/null; then
  echo "file-only bundle check: Forge bootstrap is not a readable JAR: ${bootstrap}" >&2
  exit 1
fi
bootstrap_manifest="$(unzip -p "${bootstrap}" META-INF/MANIFEST.MF)"
if ! rg -F -q 'Main-Class: org.polydevs.opusmc.bootstrap.ForgeBootstrapMain' <<<"${bootstrap_manifest}"; then
  echo "file-only bundle check: Forge bootstrap has the wrong entry point: ${bootstrap}" >&2
  exit 1
fi
retired_bootstrap_entries="$(
  unzip -Z1 "${bootstrap}" \
    | rg 'org/polydevs/opusmc/bootstrap/(BootstrapArguments|BootstrapMain|TransformingClassLoader)' \
    || true
)"
if [[ -n "${retired_bootstrap_entries}" ]]; then
  echo "file-only bundle check: Forge bootstrap contains a retired loader: ${bootstrap}" >&2
  exit 1
fi
bootstrap_ui_symbols="$(
  unzip -p "${bootstrap}" '*.class' \
    | strings \
    | rg -i 'org/cef/|com/cef/|opus-webview-helper|opus\\.webview|net/minecraft/client/gui/|GuiScreen|KeyBinding|displayGuiScreen|currentScreen|onGuiOpening|registerKeyBinding|keyTyped|mouseClicked|handleMouseInput' \
    || true
)"
if [[ -n "${bootstrap_ui_symbols}" ]]; then
  echo "file-only bundle check: Forge bootstrap contains a retired UI symbol: ${bootstrap}" >&2
  exit 1
fi

if [[ ! -x "${game_host_binary}" || ! -f "${game_host_info}" ]]; then
  echo "file-only bundle check: bundled macOS Java host is incomplete: ${bundle_root}" >&2
  exit 1
fi
if [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleDisplayName' "${game_host_info}")" != "Minecraft" ]] \
  || [[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleName' "${game_host_info}")" != "Minecraft" ]]; then
  echo "file-only bundle check: bundled Java host must display as Minecraft: ${bundle_root}" >&2
  exit 1
fi
game_host_strings="$(strings "${game_host_binary}" 2>/dev/null || true)"
if rg -i -q 'cef|webview' <<<"${game_host_strings}"; then
  echo "file-only bundle check: bundled Java host contains a retired UI marker: ${bundle_root}" >&2
  exit 1
fi

echo "file-only/Core-Mod-first bundle check: passed (${bundle_root})"
