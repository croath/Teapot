# Teapot

Expose local **provider CLIs** as **ChatGPT-compatible** and **Claude-compatible** HTTP APIs, with streaming support.

Each provider implements the `Provider` trait: Teapot expands the argv/`stdin` template, runs the process, and streams **stdout** as the completion.

> **Note:** Concrete providers are temporarily removed. Only the trait surface remains under `core/src/providers/`. The HTTP server still starts; completion endpoints return “no providers registered”.

## Workspace

| Crate | Package | Description |
|-------|---------|-------------|
| `core` | `teapot-core` | Axum server, provider traits, API types |
| `cli` | `teapot-cli` | Command-line entrypoint (`teapotx`) |
| `app-ui` | `teapot-ui` | Leptos CSR liquid-glass desktop UI (Bun + Trunk) |
| `app-tauri` | `teapot-tauri` | Tauri 2 shell; bundles `teapotx` sidecar |

Stack: **Tokio**, **Axum**, **Reqwest** (rustls), **Tracing**, **Tauri**, **Leptos**.

## Quick start

```bash
# Start the API server (provider backends currently stubbed)
cargo run -p teapot-cli -- serve

# Optional: custom config / listen address
cargo run -p teapot-cli -- serve -c teapot.toml -l 127.0.0.1:8080

# List providers (empty while backends are stubbed)
cargo run -p teapot-cli -- providers
```

### ChatGPT-compatible API

Prefix: **`/chatgpt/v1`**

```bash
# Models (empty list while no providers)
curl http://127.0.0.1:8080/chatgpt/v1/models

# Chat Completions — prefer stream:true when providers return
curl -N http://127.0.0.1:8080/chatgpt/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "example",
    "stream": true,
    "messages": [{"role":"user","content":"Say hello in one sentence."}]
  }'
```

### Claude-compatible API

Prefix: **`/claude/v1`**

```bash
curl -N http://127.0.0.1:8080/claude/v1/messages \
  -H 'Content-Type: application/json' \
  -H 'anthropic-version: 2023-06-01' \
  -d '{
    "model": "example",
    "max_tokens": 1024,
    "stream": true,
    "messages": [{"role":"user","content":"Hello"}]
  }'
```

### Models API (on each compatible surface)

- OpenAI-compatible: `GET /chatgpt/v1/models`
- Anthropic-compatible: `GET /claude/v1/models`

## Configuration

See `teapot.toml`. Print defaults:

```bash
cargo run -p teapot-cli -- default-config
```

| Key | Meaning |
|-----|---------|
| `listen` | Bind address |
| `api_key` | Optional; enables Bearer / `x-api-key` auth |
| `provider` | Optional free-form provider key (no built-ins right now) |
| `include_progress` | Stream optional `status` / `reasoning_content` (default true) |

## Desktop UI

`app-ui` is a Leptos CSR liquid-glass app (**Bun** + Tailwind v4 + Trunk; primary `#1179ac`):

1. **Home** — switch to start/stop bundled `teapotx serve`
2. **Settings** — generate config (`listen`, optional empty `api_key`)
3. **Debug** — teapotx logs + export
4. **About** — app icon and version

macOS uses an overlay transparent titlebar so the glass background fills the window.

```bash
# Install JS tooling (Bun only — no pnpm)
cd app-ui && bun install

# Browser-only UI
bun run dev

# Desktop (builds/copies teapotx sidecar if missing)
bash scripts/prepare-sidecar.sh   # once, or let beforeDevCommand ensure it
cd app-tauri && cargo tauri dev
```

## Development

```bash
cargo check -p teapot-core -p teapot-cli
cargo run -p teapot-cli -- providers
```

## License

MIT
