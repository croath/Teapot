#!/usr/bin/env bash
# Ensure a teapotx sidecar exists for the current host (build if missing).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOST="$(rustc -Vv | sed -n 's/^host: //p')"
DEST="${ROOT}/app-tauri/binaries/teapotx-${HOST}"
DEST_EXE="${DEST}.exe"

if [[ -f "${DEST}" || -f "${DEST_EXE}" ]]; then
  echo "Sidecar present for ${HOST}"
  exit 0
fi

echo "Sidecar missing for ${HOST}; building…"
exec bash "${ROOT}/scripts/prepare-sidecar.sh" release
