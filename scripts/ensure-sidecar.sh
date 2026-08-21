#!/usr/bin/env bash
# Ensure a teapotx sidecar exists and is not older than core/cli sources.
# Dev (`cargo tauri dev`) rebuilds a debug sidecar when Rust sources change so
# new providers (e.g. codex-cli) are not missing from a stale binary.
# Optional $1 is the rustc target triple; otherwise TAURI_ENV_TARGET_TRIPLE, then host.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
HOST="$(rustc -Vv | sed -n 's/^host: //p')"
TRIPLE="${1:-${TAURI_ENV_TARGET_TRIPLE:-${HOST}}}"
DEST="${ROOT}/app-tauri/binaries/teapotx-${TRIPLE}"
DEST_EXE="${DEST}.exe"

SIDECAR=""
if [[ -f "${DEST}" ]]; then
  SIDECAR="${DEST}"
elif [[ -f "${DEST_EXE}" ]]; then
  SIDECAR="${DEST_EXE}"
fi

need_build=0
if [[ -z "${SIDECAR}" ]]; then
  echo "Sidecar missing for ${TRIPLE}; building…"
  need_build=1
elif find "${ROOT}/core" "${ROOT}/cli" "${ROOT}/Cargo.toml" \
  \( -name '*.rs' -o -name 'Cargo.toml' \) -newer "${SIDECAR}" -print -quit \
  | grep -q .; then
  echo "Sidecar stale for ${TRIPLE}; rebuilding…"
  need_build=1
fi

if [[ "${need_build}" -eq 0 ]]; then
  echo "Sidecar up to date for ${TRIPLE}"
  exit 0
fi

exec bash "${ROOT}/scripts/prepare-sidecar.sh" debug "${TRIPLE}"
