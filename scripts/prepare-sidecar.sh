#!/usr/bin/env bash
# Build teapotx and copy it into app-tauri/binaries with the host target triple
# so Tauri externalBin / sidecar resolution works.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOST="$(rustc -Vv | sed -n 's/^host: //p')"
if [[ -z "${HOST}" ]]; then
  echo "error: could not detect rustc host triple" >&2
  exit 1
fi

PROFILE="${1:-release}"
case "${PROFILE}" in
  release)
    cargo build -p teapot-cli --release
    SRC="${ROOT}/target/release/teapotx"
    ;;
  debug)
    cargo build -p teapot-cli
    SRC="${ROOT}/target/debug/teapotx"
    ;;
  *)
    echo "usage: $0 [release|debug]" >&2
    exit 1
    ;;
esac

if [[ ! -f "${SRC}" ]]; then
  # Windows cargo outputs teapotx.exe
  if [[ -f "${SRC}.exe" ]]; then
    SRC="${SRC}.exe"
  else
    echo "error: teapotx binary not found at ${SRC}" >&2
    exit 1
  fi
fi

DEST_DIR="${ROOT}/app-tauri/binaries"
mkdir -p "${DEST_DIR}"

if [[ "${SRC}" == *.exe ]]; then
  DEST="${DEST_DIR}/teapotx-${HOST}.exe"
else
  DEST="${DEST_DIR}/teapotx-${HOST}"
fi

cp -f "${SRC}" "${DEST}"
chmod +x "${DEST}" 2>/dev/null || true
echo "Prepared sidecar: ${DEST}"
