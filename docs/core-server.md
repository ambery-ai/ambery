# Core Server

English | [中文](core-server.zh.md)

A thin in-process HTTP server inside the Tauri process, bound by default to `127.0.0.1:47600`, carrying only the external hook intake.

## Port Semantics

- Default `47600` is the delivery contract for hook scripts; `AMBERY_PORT` may explicitly override it (changing the port must update the hook configuration accordingly).
- **No random fallback**: when the port is occupied, print a readable error and exit — randomly changing the port would make external hooks silently fail to deliver, which is harder to diagnose than a startup failure.

## Loopback Threat Model (Decided)

No token. The boundary is **only `127.0.0.1` + trusting same-machine users**:

- `/hook` and the debug router do not listen on external interfaces; any process that can POST to loopback already has current-user privileges and can directly read/write `~/.claude/settings.json`, storage/config files, and all user files — under this threat model, a token only adds configuration cost, not an actual security boundary.
- This is common practice for local development tools (local DB/REPL/CLI services are isomorphic). Public release does not change the conclusion; if a cross-device or remote access surface is added in the future, authentication will be introduced then.

## Responsibilities

- **Only**: external hook scripts POST to `/hook` (PowerShell, out-of-process. Tauri commands cannot be invoked cross-process)

## Out of Scope

The following responsibilities are carried by Tauri native capabilities:

| Responsibility | Carrier |
|---|---|
| Frontend HTTP API (state/context/config/events) | `#[tauri::command]` + `invoke()` |
| effects broadcast + WS push | `app_handle.emit()` + frontend `listen()` |
| timer background tasks | Tauri async runtime `spawn` (already inside the runtime) |

## Routes

| Method | Path | Purpose |
|---|---|---|
| POST | `/hook` | external hook script trigger (fire-and-forget, the only HTTP port kept) |

## Frontend Communication

Tauri native IPC (not through 47600):

- **Frontend → Rust**: `invoke("get_state")` / `invoke("append_user", {text})` and other Tauri commands
- **Rust → Frontend**: `app_handle.emit("effect:render_component", spec)` → frontend `listen()` receives

## Debug Mode Full Router

case-runner starts with the full `router()` (`ambery-case serve`, browser debugging consumed by RemoteBridge, docs/case-runner.md §CLI): `/state` `/context` `/queue/user` `/events` `/config` `/config/schema` `/effect` `/ws`, plus:

- `GET /cards`, `POST /cards/layout`, `POST /cards/user_closed` — the same core logic as Tauri commands `list_cards` / `update_card_layout` / `set_card_user_closed` (shared across the dual transports), consumed by RemoteBridge (TS subprocess / browser debugging)
- `POST /debug/effect` (`case-runner` feature; not included in release default builds) — injects arbitrary effect messages into the effect downlink bus, deterministically driving render/close/config events in headless tests, without going through the LLM
- Default port 47600, `AMBERY_PORT` can override (sandboxes use an independent port to avoid production); storage/config directories are isolated via `AMBERY_STORAGE_DIR` / `AMBERY_CONFIG_DIR`

## Related Documents

- `agent-loop.md` §Protocol: Tauri IPC + `/hook` path
- `docs/agents/claude/hook.md`: external hook script integration
- `docs/terminal/timer.md`: timer background task logic
