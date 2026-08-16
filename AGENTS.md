# Agent instructions — Teapot

This repository is **Teapot**: turn local provider CLIs into OpenAI- and Anthropic-compatible HTTP APIs.

## Project layout

| Path | Role |
|------|------|
| `core/` | Library `teapot-core`: config, provider traits, Axum routes |
| `cli/` | Package `teapot-cli`, binary `teapotx` (`serve`, `default-config`, `providers`) |
| `app-ui/` | Leptos CSR liquid-glass UI (Home / Settings / Debug / About) |
| `app-tauri/` | Tauri 2 shell: transparent macOS titlebar, bundles `teapotx` sidecar |

### Frontend tooling (`app-ui`)

- **Bun only** (no pnpm/npm). + **Tailwind CSS v4** + **Trunk**. Source CSS: `styles/tailwind.css`.
- Primary brand color: `#1179ac`. macOS liquid-glass panels over transparent window + overlay titlebar.
- Pages: `pages/home.rs` (serve switch), `settings.rs` (macOS-style hub + General / Debug / About / Language panes).
- Navigation: home shows a Settings gear → `/settings`; panes at `/settings/generate|debug|about|language`. Native system menu also opens these routes.
- Dev UI: `cd app-ui && bun install && trunk serve`
- Desktop: `bash scripts/prepare-sidecar.sh` once, then `cargo tauri dev`
- Sidecar: `app-tauri/binaries/teapotx-<target-triple>` via `bundle.externalBin`
- Updater: `tauri-plugin-updater` + GitHub `latest.json`. Public key is
  `plugins.updater.pubkey` (from `.tauri/tauri-updator.key.pub`). Release
  builds need `source .tauri/export-tauri-updater.sh` (or the matching
  GitHub secrets) because `bundle.createUpdaterArtifacts` is true.
- TelemetryDeck (install + DAU): `app-tauri/src/telemetry.rs`. App ID is
  baked at compile time from `TELEMETRYDECK_APP_ID` (`app-tauri/build.rs`).
  Empty value disables sending. Process env wins, then `app-tauri/.env`,
  then workspace `.env`. Debug builds send `isTestMode` unless
  `TELEMETRYDECK_TEST_MODE=false`. Release CI should set the
  `TELEMETRYDECK_APP_ID` GitHub secret so production builds emit signals.

## Workspace dependency rules

- **Versions** live only in the root `Cargo.toml` (`[workspace.dependencies]`).
- **Features / default-features** are set only in each subproject `Cargo.toml`.
- Prefer `workspace = true` for all shared crates.

## Core behavior

1. Server starts with **one pinned provider** (`teapotx serve -p <provider>`).
2. At bootstrap, create a single [`PinnedProvider`] instance from that name, store it
   in **`AppState.provider`** (and in `ProviderRuntime`), then:
   load auth from `auth/{provider}.json` → refresh access token into **memory** →
   load models (disk cache, else upstream via the same provider) into memory + local store.
3. Credentials are **provider-owned** in memory (`StoredAuth` or `VertexSession`),
   not a shared `LiveCredentials` bag. Background tasks refresh that session near
   expiry and re-fetch models periodically. `execute` / models only read the
   provider's own session.
