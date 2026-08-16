# <img src="app/src-tauri/icons/icon.png" width="52" height="52" alt="ambery icon"> Ambery

Ambery is a desktop agent harness that supervises your Claude Code sessions and turns them into a calm, glanceable companion: a floating pet, chat panel, and persistent cards. It watches session lifecycle through Claude Code hooks, reads terminal state through a Windows UIA sidecar, and lets the agent act through a small, explicit tool set.

## What it does

- **Hook-driven supervision** — Claude Code `SessionStart` / `UserPromptSubmit` / `Stop` / `SessionEnd` / `Notification` hooks feed a local backend (loopback only).
- **Agent loop with a pet UI** — Queue → Context → LLM → effects; the pet decides notify vs. stay silent, uses kaomoji states, and renders cards via `call_component`.
- **Full-fidelity storage** — append-only JSONL logs make the OpenAI request context almost fully reconstructable.
- **Observability** — `ambery-case` replays storage snapshots and asserts concept-structure invariants; `ambery-activity` is a TUI viewer with a turn-aware trajectory mode.
- **Windows UIA sidecar** — optional enhanced terminal reading for Windows Terminal (self-contained win-x64, no .NET runtime needed).

## Platform matrix

| Platform | Status |
|---|---|
| Windows 10/11 | First-class: Tauri shell + tray + UIA sidecar + hook installer |
| macOS | Core compiles and runs; Hook-driven experience; no UIA sidecar |
| Linux | Core compiles and runs; Hook-driven experience; no UIA sidecar |

## Quickstart

Prerequisites: Rust stable, Node 24 + npm, and (Windows only) .NET 9 SDK for the UIA sidecar.

```bash
# Rust workspace (core + case runner + activity TUI)
cargo test --workspace

# Frontend headless case (mock/keyless, embeds core and runs vitest)
cargo run -p ambery-case -- frontend --silent

# Install Claude Code hooks (Windows PowerShell)
powershell -File scripts/install-hooks.ps1

# Inspect storage with the trajectory TUI
cargo run -p ambery-core --bin ambery-activity -- --dir ~/.config/ambery/storage --trajectory
```

Browser debug server:

```bash
cargo run -p ambery-case -- serve --silent
cd app && npm install && npm run dev
```

## Repository map

- `core/` — Rust core: Harness, backend, server, storage, filters, TUI activity viewer
- `ambery-case/` — storage snapshot replay and concept-observation runner
- `app/` — vanilla TypeScript frontend; `app/src-tauri/` Tauri shell
- `sidecar/` — C# Windows UIA sidecar
- `docs/` — per-domain design docs; `concepts.md` terminology; `spec.md` architecture decisions
- `dev/` — development records

## Documentation

Start with `concepts.md`, `spec.md`, and `docs/AGENTS.md`. Contribution guide: `CONTRIBUTING.md`.

## License

MIT
