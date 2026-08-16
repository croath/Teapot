#!/usr/bin/env bash
# Ensure a teapotx sidecar exists for the build target (build if missing).
# Optional $1 is the rustc target triple; otherwise TAURI_ENV_TARGET_TRIPLE, then host.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOST="$(rustc -Vv | sed -n 's/^host: //p')"
TRIPLE="${1:-${TAURI_ENV_TARGET_TRIPLE:-${HOST}}}"
DEST="${ROOT}/app-tauri/binaries/teapotx-${TRIPLE}"
DEST_EXE="${DEST}.exe"

if [[ -f "${DEST}" || -f "${DEST_EXE}" ]]; then
  echo "Sidecar present for ${TRIPLE}"
  exit 0
fi

echo "Sidecar missing for ${TRIPLE}; building…"
exec bash "${ROOT}/scripts/prepare-sidecar.sh" release "${TRIPLE}"
