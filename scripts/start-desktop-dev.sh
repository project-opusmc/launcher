#!/usr/bin/env bash
set -euo pipefail

rbw_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"${rbw_root}/scripts/prepare-desktop-assets.sh"
cd "${rbw_root}/desktop"
exec npm run dev
