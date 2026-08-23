# Error Handling Model

English | [中文](errors.zh.md)

> This document defines ambery's error presentation model — errors as notifications. It does not define the current implementation, protocol, or configuration structure.

## Model

An error is a notification — the user learns from the UI, nothing more. Errors do not come in kinds; they differ only in how long they stay visible (retention). `unconfigured` is an error notification like any other — a persistent condition needing action; only the explicit `debug` mock mode produces no notification.

## Sources

| source | retention | action | example |
|--------|-----------|--------|---------|
| LLM call failure | transient | — | HTTP error / timeout on one call |
| LLM init failure | persistent | setup | active provider misconfigured (no key / empty base_url) |
| message enqueue failure | transient | — | queue full |
| key write failure | transient | — | env file unwritable |
| core unreachable | persistent | — | backend offline |
| unconfigured | persistent | setup | no provider configured |

## Presentation

Two outlets, one per retention:

- **Bubble** (chat stream) — transient: "what happened this turn". One per occurrence, consumed and cleared.
- **Banner** (chat top) — persistent: "the system needs your attention". Opened by an error event with persistent retention, stays until dismissed. An action is optional: with one, click dispatches to the destination; without one, the banner is a pure persistent notice (dismissible only). At most two banners show at once — one per condition (repeat events of a condition claim no extra slot); excess conditions queue and slide into a slot when one is dismissed.

Retention lives in the outlet, not in a separate state value: the same error event opens both the transient bubble and the persistent banner. A persistent condition must reach the persistent outlet — a transient bubble alone would surface it once and go silent.

Cards are a component-rendering concern, not an error outlet.

## Channel

```
 error sources (any, see table above)
      │
      ▼
error event { message, retention, action? }
      │
      ├─ persistent ──► Banner (≤2 on screen, one per condition, excess
      │                     queue slides up on dismiss; action? optional —
      │                     present: click dispatches; absent: pure notice)
      └─ transient ────► Bubble (one per occurrence, consumed and cleared)
```

All errors flow through the event channel as `error` events. Each event carries `message` (user-facing text), `retention` (`transient` | `persistent`), and an optional `action` — a destination id (initial set: `setup`, the setup guide). The frontend routes by retention: persistent opens the banner, transient shows a bubble only; an action, when present, decides the banner's click behavior. The source is not part of the contract — the frontend never branches on it.

A persistent condition is emitted at its change points (startup, config or key change, call outcome), not re-emitted every turn; the banner treats repeat events as no-ops.
