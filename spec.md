# Spec v1 — Tech stack

English | [中文](spec.zh.md)

> Document responsibilities / navigation: see [docs-spec.md](docs-spec.md).
> Phase: spec1 (tech selection settled, division of work initially set). For concept definitions see [concepts.md](concepts.md); this file only sets the technical division of work and does not repeat concepts.

## Technology choices

| Layer | Technology | Concepts covered | Responsibility |
|---|---|---|---|
| UI | TypeScript (no framework, vanilla) + Tauri 2 frontend | View, pet (expression), Component, Chat Panel, Autonomy (presentation) | Floating ellipse window, kaomoji rendering, Card popup and direction selection, Chat Panel, right-click snapping |
| System | Rust (Tauri backend, single process) | Ambery, Timer, Harness (Queue / Context / Event Buffer / Compression), Filter, Tool Set execution | Instance lifecycle, LLM call loop, ordered message processing, HTTP port listening, scheduling the UIA sidecar |
| UIA reading | C# (.NET, separate sidecar process) | Terminal Window / Tab / Content, Status determination | Enumerate windows, switch tabs, read TermControl full text, state-machine determination — directly reusing exp01 verified code |
| hook | PowerShell script | hook | Claude Code `"type": "command"` hook → read stdin JSON → POST to AmberyBackend's local port (consistent with the existing ~/.claude/hooks/ ecosystem, zero interpreter dependency) |
| Persistence | JSONL files + single-file Config | Storage, Config | append-only: `queue.jsonl` / `context.jsonl` / `work-agents.jsonl`; `AGENTS.md` pet identity prompt; `config.json` separate, loaded at startup |

## Architecture decisions

1. **Single process + single protocol**: the Tauri app is Ambery (embedded ambery-core bound to 127.0.0.1). The frontend always communicates with core via **HTTP + WebSocket loopback** — browser debug mode connects to a standalone core debug binary, Tauri mode connects to the embedded server, and the frontend code is unchanged. Tauri commands/events are not used (reason in the final section of docs/harness.md).
2. **UIA reading**: keep C# (exp01 verified, no rewrite). Compiled as a standalone console exe and distributed as a Tauri sidecar with the package; Rust calls it via stdio (JSON Lines request/response), such as `read_tab`, `list_windows`, `switch_tab`. **Packaging decision**: self-contained win-x64 (not single-file), zero .NET runtime dependency for users; publish command `dotnet publish -c Release` (RID/self-contained already fixed in sidecar.csproj), Tauri `externalBin` references the publish layout (docs/sidecar.md §Packaging).
3. **hook path**: use `"type": "command"` + **PowerShell script** forwarding (consistent with the existing ~/.claude/hooks/ ecosystem, zero interpreter dependency), not `"type": "http"`. AmberyBackend's embedded HTTP listener receives the POST.
4. **LLM calls**: Rust side, OpenAI-compatible Chat Completions endpoint; base_url / key read from Config.
5. **Storage**: always append-only JSONL; restart replays to restore Queue / Context / instance list. Switch to SQLite later when query needs arise.
6. **Config**: single-file JSON, loaded at runtime; the `edit_config` tool writes it back. Config and Storage are separated (concepts §12/§13): `%USERPROFILE%\.config\ambery\config.json` + sibling `storage/`, paths resolved by core/paths.rs (`AMBERY_CONFIG_DIR` / `AMBERY_STORAGE_DIR` can override).

## Fixed constraints

- HTTP listener binds only to 127.0.0.1
- UIA sidecar communication protocol: stdio JSON Lines (docs/sidecar.md)
- No Python interpreter dependency (use PowerShell)
- Frontend framework: vanilla TS (display logic is simple, no framework; browser mode can run vite dev directly and test with Chrome DevTools)
- **hook payload contract (docs/hook.md)**: command script forwarding, session_id identity, marker location
