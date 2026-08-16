#!/usr/bin/env bash
# Source this file to export APPLE_* env vars for Tauri bundling / notarization.
#   source .tauri/export-apple-signing.sh
#
# File layout (same idea as windows-codesign / tauri-updator):
#   apple-api-issuer           → APPLE_API_ISSUER
#   apple-api-key              → APPLE_API_KEY (Key ID)
#   apple-api-key.p8           → APPLE_API_KEY_PATH (+ content for CI secret)
#   apple-signing-identity     → APPLE_SIGNING_IDENTITY
#   apple-codesign.p12.base64  → APPLE_CERTIFICATE
#   apple-codesign.password    → APPLE_CERTIFICATE_PASSWORD
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

need() {
  local f="$1"
  if [[ ! -f "$f" ]]; then
    echo "error: missing $f — see .tauri/README.md (Apple signing)" >&2
    return 1 2>/dev/null || exit 1
  fi
}

need "$ROOT/apple-api-issuer"
need "$ROOT/apple-api-key"
need "$ROOT/apple-signing-identity"
need "$ROOT/apple-codesign.p12.base64"
need "$ROOT/apple-codesign.password"

export APPLE_API_ISSUER="$(tr -d '\n' < "$ROOT/apple-api-issuer")"
export APPLE_API_KEY="$(tr -d '\n' < "$ROOT/apple-api-key")"
export APPLE_SIGNING_IDENTITY="$(tr -d '\n' < "$ROOT/apple-signing-identity")"
export APPLE_CERTIFICATE="$(tr -d '\n' < "$ROOT/apple-codesign.p12.base64")"
export APPLE_CERTIFICATE_PASSWORD="$(tr -d '\n' < "$ROOT/apple-codesign.password")"

# Prefer stable alias; fall back to AuthKey_<KEYID>.p8
P8="$ROOT/apple-api-key.p8"
if [[ ! -f "$P8" ]]; then
  P8="$ROOT/AuthKey_${APPLE_API_KEY}.p8"
fi
if [[ -f "$P8" ]]; then
  # Absolute path so Tauri/notarytool can open it regardless of cwd
  export APPLE_API_KEY_PATH="$(cd "$(dirname "$P8")" && pwd)/$(basename "$P8")"
else
  echo "warn: no .p8 at apple-api-key.p8 or AuthKey_${APPLE_API_KEY}.p8 — APPLE_API_KEY_PATH unset" >&2
fi

echo "Exported APPLE_* signing env (from .tauri/)"
