# Local signing material

Secrets live as **individual files** under `.tauri/` (not committed). Helpers export
the env vars Tauri / `tauri-action` expect. Optional combined dotenv:

```bash
source .tauri/export-all-signing.sh   # exports + regenerates .tauri/.env
```

Upload everything to GitHub Actions repository secrets:

```bash
.scripts/upload-github-secrets.sh --dry-run
.scripts/upload-github-secrets.sh
```

---

## Windows Authenticode (self-signed)

| File | Purpose | GitHub secret |
|------|---------|---------------|
| `windows-codesign.pfx` | PKCS#12 cert + key (for `signtool` / CI) | — |
| `windows-codesign.password` | PFX password | `WINDOWS_CERTIFICATE_PASSWORD` |
| `windows-codesign.pfx.base64` | Base64 of the PFX | `WINDOWS_CERTIFICATE` |
| `windows-codesign.crt` | Public certificate only | — (safe to commit) |
| `windows-codesign.key` | Private key (PEM) | — |
| `windows-codesign.thumbprint` | SHA-1 thumbprint (Windows cert store) | — (safe to commit) |

**Do not commit** `.key`, `.pfx`, `.password`, or `.pfx.base64`.

### Local / CI env (Tauri 2)

```bash
# From repo root
export WINDOWS_CERTIFICATE="$(cat .tauri/windows-codesign.pfx.base64)"
export WINDOWS_CERTIFICATE_PASSWORD="$(tr -d '\n' < .tauri/windows-codesign.password)"
```

Or helper:

```bash
source .tauri/export-windows-signing.sh
```

Tauri reads those variables when bundling Windows installers.  
`tauri-app/tauri.conf.json` sets `digestAlgorithm` + `timestampUrl` under `bundle.windows`.

### Windows host: install cert for thumbprint signing

1. Copy `windows-codesign.pfx` to the Windows machine.
2. Double-click → import into **Current User → Personal**.
3. Trust (optional, test PCs only): export `.crt` → Trusted Root.
4. Confirm thumbprint matches `windows-codesign.thumbprint`.

Self-signed certs **do not** clear SmartScreen for end users; use a CA-issued code-signing cert for public releases.

### Regenerate

```bash
# From repo root (macOS/Linux with OpenSSL)
./.tauri/generate-windows-codesign.sh
```

---

## Tauri updater (minisign)

| File | Purpose | GitHub secret |
|------|---------|---------------|
| `tauri-updator.key` | Private key (sign update artifacts) | `TAURI_SIGNING_PRIVATE_KEY` |
| `tauri-updator.key.pub` | Public key (embed in app / `tauri.conf.json`) | — (safe to commit) |
| `tauri-updator.key.password` | Private key password | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` |

**Do not commit** `.key` or `.key.password`. The `.pub` file is safe to commit.

### Local / CI env (Tauri 2)

```bash
export TAURI_SIGNING_PRIVATE_KEY="$(cat .tauri/tauri-updator.key)"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(tr -d '\n' < .tauri/tauri-updator.key.password)"
```

Or helper:

```bash
source .tauri/export-tauri-updater.sh
```

Put the public key string into `tauri.conf.json` → `plugins.updater.pubkey`.

`createUpdaterArtifacts: true` in `tauri.conf.json` requires these vars on every release build.

### Regenerate

```bash
# From repo root (requires cargo tauri / tauri-cli v2)
./.tauri/generate-tauri-updator.sh
```

---

## Apple Developer ID + notarization (optional)

Used by `.github/workflows/release.yml` on macOS runners for:

1. Nested Mach-O re-sign under `resources/` (`scripts/codesign-macos-nested.mjs`)
2. App/DMG codesign + notarization via `tauri-apps/tauri-action`

| File | Purpose | GitHub secret |
|------|---------|---------------|
| `apple-codesign.p12.base64` | Base64 of Developer ID Application `.p12` | `APPLE_CERTIFICATE` |
| `apple-codesign.password` | `.p12` password | `APPLE_CERTIFICATE_PASSWORD` |
| `apple-signing-identity` | e.g. `Developer ID Application: … (TEAMID)` | `APPLE_SIGNING_IDENTITY` |
| `apple-api-issuer` | App Store Connect API Issuer UUID | `APPLE_API_ISSUER` |
| `apple-api-key` | App Store Connect API Key ID | `APPLE_API_KEY` |
| `apple-api-key.p8` | AuthKey `.p8` body (stable alias) | `APPLE_API_KEY_CONTENT` |
| `AuthKey_<KEYID>.p8` | Same key (Apple default filename) | (same as above) |

**Do not commit** `.p12`, `.p12.base64`, `.p8`, passwords, issuer, or identity files.

### Local / CI env (Tauri 2)

```bash
source .tauri/export-apple-signing.sh
# sets APPLE_CERTIFICATE, APPLE_CERTIFICATE_PASSWORD, APPLE_SIGNING_IDENTITY,
#      APPLE_API_ISSUER, APPLE_API_KEY, APPLE_API_KEY_PATH
```

CI note: the workflow writes `APPLE_API_KEY_CONTENT` → a temp `.p8` and sets
`APPLE_API_KEY_PATH` on the runner. Local builds use `APPLE_API_KEY_PATH` pointing
at `.tauri/apple-api-key.p8`.

Hardened runtime + entitlements live in `tauri-app/tauri.conf.json` → `bundle.macOS`
(`hardenedRuntime`, `entitlements`, `minimumSystemVersion`).

---

## GitHub secrets checklist (`release.yml`)

| Secret | Required? | Source file |
|--------|-----------|-------------|
| `TAURI_SIGNING_PRIVATE_KEY` | **yes** | `tauri-updator.key` |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | **yes** | `tauri-updator.key.password` |
| `WINDOWS_CERTIFICATE` | **yes** | `windows-codesign.pfx.base64` |
| `WINDOWS_CERTIFICATE_PASSWORD` | **yes** | `windows-codesign.password` |
| `APPLE_CERTIFICATE` | optional | `apple-codesign.p12.base64` |
| `APPLE_CERTIFICATE_PASSWORD` | optional | `apple-codesign.password` |
| `APPLE_SIGNING_IDENTITY` | optional | `apple-signing-identity` |
| `APPLE_API_ISSUER` | optional* | `apple-api-issuer` |
| `APPLE_API_KEY` | optional* | `apple-api-key` |
| `APPLE_API_KEY_CONTENT` | optional* | `apple-api-key.p8` |

\* Notarization can instead use `APPLE_ID` / `APPLE_PASSWORD` / `APPLE_TEAM_ID`
(not stored as files here). This repo uses the App Store Connect API key path.

```bash
.scripts/upload-github-secrets.sh           # all present files
.scripts/upload-github-secrets.sh --required
.scripts/upload-github-secrets.sh --apple
```
