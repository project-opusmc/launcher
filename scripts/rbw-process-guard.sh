#!/usr/bin/env bash
set -euo pipefail

# Opus UI work must never create a second game/launcher instance. Keep this
# narrow: it recognizes only processes owned by Opus's QA/preview runtime.
rbw_processes() {
  # The caller's shell can contain an Opus path in its command line (for
  # example while inspecting a bundle). Exclude this script and all parents
  # before matching so an inspection can never flag itself as a live client.
  rbw_ignored_pids=""
  rbw_cursor="$$"
  while [[ "${rbw_cursor}" =~ ^[0-9]+$ && "${rbw_cursor}" -gt 1 ]]; do
    rbw_ignored_pids="${rbw_ignored_pids} ${rbw_cursor} "
    rbw_cursor="$(ps -o ppid= -p "${rbw_cursor}" | tr -d '[:space:]')"
  done

  ps -axo pid=,command= | awk -v ignored="${rbw_ignored_pids}" '
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
      if (command ~ /\/Applications\/(Opus Launcher|RBW Client)( QA| Demo)?\.app\/Contents\/MacOS\/(opus-launcher|rbw-desktop)/ \
          || command ~ /rbw\.ui\.preview\.control\.file=/ \
          || command ~ /\.opus-launcher-ui-preview\/game/ \
          || command ~ /\.rbw-client-ui-preview\/game/) {
        print pid "\t" command
      }
    }
  '
}

print_processes() {
  local rbw_found
  rbw_found="$(rbw_processes)"
  if [[ -z "${rbw_found}" ]]; then
    echo "Opus process guard: idle"
    return 0
  fi
  echo "Opus process guard: active launcher processes:"
  printf '%s\n' "${rbw_found}"
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
  stop)
    rbw_found="$(rbw_processes)"
    if [[ -z "${rbw_found}" ]]; then
      echo "Opus process guard: idle"
      exit 0
    fi
    printf '%s\n' "${rbw_found}" | while IFS=$'\t' read -r rbw_pid _; do
      kill -TERM "${rbw_pid}"
    done
    for rbw_attempt in $(seq 1 10); do
      if [[ -z "$(rbw_processes)" ]]; then
        echo "Opus process guard: stopped"
        exit 0
      fi
      sleep 1
    done
    echo "Opus process guard: a process did not stop gracefully:" >&2
    rbw_processes >&2
    exit 1
    ;;
  *)
    echo "Usage: $0 {status|assert-idle|stop}" >&2
    exit 64
    ;;
esac
