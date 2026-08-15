# Cron Design

> Concept definitions are in concepts.md §10g. This document specifies task representation, persistence format, due behavior, and
> the call contracts for the three tools cron_create / cron_delete / sleep.

## Principles

> **Scope of this document** — this document defines Cron's task model, cron.jsonl format, scheduling implementation, and the parameters/validation/returns of the three tools; for Cron's conceptual positioning and ownership see concepts.md §10g / docs/harness.md §Cron; for storage layout see docs/storage.md §Cron.

> **Design constants** — the sleep cap and the scheduling poll granularity are implementation constants (values defined in this document) and do not enter Config.

> **No special rules; use semantics to make behavior explicit** — the schedule is given explicitly as one of two choices; having no list tool is an established boundary (concepts §10a lists only two Cron tools), and no implicit query channel is invented.

## Task Representation

Cron entry:

```json
{
  "id": "a1b2c3d4",
  "schedule": { "at": 1785600000000 }            // one-shot (epoch ms)
              | { "every_ms": 86400000 },         // fixed interval (anchored to creation time)
  "message": "现在是每天夜间日报时间：请汇总今日各实例进展。"
}
```

- `schedule` is one of two: `at` (one-shot epoch ms) or `every_ms` (fixed interval, first due = creation time + every_ms). Cron expressions are not supported (to be revisited when wall-clock semantics are needed).
- `message`: the `system` input content injected into the Queue when due (isomorphic with hook content, concepts §10c).
- The payload currently has only the message form; more complex due-time actions are initiated by the Agent itself after being woken by the message (sleep-then-act scenarios are expressed by the `sleep` tool, not through the Cron payload).

## Persistence (cron.jsonl)

append-only event lines, folded by replay into the current schedule set:

```json
{"op":"create","id":"a1b2c3d4","schedule":{"every_ms":86400000},"message":"日报","next_due":1785600000000,"ts":1785513600000}
{"op":"fire","id":"a1b2c3d4","next_due":1785686400000,"ts":1785600000123}
{"op":"delete","id":"a1b2c3d4","ts":1785600100000}
```

- `create`: create; `next_due` initially = at, or creation time + every_ms.
- `fire`: due and dispatched. `every_ms` reschedules `next_due += every_ms` (multiple fires advance one by one); the fire line of `at` has `next_due: null` (finished state, no longer scheduled, log retained).
- `delete`: remove (tombstone).
- replay folding: create inserts, fire updates next_due, delete removes; when next_due is null or the entry does not exist, it is not scheduled.

## Scheduling Implementation (Shared by Cron and sleep)

`CronScheduler` (core/src/cron.rs) is the only scheduling implementation in Harness, managing two kinds of tasks:

- **entries**: persisted schedules (previous section); the server background task polls every 500ms for due entries → due messages enter the Queue as `system` input (isomorphic with hook, fire-and-forget waking the single consumer).
- **waiters**: sleep's non-persistent one-shot waits (loss on crash is acceptable, same as Queue-unadmitted input); registration returns a oneshot, and the scheduling poll notifies when due.

waiters are accessed through an independent shared handle (not through the AmberyBackend lock) — while sleep occupies the Queue serialization point waiting, the scheduling task must still be able to wake it when due (no deadlock).

## sleep

Wait through the Harness scheduler and then continue the planned tool sequence: the tool result returns late, and subsequent tool calls of the same response continue after the wait.

| Parameter | Type | Required | Validation |
|---|---|---|---|
| `ms` | integer | ✓ | 0 ≤ ms ≤ 300000 (5 minutes, design constant) |

**return**

| Case | Return |
|---|---|
| Wait finished | `{"ok": true, "slept_ms": <ms>}` |
| Parameter error | `{"ok": false, "error": "…"}` |

Semantic boundaries:

- During sleep the Queue serialization point is occupied (concepts §10c) — waiting is part of the Agent's planned behavior, and the duration cap prevents mistakes.
- sleep is not persisted: it is lost on crash and not reissued.
- `ms: 0` = return immediately (yield the current execution point once).

## cron_create

Create a persisted schedule (the Agent's entry point for adjusting Cron; backend/users may directly edit cron.jsonl to manage).

| Parameter | Type | Required | Validation |
|---|---|---|---|
| `schedule.at` | integer | one of two | epoch ms; must be later than the current time; passing it together with `every_ms` is rejected |
| `schedule.every_ms` | integer | one of two | > 0 and ≤ 2592000000 (30 days, design constant) |
| `message` | string | ✓ | non-empty (the `system` input injected into the Queue when due) |

**return**

| Case | Return |
|---|---|
| Success | `{"ok": true, "id": "<id>"}` (short hash id; the Agent may record it via write_memory) |
| Parameter error | `{"ok": false, "error": "…"}` |

## cron_delete

| Parameter | Type | Required | Validation |
|---|---|---|---|
| `id` | string | ✓ | id of an existing schedule |

**return**

| Case | Return |
|---|---|
| Success | `{"ok": true, "deleted": "<id>"}` |
| Not found | `{"ok": false, "error": "计划 '<id>' 不存在（cron 无 list tool；id 见 create 返回或 cron.jsonl）"}` |

## No list tool (established boundary)

concepts §10a lists only `cron_create` / `cron_delete`: the Agent manages its own schedules via the id returned by create (and may write it to Memory long-term memory); users and the backend can directly view/edit cron.jsonl. No implicit query channel is invented for the Agent.
