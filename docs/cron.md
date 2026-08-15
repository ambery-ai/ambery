# Cron 设计

> 概念定义见 concepts.md §10g。本文档定任务表示、持久化格式、到点行为与
> cron_create / cron_delete / sleep 三个 tool 的调用契约。

## 原则

> **本文档范围**——本文定义 Cron 的任务模型、cron.jsonl 格式、调度实现与三个 tool 的参数/校验/返回；Cron 的概念定位与所有权见 concepts.md §10g / docs/harness.md §Cron；存储布局见 docs/storage.md §Cron。

> **设计常量**——sleep 上限与调度轮询粒度是实现常量（本文定义值），不进 Config。

> **不做特殊规则，用语义明确行为**——schedule 二选一显式给出；无 list tool 是既定边界（concepts §10a 只列两个 Cron tool），不发明隐式查询通道。

## 任务表示

Cron entry：

```json
{
  "id": "a1b2c3d4",
  "schedule": { "at": 1785600000000 }            // 一次性（epoch ms）
              | { "every_ms": 86400000 },         // 间隔周期（锚定创建时刻）
  "message": "现在是每天夜间日报时间：请汇总今日各实例进展。"
}
```

- `schedule` 二选一：`at`（epoch ms 一次性）或 `every_ms`（固定间隔周期，首次到期 = 创建时刻 + every_ms）。cron 表达式不支持（需要 wall-clock 语义时再议）。
- `message`：到点注入 Queue 的 `system` 输入内容（与 hook 内容同构，concepts §10c）。
- payload 当前只有 message 形态；更复杂的到点动作由 Agent 被 message 唤醒后自行发起（sleep-then-act 场景由 `sleep` tool 表达，不走 Cron payload）。

## 持久化（cron.jsonl）

append-only 事件行，replay 折叠为当前计划集：

```json
{"op":"create","id":"a1b2c3d4","schedule":{"every_ms":86400000},"message":"日报","next_due":1785600000000,"ts":1785513600000}
{"op":"fire","id":"a1b2c3d4","next_due":1785686400000,"ts":1785600000123}
{"op":"delete","id":"a1b2c3d4","ts":1785600100000}
```

- `create`：新建；`next_due` 初始 = at，或创建时刻 + every_ms。
- `fire`：到点已发放。`every_ms` 重排 `next_due += every_ms`（多次 fire 逐次推进）；`at` 的 fire 行 `next_due: null`（完成态，不再调度，日志保留）。
- `delete`：移除（tombstone）。
- replay 折叠：create 插入、fire 更新 next_due、delete 移除；next_due 为 null 或 entry 不存在即不调度。

## 调度实现（Cron 与 sleep 共用）

`CronScheduler`（core/src/cron.rs）是 Harness 的唯一调度实现，管两类任务：

- **entries**：持久化计划（上节），server 后台任务每 500ms 轮询 due → 到点 message 作 `system` 输入入 Queue（与 hook 同构，fire-and-forget 唤醒单消费者）。
- **waiters**：sleep 的非持久化一次性等待（崩溃丢失可接受，与 Queue 未放行输入同理）；注册返回 oneshot，调度轮询到点通知。

waiters 经独立共享句柄访问（不经过 AmberyBackend 锁）——sleep 占用 Queue 串行点等待时，调度任务必须仍能到点唤醒它（无死锁）。

## sleep

通过 Harness 调度器等待后继续既定工具序列：tool result 延迟返回，同一 response 的后续 tool call 在等到后继续。

| 参数 | 类型 | 必填 | 校验 |
|---|---|---|---|
| `ms` | integer | ✓ | 0 ≤ ms ≤ 300000（5 分钟，设计常量） |

**return**

| 情况 | 返回 |
|---|---|
| 等待结束 | `{"ok": true, "slept_ms": <ms>}` |
| 参数错误 | `{"ok": false, "error": "…"}` |

语义边界：

- sleep 期间 Queue 串行点被占用（concepts §10c）——等待是 Agent 既定行为的一部分，时长上限防呆。
- sleep 不持久化：崩溃即丢失，不补发。
- `ms: 0` = 立即返回（让出当前执行点一次）。

## cron_create

创建持久化计划（Agent 调整 Cron 的入口；后端/用户可直接编辑 cron.jsonl 管理）。

| 参数 | 类型 | 必填 | 校验 |
|---|---|---|---|
| `schedule.at` | integer | 二选一 | epoch ms；须大于当前时刻；与 `every_ms` 同传拒绝 |
| `schedule.every_ms` | integer | 二选一 | > 0 且 ≤ 2592000000（30 天，设计常量） |
| `message` | string | ✓ | 非空（到点注入 Queue 的 system 输入） |

**return**

| 情况 | 返回 |
|---|---|
| 成功 | `{"ok": true, "id": "<id>"}`（id 短 hash；Agent 可经 write_memory 记录） |
| 参数错误 | `{"ok": false, "error": "…"}` |

## cron_delete

| 参数 | 类型 | 必填 | 校验 |
|---|---|---|---|
| `id` | string | ✓ | 存在的计划 id |

**return**

| 情况 | 返回 |
|---|---|
| 成功 | `{"ok": true, "deleted": "<id>"}` |
| 未找到 | `{"ok": false, "error": "计划 '<id>' 不存在（cron 无 list tool；id 见 create 返回或 cron.jsonl）"}` |

## 无 list tool（既定边界）

concepts §10a 只列 `cron_create` / `cron_delete`：Agent 经 create 返回的 id 管理自己的计划（可写入 Memory 长期记忆）；用户与后端可直接查看/编辑 cron.jsonl。不为 Agent 发明隐式查询通道。
