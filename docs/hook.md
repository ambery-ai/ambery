# Hook Contract (Real Claude Code Integration)

> Concept definition: see concepts.md §9/§9b. This document fixes the real hook contract: event layering, marker positioning, startup scan, installation.
> The mock contract (docs/agent-loop.md §Mock Hook Contract) is retained as a debug tool.
> **Design principle: no technical restrictions, the more open the better** — full capabilities (the agent can switch desktops, the three modes are configurable), conservative defaults, and all choices are left to the user.

## Chain Shape

```
Claude Code 事件 → settings.json "command" hook → ambery-hook.ps1
  → 读 stdin JSON payload → 输出 sessionTitle（定位标记）+ POST /hook（fire-and-forget）
AmberyBackend → register-on-first-sight → 按事件分层处理
内容一律走读通道补（sidecar UIA），hook 只当触发信号（docs/sidecar.md）
```

The hook script is always fire-and-forget (async + short timeout + silent failure) — if the backend is offline it must never block the user's CLI; lost hooks self-heal via register-on-first-sight.

## Payload (POST /hook)

| Field | Source | Description |
|---|---|---|
| `event` | hook_event_name | `session_start` / `user_prompt` / `stop` / `session_end` / `notification` |
| `session_id` | payload | **Instance identity = hash** (same name, different lifecycle; docs/storage.md) |
| `cwd` | payload | project = basename |
| `kind` | carried by the script | `"claude"` (input to the filter per-instance policy, docs/filter.md) |
| `prompt` | UserPromptSubmit | Full text of user input |
| `message` | Notification | Notification text |
| `last_assistant_message` | Stop | Optional reference (content is governed by the read channel) |

## Event Layering

| Event | backend behavior | Landing point | State transition |
|---|---|---|---|
| `session_start` | First-sight registration + positioning probe (see below); EventBuffer records `+ {name} registered` | **EventBuffer** (minimal text) | → Idle |
| `user_prompt` | Prompt observation injection (`[observed] User input in {name}: …`) | **Queue** (trigger; pet may stay silent) | → Processing |
| `stop` | Dual-mode (`stop_hook_mode`, see below) | **Queue** (trigger) | → Idle |
| `session_end` | Clear positioning cache; EventBuffer records `− {name} closed` | **EventBuffer** (minimal text) | → **Closed** (true signal) |
| `notification` | message injection | **Queue** (trigger) | — |

**stop three modes** (`stop_hook_mode`, locally configurable, `no_llm_visible`, hot-reloaded — read fresh on every stop):

