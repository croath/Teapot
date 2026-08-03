# Teaport

Expose local **agent CLIs** (`codex`, `claude`, `grok`, `antigravity-cli`) as **ChatGPT-compatible** and **Claude-compatible** HTTP APIs, with streaming support.

## Workspace

| Crate | Package | Description |
|-------|---------|-------------|
| `core` | `teaport-core` | Axum server, agent runner, API types |
| `cli` | `teaport` | Command-line entrypoint |
| `app-ui` | `teaport-ui` | Leptos CSR UI (Bun + Tailwind) |
| `app-tauri` | `teaport-tauri` | Tauri 2 desktop app |

Stack: **Tokio**, **Axum**, **Reqwest** (rustls), **Tracing**, **Tauri**, **Leptos**.

## Quick start

```bash
# Start the API server (default http://127.0.0.1:8080)
cargo run -p teaport -- serve

# Pin to one agent CLI (models list + default routing use only this agent)
cargo run -p teaport -- serve -a codex
cargo run -p teaport -- serve --agent claude

# Optional: custom config / listen address
cargo run -p teaport -- serve -c teaport.toml -l 127.0.0.1:8080 -a grok

# List models for one agent
cargo run -p teaport -- models -a codex
```

### ChatGPT-compatible API

Prefix: **`/chatgpt/v1`**

```bash
# Chat Completions (streaming)
curl -N http://127.0.0.1:8080/chatgpt/v1/chat/completions \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "codex",
    "stream": true,
    "messages": [{"role":"user","content":"Say hello in one sentence."}]
  }'

# Responses API
curl http://127.0.0.1:8080/chatgpt/v1/responses \
  -H 'Content-Type: application/json' \
  -d '{
    "model": "claude",
    "input": "List three Rust tips."
  }'
```

### Claude-compatible API

Prefix: **`/claude/v1`**

```bash
curl -N http://127.0.0.1:8080/claude/v1/messages \
  -H 'Content-Type: application/json' \
  -H 'anthropic-version: 2023-06-01' \
  -d '{
    "model": "claude",
    "max_tokens": 1024,
    "stream": true,
    "messages": [{"role":"user","content":"Hello"}]
  }'
```

### Models API (installed agent CLIs only)

```bash
# OpenAI-compatible
curl http://127.0.0.1:8080/chatgpt/v1/models
curl http://127.0.0.1:8080/chatgpt/v1/models/codex

# Anthropic-compatible
curl http://127.0.0.1:8080/claude/v1/models
curl http://127.0.0.1:8080/claude/v1/models/claude

# CLI helper
cargo run -p teaport -- models
```

Listing includes:

1. Agent names whose binary is on `PATH`
2. `model_map` aliases that resolve to installed agents
3. Built-in catalogs for known families (codex / claude / grok) when installed
4. Optional CLI probe via `list_models_args` in config (e.g. `codex models`)

### Model → agent routing

- Use an agent name as `model` (`codex`, `claude`, `grok`, `antigravity`).
- Or map aliases in `teaport.toml` (`[model_map]`).
- Optional extension field `"agent": "codex"` overrides the model map.

## Configuration

See `teaport.toml`. Print defaults:

```bash
cargo run -p teaport -- default-config
```

| Key | Meaning |
|-----|---------|
| `listen` | Bind address |
| `api_key` | Optional; enables Bearer / `x-api-key` auth |
| `default_agent` | Fallback agent |
| `agents.*` | Command, args (`{prompt}`, `{system}`), timeout |
| `model_map` | Request model id → agent name |

## Desktop UI

`app-ui` is a small Leptos CSR app (Bun + Tailwind + Trunk) with two pages:

1. **Server** — start / stop the local API (via Tauri commands) and health-check
2. **Playground** — send ChatGPT or Claude compatible requests to the running server

```bash
# Install JS tooling
cd app-ui && bun install

# Browser-only UI (start/stop disabled; use CLI `serve` instead)
bun run dev

# Full desktop app with start/stop
cd ../app-tauri && cargo tauri dev
```

## Development

```bash
cargo check -p teaport-core -p teaport
cargo run -p teaport -- agents
```

## License

MIT
