#!/usr/bin/env bash
# Generate a Tauri updater signing keypair into this directory.
# Produces: tauri-updator.key, tauri-updator.key.pub, tauri-updator.key.password
set -euo pipefail
DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$DIR"

KEY_BASE="tauri-updator.key"
KEY_FILE="${KEY_BASE}"
PUB_FILE="${KEY_BASE}.pub"
PASS_FILE="${KEY_BASE}.password"

if ! command -v cargo >/dev/null 2>&1; then
  echo "error: cargo not found — install Rust toolchain" >&2
  exit 1
fi

if ! cargo tauri signer generate --help >/dev/null 2>&1; then
  echo "error: cargo tauri not available — install with: cargo install tauri-cli --version \"^2\"" >&2
  exit 1
fi

PASS="$(openssl rand -base64 24 | tr -d '/+=' | head -c 24)"

# -w writes private key to path and public key to path.pub
# -p sets the private-key password; --ci / -f allow non-interactive overwrite
cargo tauri signer generate \
  --write-keys "$KEY_FILE" \
  --password "$PASS" \
  --force \
  --ci

printf '%s\n' "$PASS" > "$PASS_FILE"

chmod 600 "$KEY_FILE" "$PASS_FILE"
chmod 644 "$PUB_FILE"

echo "Created Tauri updater signing materials in $DIR"
echo "  private key: $KEY_FILE"
echo "  public key:  $PUB_FILE"
echo "  password:    $PASS_FILE"
echo
echo "Usage (signing releases / CI):"
echo "  export TAURI_SIGNING_PRIVATE_KEY=\"\$(cat .tauri/$KEY_FILE)\""
echo "  export TAURI_SIGNING_PRIVATE_KEY_PASSWORD=\"\$(tr -d '\\n' < .tauri/$PASS_FILE)\""
echo
echo "Embed the public key in tauri.conf.json under plugins.updater.pubkey:"
echo "  $(tr -d '\n' < "$PUB_FILE")"
