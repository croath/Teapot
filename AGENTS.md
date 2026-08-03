# Agent instructions — Teaport

This repository is **Teaport**: turn local agent CLIs into OpenAI- and Anthropic-compatible HTTP APIs.

## Project layout

| Path | Role |
|------|------|
| `core/` | Library `teaport-core`: config, agent process runner, Axum routes |
| `cli/` | Binary `teaport` (`serve`, `default-config`, `agents`) |
| `app-ui/` | Leptos CSR: **Server** (start/stop) + **Playground** only |
| `app-tauri/` | Tauri 2 shell; commands `start_api_server`, `stop_api_server`, `get_server_status` |

### Frontend tooling (`app-ui`)

- **Bun** + **Tailwind CSS v4** + **Trunk**. Source CSS: `styles/input.css` → `styles/app.css`.
- Pages: `pages/server.rs`, `pages/playground.rs`. Tauri bridge: `tauri_api.rs`.
- Dev: `cd app-ui && bun install && bun run dev`
- Desktop: `cd app-tauri && cargo tauri dev` (start/stop requires Tauri)

## Workspace dependency rules

- **Versions** live only in the root `Cargo.toml` (`[workspace.dependencies]`).
- **Features / default-features** are set only in each subproject `Cargo.toml`.
- Prefer `workspace = true` for all shared crates.

## Core behavior

1. Clients call ChatGPT-compatible or Claude-compatible endpoints.
2. The server maps `model` (or optional `agent` field) to a configured CLI.
3. The CLI is spawned; stdout is streamed back as SSE tokens / content deltas.

### API prefixes

- ChatGPT: `/chatgpt/v1` — `chat/completions`, `responses` (also `repsponses` alias), `models`, `models/{id}`
- Claude: `/claude/v1` — `messages`, `models`, `models/{model_id}`
- Health: `/health`, `/healthz`

### Models listing

`GET .../models` only includes models whose backing agent CLI is **installed** on `PATH`. Sources: agent name, `model_map` aliases, built-in catalogs, optional `list_models_args` CLI probe.

### Built-in agents

| Agent key | Default command |
|-----------|-----------------|
| `codex` | `codex exec --skip-git-repo-check {prompt}` |
| `claude` | `claude -p --output-format text {prompt}` |
| `grok` | `grok {prompt}` |
| `antigravity` / `antigravity-cli` | `antigravity-cli {prompt}` |

Templates may use `{prompt}` and `{system}`. Set `prompt_via_stdin = true` to feed the prompt on stdin.

## Commands

```bash
# Run API server
cargo run -p teaport -- serve

# List agents
cargo run -p teaport -- agents

# Print default TOML config
cargo run -p teaport -- default-config

# Check core + CLI
cargo check -p teaport-core -p teaport
```

## Coding guidelines

- All user-facing strings, docs, comments, and agent skill text in **English**.
- Brand name is **Teaport** (package names use `teaport` / `teaport-*`).
- Use `tracing` for logs; avoid `println!` in library code.
- Keep OpenAI/Anthropic wire types in `core/src/models/`.
- Keep process spawning in `core/src/agents/`.
- Prefer small, testable modules; do not add heavy deps without need.
- rustfmt: 2-space indent (see `rustfmt.toml`).

## Security notes

- Optional `api_key` in config; when set, require `Authorization: Bearer` or `x-api-key`.
- Default listen address is localhost only.
- Agent CLIs inherit the server environment; do not log full prompts at info level in production.
