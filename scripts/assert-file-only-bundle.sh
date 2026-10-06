#!/usr/bin/env bash
set -euo pipefail

# Reject a stale desktop binary that was built while the launcher still used
# the macOS keyring. The source gate alone cannot detect an old installed app.
# This is also the installed-artifact gate for the Web Surface V2 game contract:
# no retired web UI payload, custom game title, or extra client JAR may slip
# into a bundle even if the source-level checks passed.
bundle_root="${1:?usage: assert-file-only-bundle.sh <bundle.app>}"
binary="${bundle_root}/Contents/MacOS/opus-launcher"
bootstrap_root="${bundle_root}/Contents/Resources/bootstrap"
game_host_info="${bundle_root}/Contents/Resources/Minecraft.app/Contents/Info.plist"
game_host_binary="${bundle_root}/Contents/Resources/Minecraft.app/Contents/MacOS/Minecraft"

if [[ ! -x "${binary}" ]]; then
  echo "file-only bundle check: missing executable: ${binary}" >&2
  exit 1
fi
if [[ "$(uname -s)" == "Darwin" ]]   && ! /usr/bin/codesign --verify --deep --strict "${bundle_root}" >/dev/null 2>&1; then
  echo "file-only bundle check: application signature is invalid: ${bundle_root}" >&2
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
if [[ "${jar_count}" != "5" ]]; then
  echo "file-only bundle check: expected exactly five managed Runtime JARs, found ${jar_count}: ${bootstrap_root}" >&2
  exit 1
fi

