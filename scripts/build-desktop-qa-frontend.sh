#!/usr/bin/env bash
set -euo pipefail

# QA is a runtime-capable offline-demo build. It uses the same checksum-verified
# Java bootstrap as the Premium bundle; its offline identity and data directory
# are selected by the desktop backend.
opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"${opus_root}/scripts/prepare-desktop-assets.sh"
cd "${opus_root}/desktop"
exec npm run build