- `"queue_only"` (**default B**): stop only injects the hint (a summary of the payload's `last_assistant_message`) into the Queue — the pet decides "silent/curious" from the hint; only when curious does it `fetch_terminal` and read on demand (UIA read happens only when needed).
- `"auto_read"` (A): when stop arrives, UIA grabs the screen → filter → the normalized result updates the in-memory change-detection baseline; what is injected into the Queue is the evaluation prompt (of the form "finished; Context updated (N chars)") — the normalized full text does not enter Queue/Context (docs/storage.md §filtered_content); the pet reads the full text on demand via `fetch_terminal`. `read_tab` **does not switch when the target tab is already selected** (C# side alreadySelected short-circuit, no 200ms wait); it switches only when not selected (**does not switch back**). `read_active_tab` is the non-invasive read-only variant (no switch, no queueing; for debugging / current-window quick reads); **tab-switch throttling: at most once per global 5 seconds**, and switch-read requests inside the window wait for the window (naturally serialized under the UIA Mutex). The whole read round-trip goes through `spawn_blocking` and does not block tokio workers (docs/sidecar.md §blocking boundary).
- `"message"` (C): stop injects the **full text** of `last_assistant_message` as content directly into the Queue — the agent's report goes straight to the pet (zero UIA; the pet reads what the agent itself said, not the screen). Form: `[report] {name} finished: {full text}`, full, not truncated; when empty it degrades to the hint form ("finished, no report content").

**The agent's VD switching capability** (openness principle): not a separate tool, it is a **required field** of `fetch_terminal` — an interruptive decision must not become a forgotten default; every call faces it explicitly:

```
fetch_terminal(instance, vd_switch: bool)   // 必填,忘传报错(失败信息即教学)
  vd_switch=false: 目标在当前 VD → 正常读
                   目标 cloaked → 调用失败,错误提示「目标在另一个虚拟桌面,用 vd_switch=true 重试」
  vd_switch=true:  目标 cloaked → 切到目标桌面 → 读 → 不切回(留在目标桌面,用户自己决定何时回)
                   目标在当前 VD → 字段无效,正常读
```

Timer/stop automatic paths never switch (no background interruption principle).

**SessionStart source variants**: `startup` normal registration; `resume` same session_id → same sid8 → natural upsert reuse (no second entry); `clear`/`compact` do not touch identity, EventBuffer records one line.

**register-on-first-sight**: when any event arrives with an unknown session_id, registration is written first (first_seen = the backend's first-sight moment) before the event semantics — a lost start (backend was offline at the time) is just the ordinary case of "first sight happens to be stop", with no special-case code.

**Processing is driven by "the user assigns work"**, not by "the CLI is open": SessionStart → Idle, UserPromptSubmit → Processing, Stop → Idle, SessionEnd → Closed. Timer's None-death inference is demoted to a fallback for instances without hooks.

## Marker Positioning (Hook → Tab)

**Invariant: the marker prefix is immutable; the descriptive part may evolve.** The sessionTitle output of both hooks follows:

```
SessionStart:      "<project>·<sid8>"
UserPromptSubmit:  "<project>·<sid8> | <prompt 前 N 字>"
```

**UserPromptSubmit must be re-sent (not optional)**: claude automatically names sessions by prompt content (verified: the tab name is auto-generated from the prompt content) — if the marker does not re-assert itself it will be overwritten by the auto name. This is also the marker's **self-healing mechanism**: after the title is overwritten, the user's next prompt revives it.

After claude applies sessionTitle, the tab name carries project+sid8, so sidecar `find_tab` (Contains match; the ✳ prefix and the `| description` suffix do not affect the hit) hits it exactly. The session_title ↔ WT tab name correspondence chain holds (the .last-title cached value and the UIA tab name match pairwise); the WT window title = the active tab title.

**Positioning cache**: a registry entry may carry `{hwnd, index}` — it is an ordinary field of the snapshot (same treatment as status: append is the "update"; the projection yields the current value; no in-place mutation). Lazy retry — at session_start the tab may not have been renamed yet (async application); afterwards each read (timer/stop/fetch) that misses re-searches by marker, and the snapshot naturally picks it up after being found; the session_end closed snapshot sets it to null.

## Startup Scan

backend startup one-shot: list_windows → list_tabs, using the **claude detection rules** (verified 54/54 hits, 0 false positives):

- tab title starts with `✳` (the active glyph of an active claude session), or title == `claude` (unnamed session)
- those **with a marker** (`·<sid8>`) decode project+sid8 and register directly; **those without a marker enter as placeholder identities** (hash = `uia:<tab title>`, kind=claude) — startup sees the whole landscape; when the real identity is later registered (register-on-first-sight), it is correlated by title: the placeholder entry is set closed and the real-identity entry takes over (append log, no in-place modification)
- **three-way reconciliation** (one EventBuffer line reporting truthfully):
  - `N` = number of claude.exe in the Windows process list (including child processes, **a heuristic reference value**, not the session count)
  - `M` = number of claude tabs located by UIA
  - `K` = **number of cloaked windows** (EnumWindows + `DwmGetWindowAttribute(DWMWA_CLOAKED)`; K>0 means some windows are unreadable from other VDs → prompt to enable WT "show on all desktops", docs/sidecar.md §visibility model)

Old sessions opened before the hook was installed do not have their identity guessed; they wait for the register-on-first-sight of their next event. The information form is the same as session_start (EventBuffer).

**timer switch**: `timer.interval_ms ≤ 0 = disabled` (docs/timer.md). For the initial phase of real hook integration, disabling is recommended — keep only hook-driven operation to avoid the LLM trigger frequency of full-instance periodic scans.

## Sidecar Resident (Simplified Semantics)

The app auto-discovers the exe at startup and enables it (path discovery: `AMBERY_SIDECAR` env > the repo-conventional location sidecar/bin/…/ambery-uia-sidecar.exe); the process is lazily started (spawned on first request). **Dead is discarded; the next request starts it fresh** (cold start measured ~200ms) — no pipe keep-alive preflight, no heartbeat; the client implementation is ~55 lines. Crash handling = at most two attempts per request (start once, retry once); if it still fails, return None (the read channel degrades back to Context).

## Install / Uninstall (scripts/install-hooks.ps1)

- **install**: copy the hook script to `~/.claude/hooks/ambery-hook.ps1`; append five command entries — SessionStart / UserPromptSubmit / Stop / SessionEnd / Notification — to `~/.claude/settings.json` (**append**; existing user hooks untouched); back up `settings.json.bak` before changing.
- **uninstall**: remove the five marked entries + delete the script; the rest of settings stays as-is.
- the hook script lives in the repo (`scripts/ambery-hook.ps1`, generic, no privacy); **real samples / measured data do not enter the repo** (privacy; measured data belongs to the user).

## Explicitly Out of Scope

- PreToolUse / PostToolUse / PreCompact / SubagentStop: not needed at the current granularity.
- Notification dedup: v1 triggers all; AGENTS.md teaches the pet that silence is the norm; if real-world use finds it noisy, add a time window (config-configurable).
- opencode hook: different system, deferred (docs/filter.md open question).
- hook-supplied content (transcript parsing) not adopted — the read channel is the single source (privacy surface + dual content forms).
