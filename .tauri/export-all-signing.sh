#!/usr/bin/env bash
# Source this file to export all local signing env vars used by Tauri build / CI.
#   source .tauri/export-all-signing.sh
#
# Also regenerates .tauri/.env (gitignored) for tools that load dotenv.
# Note: TAURI_SIGNING_PRIVATE_KEY may be multi-line — stored as double-quoted
# escaped value; prefer `source` helpers over parsing .env for that key.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# shellcheck source=/dev/null
source "$ROOT/export-tauri-updater.sh"
# shellcheck source=/dev/null
source "$ROOT/export-windows-signing.sh"
# shellcheck source=/dev/null
source "$ROOT/export-apple-signing.sh"

env_escape() {
  # Escape for double-quoted dotenv values
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/$/\\n/' | tr -d '\n' | sed 's/\\n$//'
}

{
  printf 'APPLE_API_ISSUER=%s\n' "${APPLE_API_ISSUER}"
  printf 'APPLE_API_KEY=%s\n' "${APPLE_API_KEY}"
  if [[ -n "${APPLE_API_KEY_PATH:-}" ]]; then
    rel="${APPLE_API_KEY_PATH#"$(cd "$ROOT/.." && pwd)/"}"
    printf 'APPLE_API_KEY_PATH=%s\n' "$rel"
  fi
  printf "APPLE_SIGNING_IDENTITY='%s'\n" "${APPLE_SIGNING_IDENTITY}"
  printf 'APPLE_CERTIFICATE=%s\n' "${APPLE_CERTIFICATE}"
  printf 'APPLE_CERTIFICATE_PASSWORD=%s\n' "${APPLE_CERTIFICATE_PASSWORD}"
  printf 'TAURI_SIGNING_PRIVATE_KEY="%s"\n' "$(env_escape "${TAURI_SIGNING_PRIVATE_KEY}")"
  printf 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD=%s\n' "${TAURI_SIGNING_PRIVATE_KEY_PASSWORD}"
  printf 'WINDOWS_CERTIFICATE=%s\n' "${WINDOWS_CERTIFICATE}"
  printf 'WINDOWS_CERTIFICATE_PASSWORD=%s\n' "${WINDOWS_CERTIFICATE_PASSWORD}"
} > "$ROOT/.env"
chmod 600 "$ROOT/.env"
echo "Wrote $ROOT/.env from file-based secrets"
