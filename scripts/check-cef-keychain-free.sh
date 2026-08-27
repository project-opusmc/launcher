#!/usr/bin/env bash
set -euo pipefail

# Historical CEF-only gate. The canonical production path is now
# Forge + OptiFine + the native Elementa/OpusConfig UI set, so this script must
# never start a web helper, observe securityd, or exercise a credential store.
echo "Retired CEF gate: disabled by the native-UI runtime contract; no helper was launched." >&2
exit 78

if [[ "$(uname -s)" != "Darwin" ]]; then
  echo "CEF Keychain gate: skipped (macOS only)"
  exit 0
fi

launcher_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
repo_root="$(cd "${launcher_root}/.." && pwd)"
if [[ -x "${repo_root}/tools/no-keychain/check-console-unlocked.sh" ]] \
  && ! "${repo_root}/tools/no-keychain/check-console-unlocked.sh"; then
  echo "CEF Keychain gate: console is locked; refusing to start CEF without requesting a password" >&2
  exit 78
fi
if [[ -x "${repo_root}/tools/no-keychain/bin/security" ]]; then
  export PATH="${repo_root}/tools/no-keychain/bin:${PATH}"
fi
runtime_client="${OPUS_RUNTIME_CLIENT_DIR:-${launcher_root}/../runtime/legacy/1.8.9/client}"
source_helper_app="${OPUS_CEF_HELPER_APP:-${runtime_client}/native/opus-cef-helper/build/opus-webview-helper.app}"

tmp_dir="$(mktemp -d "${TMPDIR:-/tmp}/opus-cef-gate.XXXXXX")"
trace_file="${tmp_dir}/securityd.log"
test_file="${tmp_dir}/harness.log"
pid_file="${tmp_dir}/helper-pids"
trace_pid=""
test_pid=""

cleanup() {
  if [[ -n "${trace_pid}" ]]; then
    kill "${trace_pid}" 2>/dev/null || true
  fi
  if [[ -n "${test_pid}" ]]; then
    kill "${test_pid}" 2>/dev/null || true
  fi
  rm -rf "${tmp_dir}"
}
trap cleanup EXIT INT TERM

# A release resource is nested under the Tauri .app and Chromium can trap in
# that layout before the browser process starts. Mirror the launcher's runtime
# materialization step so this gate tests the packaged helper bits without
# reproducing the outer-bundle crash.
helper_app="${source_helper_app}"
if [[ "${source_helper_app}" == *".app/Contents/Resources/"* ]]; then
  helper_app="${tmp_dir}/opus-webview-helper.app"
  /usr/bin/ditto "${source_helper_app}" "${helper_app}"
fi
helper_binary="${helper_app}/Contents/MacOS/opus-webview-helper"
guard_binary="${helper_app}/Contents/Frameworks/opus-keychain-guard.dylib"

[[ -x "${helper_binary}" ]] || {
  echo "CEF Keychain gate: missing helper ${helper_binary}" >&2
  exit 1
}
[[ -f "${guard_binary}" ]] || {
  echo "CEF Keychain gate: missing guard ${guard_binary}" >&2
  exit 1
}

# Capture both sides of the IPC boundary. The helper-side records catch a
# direct Security.framework call; the daemon-side records catch a request that
# crossed into securityd/secd/securityd_system. The harness output alone is
# not evidence that the system credential services were untouched.
trace_predicate='process == "securityd" OR process == "secd" OR process == "securityd_system" OR process CONTAINS[c] "opus-webview-helper" OR composedMessage CONTAINS[c] "SecKeychain" OR composedMessage CONTAINS[c] "SecItem" OR composedMessage CONTAINS[c] "decodeDb" OR composedMessage CONTAINS[c] "authenticateDb" OR composedMessage CONTAINS[c] "unlockDb" OR composedMessage CONTAINS[c] "DBOpen" OR composedMessage CONTAINS[c] "login.keychain"'
/usr/bin/log stream --style compact --level debug --timeout 180 \
  --predicate "${trace_predicate}" >"${trace_file}" 2>&1 &
trace_pid=$!

(
  cd "${runtime_client}"
  OPUS_CEF_HELPER_APP="${helper_app}" ./gradlew --no-daemon harnessWebView
) >"${test_file}" 2>&1 &
test_pid=$!

while kill -0 "${test_pid}" 2>/dev/null; do
  ps -axo pid=,command= | awk 'tolower($0) ~ /opus-webview-helper/ {print $1}' >>"${pid_file}"
  sleep 0.2
done

set +e
wait "${test_pid}"
test_status=$?
set -e
sleep 1
kill "${trace_pid}" 2>/dev/null || true
wait "${trace_pid}" 2>/dev/null || true
trace_pid=""

if [[ "${test_status}" -ne 0 ]]; then
  cat "${test_file}" >&2
  echo "CEF Keychain gate: CEF harness failed" >&2
  exit "${test_status}"
fi

sort -nu "${pid_file}" -o "${pid_file}" 2>/dev/null || true
keychain_pattern='keychain|secitem|seckeychain|trustsettings|kcdb|unlockdb|authenticatedb|decodeDb|system keychain|dbopen'
for helper_pid in $(cat "${pid_file}" 2>/dev/null || true); do
  # A helper-originated record may identify the process as `name[pid:tid]`,
  # while a security daemon record identifies its client as `pid: N`.
  helper_records="$(rg -i "opus-webview-helper\\[${helper_pid}:|pid:[[:space:]]*${helper_pid}([ ,)]|$)" \
    "${trace_file}" || true)"
  matches="$(printf '%s\n' "${helper_records}" | rg -i "${keychain_pattern}" || true)"
  if [[ -n "${matches}" ]]; then
    echo "CEF Keychain gate: helper PID ${helper_pid} touched a credential path" >&2
    printf '%s\n' "${matches}" >&2
    exit 1
  fi
done

if rg -i 'os_crypt|encryption is not available|keychain lookup failed' "${test_file}"; then
  echo "CEF Keychain gate: harness reported a credential-store access" >&2
  exit 1
fi

echo "CEF Keychain gate: passed (CEF harness + securityd trace)"