shopt -s nullglob
bootstrap_candidates=("${bootstrap_root}"/opus-bootstrap-*.jar)
coremod_candidates=("${bootstrap_root}"/opus-runtime-legacy-1.8.9-*.jar)
standalone_loader_candidates=("${bootstrap_root}"/opus-standalone-loader-*.jar)
lwjgl_macos_compat_candidates=("${bootstrap_root}"/opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar)
native_ui_candidates=("${bootstrap_root}"/opus-native-ui-1.8.9-0.1.0.jar)
shopt -u nullglob
if [[ "${#bootstrap_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one Forge bootstrap JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#coremod_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one OPUS Core Mod JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#standalone_loader_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one OPUS Standalone Loader JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#lwjgl_macos_compat_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one LWJGL macOS compatibility JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#native_ui_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one Web Surface client JAR: ${bootstrap_root}" >&2
  exit 1
fi
bootstrap="${bootstrap_candidates[0]}"
coremod="${coremod_candidates[0]}"
standalone_loader="${standalone_loader_candidates[0]}"
lwjgl_macos_compat="${lwjgl_macos_compat_candidates[0]}"
native_ui="${native_ui_candidates[0]}"

webview_helper="${bootstrap_root}/opus-webview-helper.app"
webview_binary="${webview_helper}/Contents/MacOS/opus-webview-helper"
cef_framework="${webview_helper}/Contents/Frameworks/Chromium Embedded Framework.framework/Chromium Embedded Framework"
if [[ ! -x "${webview_binary}" || ! -f "${cef_framework}" ]]; then
  echo "file-only bundle check: Web Surface V2 CEF helper is incomplete: ${webview_helper}" >&2
  exit 1
fi
if ! /usr/bin/codesign --verify --deep --strict "${webview_helper}" >/dev/null 2>&1; then
  echo "file-only bundle check: Web Surface V2 CEF helper signature is invalid: ${webview_helper}" >&2
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

if ! unzip -tqq "${standalone_loader}" >/dev/null; then
  echo "file-only bundle check: Standalone Loader is not a readable JAR: ${standalone_loader}" >&2
  exit 1
fi
forbidden_standalone_symbols="$(
  unzip -p "${standalone_loader}" '*.class' \
    | strings \
    | rg -i 'WindowTitleTransformer|opus\\.window\\.title|ClientConfigUi|OpusClientOptionsScreen|org/cef|com/cef' \
    || true
)"
if [[ -n "${forbidden_standalone_symbols}" ]]; then
  echo "file-only bundle check: Standalone Loader contains a retired UI/title symbol: ${standalone_loader}" >&2
  exit 1
fi

if ! unzip -tqq "${lwjgl_macos_compat}" >/dev/null; then
  echo "file-only bundle check: LWJGL macOS compatibility artifact is not a readable JAR: ${lwjgl_macos_compat}" >&2
  exit 1
fi
lwjgl_compat_manifest="$(unzip -p "${lwjgl_macos_compat}" META-INF/MANIFEST.MF)"
if ! rg -F -q 'Sealed: true' <<<"${lwjgl_compat_manifest}"; then
  echo "file-only bundle check: LWJGL macOS compatibility package is not sealed: ${lwjgl_macos_compat}" >&2
  exit 1
fi
for required_entry in \
  org/lwjgl/MacOSXSysImplementation.class \
  org/lwjgl/J2SESysImplementation.class \
  org/lwjgl/DefaultSysImplementation.class \
  org/lwjgl/SysImplementation.class \
  org/lwjgl/opengl/MacOSXDisplay.class \
  org/lwjgl/opengl/MacOSXCanvasPeerInfo.class \
  'org/lwjgl/opengl/MacOSXCanvasPeerInfo$1.class' \
  org/lwjgl/opengl/MacOSXDisplayPeerInfo.class; do
  if ! unzip -Z1 "${lwjgl_macos_compat}" | rg -F "${required_entry}" >/dev/null; then
    echo "file-only bundle check: LWJGL macOS compatibility entry is missing: ${required_entry}" >&2
    exit 1
  fi
done
if ! unzip -Z1 "${lwjgl_macos_compat}" | rg -x 'liblwjgl\.dylib' > /dev/null; then
  echo "file-only bundle check: LWJGL macOS compatibility native is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
lwjgl_native_sha1="$(unzip -p "${lwjgl_macos_compat}" liblwjgl.dylib | shasum -a 1 | awk '{print $1}')"
lwjgl_native_size="$(unzip -p "${lwjgl_macos_compat}" liblwjgl.dylib | wc -c | tr -d '[:space:]')"
# Consume the complete ZIP entry before selecting the first four bytes. A
# short-read command such as od -N4 makes unzip receive SIGPIPE under
# set -o pipefail, turning a valid native into a false bundle failure.
lwjgl_native_magic="$(unzip -p "${lwjgl_macos_compat}" liblwjgl.dylib | xxd -p | awk 'NR == 1 {print substr($0, 1, 8)}')"
if [[ "${lwjgl_native_sha1}" != "5d53a208e2f5bffa1d8d4b3755190fb5cb835022" ]] \
  || [[ "${lwjgl_native_size}" != "838892" ]] \
  || { [[ "${lwjgl_native_magic}" != "cafebabe" ]] && [[ "${lwjgl_native_magic}" != "cffaedfe" ]]; }; then
  echo "file-only bundle check: embedded macOS LWJGL native failed its pinned integrity contract: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if unzip -p "${lwjgl_macos_compat}" org/lwjgl/MacOSXSysImplementation.class \
  | strings \
  | rg -F 'java/awt/Toolkit' >/dev/null; then
  echo "file-only bundle check: LWJGL macOS compatibility class still initializes AWT: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" org/lwjgl/MacOSXSysImplementation.class \
  | strings \
  | rg -F 'com.apple.eio.FileManager' >/dev/null; then
  echo "file-only bundle check: LWJGL macOS compatibility class lost URL-opening support: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" org/lwjgl/opengl/MacOSXDisplay.class \
  | strings \
  | rg -F 'OPUS_MODERN_MACOS_DISPLAY_COMPAT_V1' >/dev/null; then
  echo "file-only bundle check: LWJGL display compatibility patch is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" org/lwjgl/opengl/MacOSXDisplay.class \
  | strings \
  | rg -F 'OPUS_COCOA_LIVE_RESIZE_HANDSHAKE_V10_APPKIT_DRAWABLE_UPDATE' >/dev/null; then
  echo "file-only bundle check: LWJGL Cocoa live-resize handshake is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" org/lwjgl/opengl/MacOSXDisplay.class \
  | strings \
  | rg -F 'opus.lwjgl.macos.liveResizeGuard' >/dev/null; then
  echo "file-only bundle check: LWJGL preview resize-guard opt-out is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -Z1 "${lwjgl_macos_compat}" | rg -x 'libopus_lwjgl_resize_guard\.dylib' > /dev/null; then
  echo "file-only bundle check: Cocoa live-resize guard is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
guard_sha1="$(unzip -p "${lwjgl_macos_compat}" libopus_lwjgl_resize_guard.dylib | shasum -a 1 | awk '{print $1}')"
guard_size="$(unzip -p "${lwjgl_macos_compat}" libopus_lwjgl_resize_guard.dylib | wc -c | tr -d '[:space:]')"
if [[ "${guard_sha1}" != "6005c797463267ed1089231d3b6fb88f3bc0b0fd" ]] \
  || [[ "${guard_size}" != "21184" ]]; then
  echo "file-only bundle check: Cocoa live-resize guard failed its integrity contract: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" libopus_lwjgl_resize_guard.dylib \
  | strings \
  | rg -F 'OPUS_COCOA_LIVE_RESIZE_GUARD_V14_RENDER_THREAD_DRAWABLE_OWNERSHIP' >/dev/null; then
  echo "file-only bundle check: Cocoa live-resize guard marker is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" org/lwjgl/opengl/MacOSXCanvasPeerInfo.class \
  | strings \
  | rg -F 'OPUS_MODERN_MACOS_PIXEL_FORMAT_COMPAT_V2' >/dev/null; then
  echo "file-only bundle check: LWJGL pixel-format compatibility patch is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -tqq "${native_ui}" >/dev/null; then
  echo "file-only bundle check: Web Surface client is not a readable JAR: ${native_ui}" >&2
  exit 1
fi
for required_entry in \
  org/polydevs/opusmc/client/OpusClientMod.class \
  org/polydevs/opusmc/client/OpusClientScreen.class \
  org/polydevs/opusmc/client/embed/OpusWebViewClient.class \
  org/polydevs/opusmc/client/embed/OpusWebTextureSurface.class \
  org/polydevs/opusmc/websurface/OpusDisplayMetrics.class \
  org/polydevs/opusmc/websurface/OpusInputMapper.class \
  opusui/index.html; do
  # Do not use rg -q here: with pipefail enabled it exits before unzip has
  # finished writing, turning a valid match into a false broken-pipe failure.
  if ! unzip -Z1 "${native_ui}" | rg -F "${required_entry}" >/dev/null; then
    echo "file-only bundle check: Web Surface client entry is missing: ${required_entry}" >&2
    exit 1
  fi
done
if unzip -Z1 "${native_ui}" | rg -F 'org/polydevs/opusmc/nativeui/OpusNativeUiMod.class' >/dev/null; then
  echo "file-only bundle check: retired Elementa screen owner is still present: ${native_ui}" >&2
  exit 1
fi
for retired_ui in \
  opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar \
  opus-elementa-762.jar \
  opus-universalcraft-1.8.9-forge-516.jar; do
  if [[ -e "${bootstrap_root}/${retired_ui}" ]]; then
    echo "file-only bundle check: retired UI dependency returned: ${retired_ui}" >&2
    exit 1
  fi
done

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

echo "file-only/Web-Surface bundle check: passed (${bundle_root})"
