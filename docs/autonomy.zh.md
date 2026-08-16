# Autonomy 设计

[English](autonomy.md) | 中文

> 概念定义见 concepts.md §4。本文档定表达式模型、默认映射表与覆盖语义。

## 本文档范围

本文定义表情状态、默认推导、覆盖语义与表情池解析；Config 的通用 validation 与持久化机制见 `docs/config.md`。

## 表达式模型

pet 的外在表现 = `Expression { face: string, motion: Motion }`，`Motion = still | float | bounce | shake`。

- `face`：颜文字，渲染在 View 内。
- `motion`：View 的运动模式，CSS animation 实现，两种运行模式（Tauri/浏览器）一致。

## Config 字段

| 字段 | 生效 | agent 访问 | 行为 |
|---|---|---|---|
| `kaomoji.system` | 热 | 可见、可修改；默认不要修改 | 下一项运行操作起参与默认状态与 `set_autonomy(key)` 的两池并集解析；立即重新扫描系统池、重算 pet 尺寸与固定障碍区；当前覆盖或状态 key 不再存在时回落默认状态 |
| `kaomoji.user` | 热 | 可见、可修改 | 下一项运行操作起参与默认状态与 `set_autonomy(key)` 的两池并集解析；当前覆盖或状态 key 不再存在时回落默认状态 |
| `set_autonomy_default_ttl_ms` | 热 | 可见、可修改 | 下一次 `set_autonomy` 省略 `ttlMs` 时即取新默认值（前端现读运行时投影） |

## 两路控制

1. **默认行为（不经 LLM）**：按颜文字映射表输出当前表情与动作。规则（优先级从高到低）：

   | 条件 | 状态 key | 默认 face | 默认 motion |
   |---|---|---|---|
   | 有未决通知 | `notify` | `✧*｡٩(ˊᗜˋ*)و✧*｡` | bounce |
   | 任一实例 Processing | `processing` | `(ˇωˇ」∠)_` | float |
   | 其他（全部 Idle / 无实例） | `idle` | `(´ω`)` | still |

   映射表存于 Config 的 `kaomoji.system` 与 `kaomoji.user` 两池；按 key 在两池并集中唯一解析。`idle` / `processing` / `notify` 必须存在于并集，系统默认推导与 `set_autonomy(key)` 都按此解析。两池均可由 agent 按 `query(view=object) → update(完整 map)` 管理；系统池默认不要修改，且它是尺寸扫描来源。池间 validation 见 docs/config.md。状态 key 与 concepts §4 的示例一致：Processing → `(ˇωˇ」∠)_` + 缓慢浮动，有通知 → `✧*｡٩(ˊᗜˋ*)و✧*｡` + 跳动。

2. **pet 主动覆盖**：`set_autonomy` tool call。

## set_autonomy 语义

```
set_autonomy(key?: string, motion?: Motion, ttlMs?: number, once?: boolean)
```

- `key` 可传状态 key 名（`idle`/`processing`/`notify`/自定义 key）：在两池并集中解析为映射表本体（仅解析 face，motion 不连带）。
- 仅传参的字段被覆盖，其余保持默认输出。
- `ttlMs` 省略时默认 60000ms；TTL 到期后回落到默认输出。
- `once: true` 时从 `MotionDef.durationMs` 自动取持续时间；它与 motion 的四向 overflow 同属动画注册表，必须和 CSS `animation-duration` 同步。动画 CSS 仍循环，TTL 到期后回落默认状态，由此收束为一次性动作。
- `once: true` 与显式 `ttlMs` 同时传入直接拒绝，避免两套持续时间语义冲突。
- 全部参数省略（或 `ttlMs: 0`）→ 立即清除覆盖，回落默认。
- 覆盖期间实例状态变化不中断覆盖（pet 的表达优先），TTL 到期后回落默认。

## 与 LLM 侧的关系

Autonomy 是自有引擎，独立于 AmberyBackend。状态格式 `[face: key, motion: key]`，约 6-7 token，每轮附加到请求末端。持久于 Context（每轮一条记录），不落 Queue。

注意：LLM 通过 Context 的 diff 事件感知 Code CLI 实例状态变化（见 docs/harness.md），两者数据来源不同，互不依赖，不要混淆。
