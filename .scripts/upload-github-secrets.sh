#!/usr/bin/env bash
# Upload local signing materials under .tauri/ as GitHub Actions repository secrets.
#
# Matches secrets consumed by .github/workflows/release.yml (tauri-action + nested
# macOS codesign / notarization steps).
#
# Usage:
#   .scripts/upload-github-secrets.sh              # upload all present secrets
#   .scripts/upload-github-secrets.sh --dry-run    # list what would be set
#   .scripts/upload-github-secrets.sh --required   # only required secrets
#   .scripts/upload-github-secrets.sh --apple      # only Apple signing secrets
#   .scripts/upload-github-secrets.sh --windows    # only Windows secrets
#   .scripts/upload-github-secrets.sh --updater    # only Tauri updater secrets
#   .scripts/upload-github-secrets.sh --help
#
# Requires: gh (authenticated with repo admin or secrets:write)
#
# Secret map (local file → GitHub secret name):
#
#   Required (updater + Windows):
#     .tauri/tauri-updator.key              → TAURI_SIGNING_PRIVATE_KEY
#     .tauri/tauri-updator.key.password     → TAURI_SIGNING_PRIVATE_KEY_PASSWORD
#     .tauri/windows-codesign.pfx.base64    → WINDOWS_CERTIFICATE
#     .tauri/windows-codesign.password      → WINDOWS_CERTIFICATE_PASSWORD
#
#   Optional (macOS codesign + notarize):
#     .tauri/apple-codesign.p12.base64      → APPLE_CERTIFICATE
#     .tauri/apple-codesign.password        → APPLE_CERTIFICATE_PASSWORD
#     .tauri/apple-signing-identity         → APPLE_SIGNING_IDENTITY
#     .tauri/apple-api-issuer               → APPLE_API_ISSUER
#     .tauri/apple-api-key                  → APPLE_API_KEY
#     .tauri/apple-api-key.p8               → APPLE_API_KEY_CONTENT
#       (fallback: .tauri/AuthKey_<KEYID>.p8)
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"
TAURI_DIR="$ROOT/.tauri"

DRY_RUN=0
MODE=all   # all | required | apple | windows | updater

usage() {
  sed -n '2,36p' "$0" | sed 's/^# \{0,1\}//'
  exit 0
}

die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }
log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
ok()  { printf '\033[1;32m✓\033[0m %s\n' "$*"; }
warn(){ printf '\033[1;33mwarn:\033[0m %s\n' "$*" >&2; }

have() { command -v "$1" >/dev/null 2>&1; }

while [[ $# -gt 0 ]]; do
  case "$1" in
    --dry-run) DRY_RUN=1; shift ;;
    --required) MODE=required; shift ;;
    --apple) MODE=apple; shift ;;
    --windows) MODE=windows; shift ;;
    --updater) MODE=updater; shift ;;
    --help|-h) usage ;;
    -*) die "unknown option: $1 (try --help)" ;;
    *) die "unexpected argument: $1 (try --help)" ;;
  esac
done

have gh || die "gh is required (https://cli.github.com/)"
gh auth status >/dev/null 2>&1 || die "gh is not authenticated (run: gh auth login)"

# Resolve APPLE_API_KEY_CONTENT source (.p8 body)
resolve_apple_p8() {
  local p8="$TAURI_DIR/apple-api-key.p8"
  if [[ -f "$p8" ]]; then
    printf '%s' "$p8"
    return 0
  fi
  local key_id=""
  if [[ -f "$TAURI_DIR/apple-api-key" ]]; then
    key_id="$(tr -d '\n' < "$TAURI_DIR/apple-api-key")"
  fi
  if [[ -n "$key_id" && -f "$TAURI_DIR/AuthKey_${key_id}.p8" ]]; then
    printf '%s' "$TAURI_DIR/AuthKey_${key_id}.p8"
    return 0
  fi
  # Any AuthKey_*.p8
  local found
  found="$(ls -1 "$TAURI_DIR"/AuthKey_*.p8 2>/dev/null | head -1 || true)"
  if [[ -n "$found" && -f "$found" ]]; then
    printf '%s' "$found"
    return 0
  fi
  return 1
}

