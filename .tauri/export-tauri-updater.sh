#!/usr/bin/env bash
# Source this file to export TAURI_SIGNING_* for updater artifact signing.
#   source .tauri/export-tauri-updater.sh
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEY_FILE="$ROOT/tauri-updator.key"
PASS_FILE="$ROOT/tauri-updator.key.password"

if [[ ! -f "$KEY_FILE" || ! -f "$PASS_FILE" ]]; then
  echo "error: missing $KEY_FILE or $PASS_FILE — run .tauri/generate-tauri-updator.sh" >&2
  return 1 2>/dev/null || exit 1
fi

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(tr -d '\n' < "$PASS_FILE")"
echo "Exported TAURI_SIGNING_PRIVATE_KEY and TAURI_SIGNING_PRIVATE_KEY_PASSWORD (from .tauri/)"
