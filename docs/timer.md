# Timer Design

English | [中文](timer.zh.md)

> See concepts.md §1a for the concept definition. This document defines the scheduling mechanism, the stagger algorithm, and where the scan action applies.

## Positioning

hook is the primary channel; Timer is the observation cycle and fallback. When an instance's hook has not fired for a long time, Timer does one catch-up scan — read Terminal Content → Filter → change detection → only inject into the Queue for evaluation when there is substantive change (Example C: "config-service's last hook did not fire, but the Timer fallback scan has already updated its Context"). The scan also delivers read evidence to the instance's belief state (concepts §9a): `Content` refreshes the belief as alive, a confirmed `Gone` is death evidence, `Error` is not an observation — Timer never infers life or death from elapsed time; absence of evidence only moves a belief toward `unknown`.

**Switch**: `timer.interval_ms ≤ 0 = 禁用` (Config, configurable from panel/CLI). It is recommended to disable it in the early stage of real hook integration — keep only the hook drive and avoid the LLM trigger frequency caused by periodic full-instance scans; use a positive value during mock debugging.

## Config fields

All cold fields: take effect after app restart; visible to and modifiable by the agent. TimerWheel / main loop are built at startup and are not rebuilt at runtime.

| Field | Default | Takes effect | Agent access | Semantics |
|---:|---|---|---|---|
| `timer.interval_ms` | 300000 (5 minutes) | cold | visible, modifiable | per-instance fallback scan interval; ≤ 0 = disabled |
| `timer.stagger_ms` | 30000 (30 seconds) | cold | visible, modifiable | stagger window: due times of multiple instances are spread within this window |
| `timer.tick_ms` | 60000 (60 seconds) | cold | visible, modifiable | main loop granularity: wakes each tick and takes due instances (if interval is smaller than this, it scans at most once per tick); legal lower bound ≥ 100 |
| `timer.batch` | 2 | cold | visible, modifiable | maximum number of instances scanned per tick (rate limiting); legal lower bound ≥ 1 |

## Scheduling (TimerWheel)

```rust
struct TimerWheel {
    interval_ms: i64,               // 兜底扫描间隔（Config，默认 5 分钟）
    stagger_ms: i64,                // 错峰窗口（Config，默认 30s）
    next_due: HashMap<String, i64>, // instance → 下次到期时间
}
```

- **Stagger**: `due(instance) = now + interval + hash(instance) % stagger`. The deterministic hash (instance name) guarantees the same offset for the same instance each time and naturally spreads different instances apart, avoiding simultaneous scans (concepts §1a "staggered distribution").
- **hook arrives → reset**: `handle_hook` calls `reset(now)` for the triggering instance (recomputes interval + stagger offset) — hook is the primary channel, and instances with recent hooks should not be catch-up scanned.
- **Due extraction**: `due(now, batch)` returns the due instances (at most batch at a time); once taken, they are rescheduled to `now + interval + stagger`; the rest remain due and are taken on the next tick — the batch cap is also a form of staggering.

## Scan action (Terminal Adapter read)

The scan read channel = Terminal Adapter (docs/terminal-adapter.md): `locate(instance) → read(tab)` reads the current Terminal Content; None if it cannot be read. For the per-terminal implementations (WtAdapter / MapAdapter / Composite dispatch) and assembly gating (`terminal.adapter_*`), see that document.

- case-runner scenario side: the `terminal` step writes the shared map of MapAdapter to **simulate "what the terminal currently displays"**. Symmetric with the mock hook: hook simulates the push channel, the terminal scenario simulates the read channel.
- The `fetch_terminal` tool reads the same adapter (falls back to the latest Context record when unreadable) — there is only one read channel.

## Scan processing flow (the real application point of change detection)

```
tick (server background task, default 60s (config `timer.tick_ms`; case-runner can override via AMBERY_TIMER_TICK_MS))
  → due(now, batch)
  → adapter read by located tab (three states: Content = evidence of liveness / Gone = confirmed absence → closed evidence / Error = skip this round, belief unchanged — docs/storage.md)
  → raw text archived to terminal-content.jsonl → Filter.digest normalization
  → detect_change against the in-memory prev baseline (normalized full text is not persisted; prev lives in memory, lost on restart)
  → Substantive: inject "{instance} fallback scan detected changes, Context updated ({len} chars). Evaluate whether to notify." into Queue (source=timer_scan) → run_trigger (the normalized full text itself does not enter Context)
  → Minor / Unchanged: raw archive + prev update, no disturbance (consistent with the concepts §9b silence spirit)
```

The injected message is isomorphic to the stop hook (`…，Context 已更新（N 字）。评估是否通知。`) — the notification/silence decision path is identical.