# Each entry: SECRET_NAME|relative_path|group|required(0|1)
# path may be special: @apple-p8
SECRETS=(
  "TAURI_SIGNING_PRIVATE_KEY|.tauri/tauri-updator.key|updater|1"
  "TAURI_SIGNING_PRIVATE_KEY_PASSWORD|.tauri/tauri-updator.key.password|updater|1"
  "WINDOWS_CERTIFICATE|.tauri/windows-codesign.pfx.base64|windows|1"
  "WINDOWS_CERTIFICATE_PASSWORD|.tauri/windows-codesign.password|windows|1"
  "APPLE_CERTIFICATE|.tauri/apple-codesign.p12.base64|apple|0"
  "APPLE_CERTIFICATE_PASSWORD|.tauri/apple-codesign.password|apple|0"
  "APPLE_SIGNING_IDENTITY|.tauri/apple-signing-identity|apple|0"
  "APPLE_API_ISSUER|.tauri/apple-api-issuer|apple|0"
  "APPLE_API_KEY|.tauri/apple-api-key|apple|0"
  "APPLE_API_KEY_CONTENT|@apple-p8|apple|0"
)

group_match() {
  local group="$1"
  case "$MODE" in
    all) return 0 ;;
    required) [[ "$2" == "1" ]] && return 0; return 1 ;;
    apple|windows|updater) [[ "$group" == "$MODE" ]] && return 0; return 1 ;;
    *) return 1 ;;
  esac
}

set_secret_from_file() {
  local name="$1"
  local file="$2"
  local bytes
  bytes="$(wc -c < "$file" | tr -d ' ')"
  if [[ "$DRY_RUN" -eq 1 ]]; then
    log "[dry-run] gh secret set $name  ←  $file ($bytes bytes)"
    return 0
  fi
  log "Setting secret $name from $file ($bytes bytes)"
  # Strip a single trailing newline so passwords/keys match local tooling
  # (gh reads stdin body as-is; we normalize via tr for single-line secrets)
  case "$name" in
    TAURI_SIGNING_PRIVATE_KEY)
      # Multi-line minisign key — keep exact file body
      gh secret set "$name" < "$file"
      ;;
    APPLE_API_KEY_CONTENT)
      # .p8 PEM — keep exact file body
      gh secret set "$name" < "$file"
      ;;
    *)
      # Single-line values: strip CR/LF
      tr -d '\r\n' < "$file" | gh secret set "$name"
      ;;
  esac
  ok "$name"
}

UPLOADED=0
SKIPPED=0
MISSING_REQUIRED=0

for entry in "${SECRETS[@]}"; do
  IFS='|' read -r name rel group required <<< "$entry"
  group_match "$group" "$required" || continue

  file=""
  if [[ "$rel" == "@apple-p8" ]]; then
    if ! file="$(resolve_apple_p8)"; then
      if [[ "$required" == "1" ]]; then
        warn "required secret $name: no .p8 found under .tauri/"
        MISSING_REQUIRED=$((MISSING_REQUIRED + 1))
      else
        warn "skip $name (optional): no apple-api-key.p8 / AuthKey_*.p8"
        SKIPPED=$((SKIPPED + 1))
      fi
      continue
    fi
  else
    file="$ROOT/$rel"
    if [[ ! -f "$file" ]]; then
      if [[ "$required" == "1" ]]; then
        warn "required secret $name: missing $rel"
        MISSING_REQUIRED=$((MISSING_REQUIRED + 1))
      else
        warn "skip $name (optional): missing $rel"
        SKIPPED=$((SKIPPED + 1))
      fi
      continue
    fi
  fi

  set_secret_from_file "$name" "$file"
  UPLOADED=$((UPLOADED + 1))
done

echo
if [[ "$DRY_RUN" -eq 1 ]]; then
  log "Dry-run complete: would upload $UPLOADED secret(s), skip $SKIPPED, missing-required $MISSING_REQUIRED"
else
  log "Done: uploaded $UPLOADED secret(s), skipped $SKIPPED, missing-required $MISSING_REQUIRED"
fi

if [[ "$MISSING_REQUIRED" -gt 0 ]]; then
  die "$MISSING_REQUIRED required secret file(s) missing — see .tauri/README.md"
fi

if [[ "$DRY_RUN" -eq 0 && "$UPLOADED" -gt 0 ]]; then
  echo "Verify: gh secret list"
  gh secret list
fi
