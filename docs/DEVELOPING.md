# Development

English | [中文](DEVELOPING.zh.md)

How to build, run, observe, and debug ambery while developing. All commands run from the repository root unless noted.

## Build and test

```bash
cargo test --workspace                          # Rust workspace (core + case runner + activity viewer)
cargo run -p ambery-case -- frontend --silent   # headless frontend case (mock/keyless, embeds core)
cd app && npm ci && npm run build               # frontend: tsc + vite build
```

## Run the full stack (browser debug UI)

Three processes, three terminals:

```bash
# 1) local OpenAI-compatible LLM replacement (default port 47777)
python3 scripts/debug_brain.py

# 2) core backend (HTTP + WS on 127.0.0.1:47600; AMBERY_PORT overrides the port)
cargo run -p ambery-case -- serve --brain-addr http://127.0.0.1:47777

# 3) frontend dev server — open the printed URL (usually http://localhost:5173)
cd app && npm run dev
```

The debug brain is a minimal threshold decision source, not a conversational model: a hook that passes its notify rule produces a notification card, and an ordinary chat gets an empty reply. For real conversation, point `--brain-addr` at any OpenAI-compatible endpoint.

## Watch storage in real time

```bash
cargo run -p ambery-core --bin ambery-activity -- \
  --dir ~/.config/ambery/storage --trajectory --follow
```

This is the standard development command: the trajectory ledger (session / turn / event; docs/tools.md) tails newly written records (`--follow`). Storage lives under the config root (default `~/.config/ambery/storage`, overridable by `AMBERY_STORAGE_DIR`); pass `--dir` explicitly to browse a snapshot.

## Simulate hooks

The real product is driven by Claude Code hooks; during development you can drive the same path over HTTP:

```bash
curl -X POST http://127.0.0.1:47600/hook -H 'Content-Type: application/json' \
  -d '{"event":"session_start","session_id":"dev-1","cwd":"/tmp","kind":"claude"}'
curl -X POST http://127.0.0.1:47600/hook -H 'Content-Type: application/json' \
  -d '{"event":"user_prompt","session_id":"dev-1","cwd":"/tmp","kind":"claude","prompt":"hello"}'
```

`kind` must be a filter name (`claude` / `opencode`). Observe effects via `GET /state`, `GET /context`, the WS stream, and the storage files (docs/storage.md).

## Tauri shell

```bash
cd app && npx tauri build               # the only correct build entry point (see below)
cd app/src-tauri && AMBERY_PORT=47601 ./target/release/ambery   # run on a non-default port
```

The shell embeds the built frontend and core. Packaging (`.app` bundle) is not active until the release round (docs/terminal/wt/sidecar.md).

**The shell build must go through `npx tauri build` — never bare `cargo build`.** The frontend `dist/` is embedded into `tauri-codegen-assets/` by the build script, which does not watch `dist/`; the tauri CLI re-runs the build script by injecting `TAURI_CONFIG` (`cargo:rerun-if-env-changed=TAURI_CONFIG`). A bare `cargo build --release` can reuse a stale build-script output and embed an outdated or empty frontend — the shell then shows a blank pet/UI. `npx tauri build` runs `npm run build` (tsc + vite) first, so it always embeds the current frontend.

Dev iteration with live reload: keep the vite dev server pinned to the port in `tauri.conf.json` `devUrl` (127.0.0.1:5174) — `cd app && npm run dev -- --port 5174 --strictPort` — then run `AMBERY_PORT=47602 npx tauri dev` from `app/` (the shell embeds its own core server, so `AMBERY_PORT` must not collide with the standalone backend).

**macOS: blank/absent windows after `./target/release/ambery`.** Two distinct causes, both producing "no windows on screen" (check with `swift -e 'import CoreGraphics; CGWindowListCopyWindowInfo(...)'`):

1. **Stale frontend embedded** — a bare `cargo build --release` (not `npx tauri build`) can embed an outdated or empty `dist/`; the shell shows a blank UI. Fix: build via `npx tauri build`.
2. **WebKit cache directory missing** — if `~/Library/Caches/ambery/WebKit/ServiceWorkers/` does not exist, WebKit fails to initialize, the webview never loads (no `[tauri-cmd]` lines in the log), and windows exist but are invisible. Fix: `mkdir -p ~/Library/Caches/ambery/WebKit/ServiceWorkers`, then restart. Symptom in the log: `could not create directory ".../WebKit/ServiceWorkers" ... Operation not permitted`.

## Inspect windows

At runtime, invoke locate to list every ambery window with its position, size, and visibility, leaving a 10s border on the desktop (red visible, lime hidden):

Windows PowerShell:
```bash
pwsh -NoProfile -File tools/locate.ps1 -Highlight
```

macOS:
```bash
swift tools/locate.swift --highlight
```

## Debugging LLM failures

Point the backend at a dead endpoint to exercise the non-silent failure path:

```bash
AMBERY_PORT=47630 cargo run -p ambery-case -- serve --brain-addr http://127.0.0.1:9
```

The round falls back to the debug agent and the UI receives an `llm_error` frame instead of failing silently.
