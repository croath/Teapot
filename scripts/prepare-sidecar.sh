#!/usr/bin/env bash
# Build teapotx and copy it into app-tauri/binaries with the build target triple
# so Tauri externalBin / sidecar resolution works.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

HOST="$(rustc -Vv | sed -n 's/^host: //p')"
if [[ -z "${HOST}" ]]; then
  echo "error: could not detect rustc host triple" >&2
  exit 1
fi

# Prefer Tauri's target (set during `cargo tauri build --target …`), then $2, then host.
TRIPLE="${TAURI_ENV_TARGET_TRIPLE:-${2:-${HOST}}}"
if [[ -z "${TRIPLE}" ]]; then
  echo "error: could not determine target triple" >&2
  exit 1
fi

if [[ "${TRIPLE}" != "${HOST}" ]]; then
  rustup target add "${TRIPLE}"
fi

PROFILE="${1:-release}"
case "${PROFILE}" in
  release)
    cargo build -p teapot-cli --release --target "${TRIPLE}"
    SRC="${ROOT}/target/${TRIPLE}/release/teapotx"
    ;;
  debug)
    cargo build -p teapot-cli --target "${TRIPLE}"
    SRC="${ROOT}/target/${TRIPLE}/debug/teapotx"
    ;;
  *)
    echo "usage: $0 [release|debug] [target-triple]" >&2
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
  DEST="${DEST_DIR}/teapotx-${TRIPLE}.exe"
else
  DEST="${DEST_DIR}/teapotx-${TRIPLE}"
fi

cp -f "${SRC}" "${DEST}"
chmod +x "${DEST}" 2>/dev/null || true
echo "Prepared sidecar: ${DEST}"
