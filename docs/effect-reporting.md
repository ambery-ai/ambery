# Effect Reporting (Tauri Runtime Action Reporting)

> Concept: all **non-readonly Tauri runtime actions** uniformly enter the effect stream (docs/case-runner.md
> §Tauri runtime actions observable: `(Tauri runtime actions − readonly) ⊆ effects`). Tauri runtime actions cover both the WebView `@tauri-apps/api` and the Rust shell `tauri` API; for the action-stream format and record points see `docs/storage.md` §effect.jsonl. This document defines the runtime action layer, channels, kind/payload, packing rules, and the instrumentation checklist.

## Principles

> **Every non-readonly action enters, readonly actions never enter** — side-effecting Tauri runtime actions are uniformly reported into the action stream; pure read calls do not enter.

> **One action, one record** — each non-readonly Tauri runtime action has its own effect; when one call causes several actions, record each separately, and do not merge them under a coarse call name.

> **High-frequency packing** — high-frequency actions of the same kind are debounced and merged by target into one record; the concrete list and window parameters are in the contract below.

> **Simulated environments do not enter** — the principle covers only real runtime calls; DOM simulation operations in the debug environment (browser) are not reported.

> **Actions and traces share one exit** — all non-readonly Tauri runtime actions may only be executed through the semantic runtime action layer; that layer writes the corresponding effect after the action succeeds, and a failed call does not write an "it happened" effect.

## Runtime Action Layer

The only exit for non-readonly Tauri runtime actions is the same-named action layer on both sides:

```text
app/src/tauri_runtime_actions.ts       ← WebView `@tauri-apps/api` 的写动作
app/src-tauri/src/tauri_runtime_actions.rs
                                        ← Rust 壳 `tauri` API 的写动作
```

They share the **action vocabulary and kind/payload contract**, not cross-language code. Business entry points only orchestrate semantic actions; they do not call Tauri write APIs directly and do not hand-assemble effect kind/payload:

```text
业务入口
  → hide_window("pet")              → window_hidden {window:"pet"}
  → show_window("chat")             → window_visible {window:"chat"}
  → close_window("card-x")          → window_closed {window:"card-x"}
  → emit_event("cards:hide")        → event_emit {event:"cards:hide"}
  → create_card_window(...)          → window_opened {window:"card-x"}
```

The action layer records only after executing the real Tauri API:

```text
运行时动作
  ├─ 成功 → 恰好一条对应 effect（高频打包例外见下）
  └─ 失败 → 返回/处理原错误；不写“动作已发生”的 effect
```

When one business call triggers multiple runtime actions, forward and record them one by one. For example, hiding pet, hiding chat, sending `cards:hide` and `pet:hidden` are four actions and must not be merged into one `toggle` effect. Read-only calls are not part of the action layer: `getByLabel`, `outerPosition`, `currentMonitor`, `listen`, etc. may be used directly at the call site.

The Tauri write operations in `window-adapter.ts` are the window part of the WebView action layer (absorbed into the same-named action layer; no second layer is stacked on top of it).

#### ⟡ Consistency Analysis

The WebView `@tauri-apps/api` and the Rust shell `tauri` API are two implementation faces of the same kind of Tauri runtime action; kind, payload, or `origin` must not split just because the code lives in different places. Both sides share the action vocabulary through the same-named semantic action layer: record the corresponding effect after success; on failure do not manufacture "it happened" evidence; compound entry points forward per action and must not substitute a coarse `toggle` effect for several window and event actions. What the action layer unifies is non-readonly actions and their traces; pure reads may still be called directly, avoiding extra wrappers for side-effect-free queries.

## Channels

Tauri runtime actions fall into two classes, with different channels:

| Class | Examples | Channel |
|---|------|------|
| Write actions that call core | WebView invoke `append_user` / `push_event` / `set_config` / `update_card_layout` / `set_card_user_closed` / `ensure_card_window` / `close_card_window` / `export_theme` / `import_theme` | **Not reported separately** — the core receiving end (both transport layers: the HTTP handler and the Tauri command) records on receipt, origin=frontend |
| Runtime actions that do not go through core | WebView WebviewWindow create/close, setSize/setPosition, show/hide, startDragging, emit/emitTo; the equivalent WebviewWindow / AppHandle actions in the Rust shell | Recordable only by the runtime action layer: WebView via the `record_effect` Tauri command (production) / `POST /effect` (debug HTTP, also for tests); the Rust shell writes directly to the same record entry |

