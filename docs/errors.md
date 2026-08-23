# Error Handling Model

English | [中文](errors.zh.md)

> This document defines ambery's two-layer error model — the **error layer** (what can fail and its nature) and the **display layer** (where each error surfaces). It does not define the current implementation, protocol, or configuration structure.

## Two layers

```
   error layer                  display layer
 (sources × nature)           (where the user sees it)
                        ──►
    事件 / 状态                 气泡 / banner / Card
```

## Error layer

Each error source has a **nature**:

- **Event** — a one-time occurrence (this turn's call failed). Consumed once and cleared.
- **State** — a persistent condition (the active provider is broken). Lasts until the condition is fixed.

| source | nature | example |
|--------|--------|---------|
| LLM call failure | event | HTTP error / timeout on one call |
| LLM init failure | state | active provider misconfigured (no key / empty base_url) |
| message enqueue failure | event | queue full |
| key write failure | event | env file unwritable |
| core unreachable | state | backend offline |

`unconfigured` / `debug` are not errors — they have their own onboarding flow.

## Display layer

Three outlets, each answering one question:

- **Bubble** (chat stream) — "what happened this turn"; event-driven, one per occurrence.
- **Banner** (chat top) — "the system needs attention"; state-driven, persistent entry to the setup guide.
- **Error-frame Card** (pet side) — the standing presence of a degraded state; complements the bubble's one-shot nature.

## Channel

The nature of a source decides its channel:

- **Events** flow as an `llm_error`-style effect (consume-and-clear per turn) → **bubble**.
- **State** is detected independently of events → **banner / Card**.

A source reported through the wrong channel breaks: a persistent state reported as a one-shot event goes silent after the first report; an event re-reported every turn becomes spam.