4. **Chat Completions** checks the request `model` against the provider's cached
   models list (error if missing; no auto-adapt), then calls
   `state.provider.execute(…)` (uses that provider's native session).
5. **Models** APIs (`/chatgpt/v1/models`, `/claude/v1/models`) read the catalog
   owned by the runtime for that pinned provider.
6. Optional CLI path remains: `SpawnSpec` → process spawn → stream **stdout**.

**Providers:** `codex`, `claude`, `xai`, `antigravity`, `vertex` under
`core/src/providers/`. Each implements `Provider` (spawn + **auth** + **execute** + **models**).

### API prefixes (compatible surfaces)

- ChatGPT (`ChatGptSurface`): `/chatgpt/v1`
  - `POST chat/completions`
  - Responses API (`api/chatgpt/responses.rs`), OpenAI-compatible:
    - `POST responses` (also `repsponses` alias), `GET|DELETE responses/{id}`
    - `POST responses/{id}/cancel`, `GET responses/{id}/input_items`
    - `POST responses/compact`, `POST responses/input_tokens`
  - `GET models`, `GET models/{id}` — OpenAI-compatible models API (`api/chatgpt/models.rs`)
- Claude (`ClaudeSurface`): `/claude/v1`
  - `POST messages`
  - `GET models`, `GET models/{model_id}` — Anthropic-compatible models API (`api/claude/models.rs`)
- Health: `/health`, `/healthz`

### Models listing

Models list lives **on each compatible surface** (not a standalone Teapot route):

- OpenAI shape: `GET /chatgpt/v1/models`
- Anthropic shape: `GET /claude/v1/models`

Both read the **pinned provider's in-memory catalog** (seeded at startup from
that provider's own disk file, or an upstream fetch, refreshed periodically).

**One file per provider** under `{data_local}/teapot/models/`:

```text
models/
  codex.json         # { updated_at, models: [CodexModel, …] }
  claude.json        # { updated_at, models: [ClaudeModel, …] }
  …
```

A server pinned to e.g. `codex` **only** reads/updates `models/codex.json`.
Common typed API: `ModelsStore::load_models::<T>(kind)` /
`ModelsStore::save_models(kind, &native_list)`.

### Providers (trait backends)

Canonical module: `core/src/providers/`.

| Item | Role |
|------|------|
| `traits::Provider` | CLI backend: `id`, `command`, `spawn_spec`, **auth** (`login` / `refresh_auth` / …) |
| `ProviderAuth` | Object-safe auth surface for CLI / registry (`Arc<dyn ProviderAuth>`) |
| `pinned` | `PinnedProvider` enum — one concrete instance for the server process |
| `<name>/execute.rs` | Struct method `execute(…)` using that provider's own session |
| `<name>/models.rs` | Native model structs + `models` / `model` (decode HTTP) |
| `models_store` | One `{provider}.json` per provider; typed load/save of native lists |
| `models_cache` | `ModelsCache` + `NativeModelCatalog`: in-memory catalog for the pinned provider |
| `runtime` | `ProviderRuntime`: holds `Arc<PinnedProvider>`, `Arc<ModelsCache>`, session bootstrap |
| `model_info` | Shared `ModelInfo` + `ProviderModel` convert-on-use trait |
| `execute` | Shared `ExecRequest` / `ExecResponse` |
| `traits::ProviderExecutor` | Spawn + stream `ProviderEvent`s |
| Helpers | `expand_args`, `stdin_prompt`, `resolve_binary`, `flatten_messages` |

Each builtin lives in its own directory under `core/src/providers/<name>/`:

| Path | Contents |
|------|----------|
| `mod.rs` | Provider type + `spawn_spec` + auth method dispatch |
| `auth.rs` | Provider-specific login / refresh / token helpers |
| `execute.rs` | HTTP `execute` (struct methods only) |
| `models.rs` | HTTP `models` / `model` from upstream (no hard-coded catalog) |

Built-ins: `codex`, `claude`, `xai` (Grok CLI), `antigravity` (`agy`), `vertex`.

Argv templates may use `{prompt}`, `{system}`, and `{model}` where a provider expands them in code.

### Auth (per-provider JSON files)

Shared helpers only: `core/src/auth/` (JSON store, PKCE, form POST, browser,
`LoginOptions` / `AuthMethod`). Identity is the typed enum **`ProviderKind`**
(not free-form strings). Trait boundary uses **`AuthEntry`**, an enum that
wraps each provider’s own `StoredAuth` struct (no flattened common credential).

Each provider owns OAuth constants, **local callback**, JWT parsing, import
options, and `StoredAuth` under `providers/<name>/`.

Antigravity Google OAuth client credentials are **not** hardcoded. `teapot-core`
bakes them in at compile time (`core/build.rs`) from
`ANTIGRAVITY_CLIENT_ID` and `ANTIGRAVITY_CLIENT_SECRET`. Process env wins, then
`core/.env`, then the workspace `.env`. See `.env.example`. A missing value
fails the build. Do not add unprefixed `CLIENT_ID` / `CLIENT_SECRET` aliases.

**One file per provider** under `{data_local}/teapot/auth/`:

```text
auth/
  codex.json         # account map of Codex StoredAuth (native fields only)
  claude.json
  xai.json
  antigravity.json
  vertex.json
```

Example `codex.json`:

```json
{
  "user@example.com": {
    "auth_kind": "oauth",
    "access_token": "…",
    "refresh_token": "…",
    "account_id": "…"
  }
}
```

Common typed API on [`AuthStore`] (same methods, different `T` per provider):

```rust
store.save_account(ProviderKind::Codex, account, &codex_auth)?;
let rows: Vec<(String, CodexAuth)> = store.load_all(ProviderKind::Codex)?;
let one: ClaudeAuth = store.load_account(ProviderKind::Claude, account)?;
```

Legacy single-file `{data_local}/teapot/auth.json` is still read once and
migrated into the per-provider files.

| Provider | Flow |
|----------|------|
| `codex` | Browser OAuth + PKCE (OpenAI) |
| `claude` | Browser OAuth + PKCE (Anthropic) |
| `xai` | Device-code OAuth |
| `antigravity` | Browser OAuth (Google); client id/secret from `ANTIGRAVITY_CLIENT_*` at build |
| `vertex` | Import service-account JSON |

```bash
cargo run -p teapot-cli -- auth login codex
cargo run -p teapot-cli -- auth login xai
cargo run -p teapot-cli -- auth login vertex -c /path/to/sa.json
cargo run -p teapot-cli -- auth list
cargo run -p teapot-cli -- auth status
cargo run -p teapot-cli -- auth refresh codex
cargo run -p teapot-cli -- auth logout codex
cargo run -p teapot-cli -- auth path
```

Override store dir with `--auth-dir` / `TEAPOT_AUTH_DIR`.

## Commands

```bash
# Run API server
cargo run -p teapot-cli -- serve

# Optional provider pin
cargo run -p teapot-cli -- serve -p codex

# List providers
cargo run -p teapot-cli -- providers

# Provider auth (TOML under local app data)
cargo run -p teapot-cli -- auth login codex
cargo run -p teapot-cli -- auth list

# Print default TOML config
cargo run -p teapot-cli -- default-config

# Check core + CLI
cargo check -p teapot-core -p teapot-cli
```

## Coding guidelines

- All user-facing strings, docs, comments, and agent skill text in **English**.
- Brand name is **Teapot** (packages use `teapot-*`; CLI binary is `teapotx`).
- Use `tracing` for logs; avoid `println!` in library code.
- Keep OpenAI/Anthropic wire types in `core/src/models/`.
- Keep process spawning in `core/src/providers/` (`Provider` trait + executor).
- Keep API prefixes behind `ApiSurface` in `core/src/api/surface.rs`.
- Prefer small, testable modules; do not add heavy deps without need.
- rustfmt: 2-space indent (see `rustfmt.toml`).

## Desktop analytics (TelemetryDeck)

The Tauri shell posts **Ingest API v2** to `https://nom.telemetrydeck.com/v2/`.
CLI-only `teapotx` does not send analytics.

| Type | When | Metric |
|------|------|--------|
| `App.installed` | First launch only (new `telemetry_client_id`) | Installs / new users |
| `TelemetryDeck.Session.started` | Every process start | Sessions |
| `App.dailyActive` | ≤1× per local calendar day | DAU |

Identity: random UUID in the Tauri app-data dir (`telemetry_client_id`) →
SHA-256 hex as `clientUser`. DAU marker: `telemetry_last_daily_active`
(`YYYY-MM-DD`). A background thread re-checks every 30 minutes so
long-running sessions still count the next day.

```bash
# .env (workspace root or app-tauri/.env)
TELEMETRYDECK_APP_ID=YOUR-UUID-APP-ID
# TELEMETRYDECK_TEST_MODE=false
```

Rebuild after changing env. Dashboard: enable **Test Mode** to see debug
signals (`Explore → Recent Signals`). Production Overview hides test data.

## Security notes

- Optional `api_key` in config; when set, require `Authorization: Bearer` or `x-api-key`.
- Default listen address is localhost only.
- Provider CLIs inherit the server environment; do not log full prompts at info level in production.
- TelemetryDeck never receives serial numbers or hardware UUIDs. The install
  id stays local; only its SHA-256 hex is posted.