- Both channels ultimately share a single record point: `harness.log_effect(Frontend, kind, payload, now_ms)` writes effect.jsonl; `origin=frontend` means the Tauri UI side, without distinguishing whether the implementation lives in the WebView or the Rust shell.
- WebView reporting is fire-and-forget: no await, errors swallowed (reporting failure does not break window logic); Rust shell recording likewise must not block its main action.

## kind / payload

| kind | Meaning | payload | Frequency |
|------|------|---------|------|
| user_message | append_user (endpoint-recorded) | {text} | low |
| interaction | push_event (endpoint-recorded) | {desc, card_id?} | low |
| config_update | set_config (endpoint-recorded; the LLM edit_config path remains config_changed/backend) | {path} | low |
| card_layout | update_card_layout (endpoint-recorded; Card layout write-back, docs/components.md §Card file) | {id, manual} | low |
| card_visibility | set_card_user_closed (endpoint-recorded; Cards Shelf show/hide toggle writes the display choice) | {id, user_closed} | low |
| expression_changed | Autonomy expression/motion actually changed (explicit override/fallback/derived semantics; unchanged is not recorded; window_resized does not infer it sideways) | {face, motion, source: set_autonomy\|revert\|derive} | low |
| window_opened | Rust shell `ensure_card_window` create branch (window decision hoisted, docs/case-runner.md) | {window} | low |
| window_closed | Rust shell `close_card_window` (agent close / shelf dismiss / user × — three paths converge) | {window} | low |
| window_resized | WebView / Rust shell `setSize` / setOffset | {window, w?, h?, top?, left?, count?} | packed |
| window_moved | WebView / Rust shell `setPosition` | {window, x, y, count?} | packed |
| window_drag | WebView `startDragging` | {window} | low |
| window_visible | WebView / Rust shell `show` | {window} | low |
| window_focused | WebView `setFocus` / Rust shell focus equivalent action (the second action inside showWindow; menu foreground focus grab) | {window} | low |
| window_hidden | WebView / Rust shell `hide` | {window} | low |
| theme_export | `export_theme` (endpoint-recorded; theme export file side effect, docs/theme.md) | {name} | low |
| event_emit | WebView / Rust shell `emit` / `emitTo` | {event, target?, count?} | packed |

Packing: key = kind + distinguishing key (window name / event name); after a 250ms quiet period flush one record; the payload keeps
the last value and appends `count` (number of merged records).

## Runtime Action Layer Coverage Table

The same runtime action is recorded exactly once, at the place where it is actually executed; when one call triggers several actions, record each one. For example, when the Rust shell toggle hides pet / chat and emits two events, it must write two `window_hidden` and two `event_emit` records, not one `toggle`. The table below is the current action-layer coverage.

| Action layer | Semantic action | Current call sites / coverage |
|------|------------|------------------|
| WebView | resize_window / move_window / show_window / hide_window | setSize/setOffset, setPosition, show/hide in `window-adapter.ts`; shared by the pet/card/chat windows, window name taken from getCurrentWindow().label |
| Rust shell | ensure_card_window / close_card_window | pet/card/shelf three paths (agent render/close, shelf show-hide/dismiss, user ×) triggered via the WebView action layer invoke; Rust command endpoint records: create→window_opened, reuse→event_emit(card:spec), close→window_closed |
| WebView | start_dragging | `windows/pet.ts`, `windows/card-window.ts`, `windows/chat-window.ts`; corresponds to window_drag |
| WebView | emit_event | emit / emitTo in `windows/pet.ts`, `positioning/tauri-server.ts`; corresponds to event_emit |
| WebView | hide_window | menu hide in `windows/menu.ts`; corresponds to window_hidden |
| Rust shell | show_window / hide_window / close_window / emit_event | toggle, tray close, and other equivalent WebviewWindow / AppHandle actions in `app/src-tauri`; each corresponds to window_visible / window_hidden / window_closed / event_emit |
| Browser simulation | — | browser adapter / drag.ts / component-manager.ts **do not enter the action layer and are not instrumented** (DOM simulation, not Tauri runtime actions) |

## Explicitly Out of Scope

- **No replay for effect.jsonl**: the action stream is an observation log and is not restored at startup (consistent with the other trace files;
  effect lines do not enter memory).
- **No effect section in case data**: a case starts its story from an empty effect.jsonl; runtime records are produced by steps
  (backend) — frontend reporting belongs to the real runtime environment and is unreachable under headless (case-runner.md §boundary isolation).
