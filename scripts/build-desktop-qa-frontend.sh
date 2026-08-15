#!/usr/bin/env bash
set -euo pipefail

# QA is a runtime-capable offline-demo build. It uses the same checksum-verified
# Java bootstrap as the Premium bundle; its offline identity and data directory
# are selected by the desktop backend.
rbw_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"${rbw_root}/scripts/prepare-desktop-assets.sh"
cd "${rbw_root}/desktop"
exec npm run build
