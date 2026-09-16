#!/usr/bin/env bash
set -euo pipefail

# Reject a stale desktop binary that was built while the launcher still used
# the macOS keyring. The source gate alone cannot detect an old installed app.
# This is also the installed-artifact gate for the native UI game contract:
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
if [[ "${jar_count}" != "7" ]]; then
  echo "file-only bundle check: expected exactly seven managed Runtime JARs, found ${jar_count}: ${bootstrap_root}" >&2
  exit 1
fi

shopt -s nullglob
bootstrap_candidates=("${bootstrap_root}"/opus-bootstrap-*.jar)
coremod_candidates=("${bootstrap_root}"/opus-runtime-legacy-1.8.9-*.jar)
lwjgl_macos_compat_candidates=("${bootstrap_root}"/opus-lwjgl-macos-compat-2.9.2-nightly-20140822.jar)
native_ui_candidates=("${bootstrap_root}"/opus-native-ui-1.8.9-0.1.0.jar)
opusconfig_candidates=("${bootstrap_root}"/opus-oneconfig-1.8.9-forge-0.2.2-alpha228-full.jar)
elementa_candidates=("${bootstrap_root}"/opus-elementa-762.jar)
universalcraft_candidates=("${bootstrap_root}"/opus-universalcraft-1.8.9-forge-516.jar)
shopt -u nullglob
if [[ "${#bootstrap_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one Forge bootstrap JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#coremod_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one OPUS Core Mod JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#lwjgl_macos_compat_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one LWJGL macOS compatibility JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#native_ui_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one native UI JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#opusconfig_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one OpusConfig JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#elementa_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one Elementa JAR: ${bootstrap_root}" >&2
  exit 1
fi
if [[ "${#universalcraft_candidates[@]}" -ne 1 ]]; then
  echo "file-only bundle check: expected exactly one UniversalCraft JAR: ${bootstrap_root}" >&2
  exit 1
fi
bootstrap="${bootstrap_candidates[0]}"
coremod="${coremod_candidates[0]}"
lwjgl_macos_compat="${lwjgl_macos_compat_candidates[0]}"
native_ui="${native_ui_candidates[0]}"
opusconfig="${opusconfig_candidates[0]}"
elementa="${elementa_candidates[0]}"
universalcraft="${universalcraft_candidates[0]}"

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
if [[ "${lwjgl_native_sha1}" != "9ade823779dae4be29556dc39ed28b42ba5508e1" ]] \
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
if [[ "${guard_sha1}" != "b80da68bc3de6c8823bcfdee42d660a90246a09f" ]] \
  || [[ "${guard_size}" != "15696" ]]; then
  echo "file-only bundle check: Cocoa live-resize guard failed its integrity contract: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" libopus_lwjgl_resize_guard.dylib \
  | strings \
  | rg -F 'OPUS_COCOA_LIVE_RESIZE_GUARD_V10_MAIN_QUEUE_INSTALL' >/dev/null; then
  echo "file-only bundle check: Cocoa live-resize guard marker is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
if ! unzip -p "${lwjgl_macos_compat}" org/lwjgl/opengl/MacOSXCanvasPeerInfo.class \
  | strings \
  | rg -F 'OPUS_MODERN_MACOS_PIXEL_FORMAT_COMPAT_V2' >/dev/null; then
  echo "file-only bundle check: LWJGL pixel-format compatibility patch is missing: ${lwjgl_macos_compat}" >&2
  exit 1
fi
for managed_ui_jar in "${native_ui}" "${opusconfig}" "${elementa}" "${universalcraft}"; do
  if ! unzip -tqq "${managed_ui_jar}" >/dev/null; then
    echo "file-only bundle check: managed UI dependency is not a readable JAR: ${managed_ui_jar}" >&2
    exit 1
  fi
done
for required_entry in \
  org/polydevs/opusmc/nativeui/OpusNativeUiMod.class \
  org/polydevs/opusmc/nativeui/OpusElementaShellScreen.class \
  org/polydevs/opusmc/nativeui/OpusConfigModule.class; do
  # Do not use rg -q here: with pipefail enabled it exits before unzip has
  # finished writing, turning a valid match into a false broken-pipe failure.
  if ! unzip -Z1 "${native_ui}" | rg -F "${required_entry}" >/dev/null; then
    echo "file-only bundle check: native UI entry is missing: ${required_entry}" >&2
    exit 1
  fi
done
for required_entry in \
  assets/oneconfig/icons/OpusConfigHead.svg \
  cc/polyfrost/oneconfig/gui/OneConfigGui.class \
  cc/polyfrost/oneconfig/gui/SideBar.class; do
  if ! unzip -Z1 "${opusconfig}" | rg -F "${required_entry}" >/dev/null; then
    echo "file-only bundle check: OpusConfig entry is missing: ${required_entry}" >&2
    exit 1
  fi
done
if ! unzip -Z1 "${elementa}" | rg -F 'gg/essential/elementa/components/UIBlock.class' >/dev/null; then
  echo "file-only bundle check: Elementa UI classes are missing: ${elementa}" >&2
  exit 1
fi
if ! unzip -Z1 "${universalcraft}" | rg -F 'gg/essential/universal/UMatrixStack.class' >/dev/null; then
  echo "file-only bundle check: UniversalCraft classes are missing: ${universalcraft}" >&2
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

echo "file-only/native-UI bundle check: passed (${bundle_root})"
