# WtAdapter Process Protocol

English | [中文](sidecar.zh.md)

> One implementation of terminal-adapter (docs/terminal-adapter.md §Implementation). This document defines the stdio JSONL protocol, command set, and lifecycle of the standalone WtAdapter process. See spec.md for the technology choice (UIA remains C#, called from Rust).

## Process Model

- sidecar is a standalone console exe (.NET 9, `UseWPF` pulls in the UIA assemblies), distributed with the package in Tauri sidecar mode; during debug it is launched by case-runner via `AMBERY_SIDECAR=<exe path>`.
- **Protocol: stdio JSON Lines** — one JSON request per line on stdin, one JSON response per line on stdout. The Rust `SidecarClient` holds the process handle; a Mutex serializes requests (UIA operations themselves cannot run in parallel — switching tabs is global state).
- Crash handling: each request checks whether the process is alive; if it exited, restart it once and retry; if it still fails, return None (the read path degrades back to Context, and AmberyBackend semantics remain unchanged).

## Packaging (Decided)

- **self-contained win-x64, not single-file**: `sidecar.csproj` pins `RuntimeIdentifier=win-x64` / `SelfContained=true` / `PublishSingleFile=false`; the user's machine does not need the .NET 9 Desktop Runtime.
- Publish command: `dotnet publish sidecar/sidecar.csproj -c Release` → `sidecar/bin/Release/net9.0-windows/win-x64/publish/ambery-uia-sidecar.exe`.
- Tauri side: `bundle.active` is currently false (enabled in the release round). When Windows packaging is enabled, add `../../sidecar/bin/Release/net9.0-windows/win-x64/publish/ambery-uia-sidecar.exe` to `bundle.externalBin` in `app/src-tauri/tauri.conf.json`; do not keep this configuration resident in non-Windows builds — the Tauri build script resolves externalBin paths according to the current platform.
- Path discovery priority (`core/src/paths.rs`): `AMBERY_SIDECAR` env > next to the current exe > `sidecar/` next to the current exe > Release publish > Debug. Before real-machine Windows verification, the publish layout has not been tested by the packaging pipeline (flagged in `dev/issues.md`).

## Command Set

```
→ {"cmd":"list_windows"}
← {"ok":true,"windows":[{"hwnd":12345,"title":"..."}]}

→ {"cmd":"list_tabs","hwnd":12345}
← {"ok":true,"tabs":[{"index":0,"name":"✳ demo-webapp","selected":true}]}

→ {"cmd":"find_tab","name":"demo-webapp"}        # substring match, across all windows
← {"ok":true,"hwnd":12345,"index":2,"name":"✳ demo-webapp"}

→ {"cmd":"read_tab","hwnd":12345,"index":2}          # SelectionItemPattern.Select() switch (~200ms) + read full text
← {"ok":true,"text":"..."}

→ {"cmd":"read_active_tab","hwnd":12345}             # no switch, non-invasive read-only
← {"ok":true,"text":"..."}

→ {"cmd":"count_processes","name":"claude"}         # process count (used for N/M/K three-way reconciliation during startup scan)
← {"ok":true,"count":13}

Unified error: {"ok":false,"error":"..."}
```

- `read_tab` really switches the user's tab (concept §7: 200ms cost) — the semantics of Timer fallback scanning / fetch_terminal is precisely "switch over and read"; `read_active_tab` is reserved for non-invasive scenarios (debugging, quick read of the current window).
- The response text is the raw UIA grid text (right-padded, spinner included); Filter processes it on the Rust side (docs/filter.md) — sidecar does not filter; its responsibility is single.

## Read Path Wiring (Terminal Adapter, docs/terminal-adapter.md)

```
fetch_terminal / Timer scan (instance name = Tab name)
  → find_tab(instance) → not found → None (fall back to latest Context record)
  → read_tab(hwnd, index) → text
```

See concepts §9 for the 1:1 relationship between instance ↔ Tab. **hook→Tab location is solved**: sessionTitle marker (the `<project>·<sid8>` prefix invariant, docs/hook.md §marker location); find_tab hits it exactly by marker; the location result is cached in the registry (lazy retry, frozen once found).

## Visibility Model

**Optional convenience (not mandatory)**: after Windows Terminal enables "Show this application's windows on all desktops", WT windows/tabs/content on all virtual desktops are fully visible and readable by UIA (measured: 17/17 windows uncloaked; a window that read empty recovered 6 tabs + 5415 characters in full). Without the setting everything still works — hooks from instances on other VDs are still received, and the read path falls back to Context; when full content is needed the agent can use `fetch_terminal` (`vd_switch: true` explicit consent) to switch over and read (docs/hook.md §VD switching capability).

Mechanism background: windows on other VDs are marked `cloaked=2` by DWM (shell-level invisibility — the visual tree is suspended, but the process/message loop stays alive); EnumWindows can see the handle + title, but **the UIA tree is not materialized (reads empty)**. The "show on all desktops" setting removes cloaking; VD switching removes it temporarily.

**The real visibility boundary** (not even the setting can save it):

- **Non-Windows-Terminal terminals** (WezTerm / VS Code terminal / ConEmu): the CASCADIA class name does not match, so they are invisible
- **Background tabs of cloaked windows** (when the setting is not enabled): the window title only reflects the active tab, so background tabs are invisible

VD switching is not used as a background path, but it is exposed as an **explicit agent capability** (openness principle).

## Residency and Launch (Simplified Semantics)

At app startup the exe is auto-discovered and enabled (`AMBERY_SIDECAR` env > repository-convention location); the process is launched lazily (spawned on the first request). **Dead means discarded; the next request spawns it fresh** (cold start measured at ~200ms) — no keep-alive pre-check, no heartbeat; each request makes at most two attempts, and if it still fails, returns None (the read path degrades back to Context, and AmberyBackend semantics remain unchanged).

## Decided Approach for the Blocking Boundary

`TerminalAdapter`'s locate/read are synchronous trait calls; AmberyBackend's read entry (`read_terminal`) puts them as a whole into `tokio::task::spawn_blocking` — sidecar's process IO / Mutex waiting / 5s switch rate-limit sleep all run inside the blocking thread pool, not occupying tokio workers. SidecarClient itself remains a synchronous protocol client and does not introduce a nested runtime.
