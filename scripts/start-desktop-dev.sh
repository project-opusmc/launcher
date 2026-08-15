#!/usr/bin/env bash
set -euo pipefail

opus_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
"${opus_root}/scripts/prepare-desktop-assets.sh"
cd "${opus_root}/desktop"
exec npm run dev
