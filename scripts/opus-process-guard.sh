#!/usr/bin/env bash
set -euo pipefail

# Installer operations must not replace a running Opus bundle. Keep matching
# narrow so unrelated Java and desktop processes are never affected.
opus_processes() {
  local opus_scope="${1:-all}"
  # The caller's shell can contain an Opus path in its command line (for
  # example while inspecting a bundle). Exclude this script and all parents
  # before matching so an inspection can never flag itself as a live client.
  opus_ignored_pids=""
  opus_cursor="$$"
  while [[ "${opus_cursor}" =~ ^[0-9]+$ && "${opus_cursor}" -gt 1 ]]; do
    opus_ignored_pids="${opus_ignored_pids} ${opus_cursor} "
    opus_cursor="$(ps -o ppid= -p "${opus_cursor}" | tr -d '[:space:]')"
  done

  opus_launcher_root="${OPUS_HOME:-${HOME}/.opus-launcher}"
  opus_preview_root="${OPUS_UI_PREVIEW_HOME:-${HOME}/.opus-launcher-ui-preview}"
  ps -axo pid=,command= | awk \
    -v ignored="${opus_ignored_pids}" \
    -v scope="${opus_scope}" \
    -v launcher_status="-Dopus.game.statusFile=${opus_launcher_root}/" \
    -v preview_status="-Dopus.game.statusFile=${opus_preview_root}/" '
    {
      line = $0
      sub(/^[[:space:]]*/, "", line)
      pid = line
      sub(/[[:space:]].*$/, "", pid)
      if (index(ignored, " " pid " ") != 0) {
        next
      }
      command = line
      sub(/^[0-9]+[[:space:]]+/, "", command)
      if (command ~ /^awk([[:space:]]|$)/) {
        next
      }
      if (scope == "installed") {
        if (command ~ /^\/Applications\/Opus Launcher\.app\/Contents\/MacOS\/opus-launcher([[:space:]]|$)/ \
            || command ~ /^\/Applications\/Opus Launcher\.app\/Contents\/Resources\/Minecraft\.app\/Contents\/MacOS\/Minecraft([[:space:]]|$)/ \
            || index(command, launcher_status) != 0) {
          print pid "\t" command
        }
        next
      }
      # ps prints bundle paths containing spaces as unquoted command text.
      # Match the full command prefix rather than splitting at the first space.
      if (command ~ /^\/Applications\/Opus Launcher( Preview)?\.app\/Contents\/MacOS\/opus-launcher([[:space:]]|$)/ \
          || command ~ /^\/Applications\/Opus( Launcher( Preview)?)?\.app\/Contents\/Resources\/Minecraft\.app\/Contents\/MacOS\/Minecraft([[:space:]]|$)/ \
          || command ~ /^\/Applications\/Minecraft\.app\/Contents\/MacOS\/Minecraft([[:space:]]|$)/ \
          || index(command, launcher_status) != 0 \
          || index(command, preview_status) != 0 \
          || command ~ /opus\.ui\.preview\.control\.file=/ \
          || command ~ /\.opus-launcher-ui-preview\/game/) {
        print pid "\t" command
      }
    }
  '
}

print_processes() {
  local opus_found
  opus_found="$(opus_processes "${1:-all}")"
  if [[ -z "${opus_found}" ]]; then
    echo "Opus process guard: idle"
    return 0
  fi
  echo "Opus process guard: active launcher processes:"
  printf '%s\n' "${opus_found}"
  return 1
}

stop_processes() {
  local opus_scope="${1:-all}"
  local opus_found opus_pid opus_attempt
  opus_found="$(opus_processes "${opus_scope}")"
  if [[ -z "${opus_found}" ]]; then
    echo "Opus process guard: idle"
    return 0
  fi
  while IFS=$'\t' read -r opus_pid _; do
    kill -TERM "${opus_pid}" 2>/dev/null || true
  done <<< "${opus_found}"
  for opus_attempt in $(seq 1 10); do
    if [[ -z "$(opus_processes "${opus_scope}")" ]]; then
      echo "Opus process guard: stopped"
      return 0
    fi
    sleep 1
  done
  # Only matching Opus processes are escalated after the graceful timeout.
  opus_found="$(opus_processes "${opus_scope}")"
  while IFS=$'\t' read -r opus_pid _; do
    kill -KILL "${opus_pid}" 2>/dev/null || true
  done <<< "${opus_found}"
  for opus_attempt in $(seq 1 5); do
    if [[ -z "$(opus_processes "${opus_scope}")" ]]; then
      echo "Opus process guard: stopped"
      return 0
    fi
    sleep 1
  done
  echo "Opus process guard: a process did not stop:" >&2
  opus_processes "${opus_scope}" >&2
  return 1
}

case "${1:-status}" in
  status)
    print_processes || true
    ;;
  assert-idle)
    if ! print_processes; then
      echo "Refusing to launch another Opus process. Stop the listed process first." >&2
      exit 1
    fi
    ;;
  assert-installed-idle)
    if ! print_processes installed; then
      echo "Refusing to replace a running Opus installation." >&2
      exit 1
    fi
    ;;
  stop)
    stop_processes
    ;;
  stop-installed)
    stop_processes installed
    ;;
  *)
    echo "Usage: $0 {status|assert-idle|assert-installed-idle|stop|stop-installed}" >&2
    exit 64
    ;;
esac
