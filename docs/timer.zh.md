# Timer 设计

[English](timer.md) | 中文

> 概念定义见 concepts.md §1a。本文档定调度机制、错峰算法与扫描动作的应用点。

## 定位

Hook 是主通道，Timer 是观测循环兼兜底。实例 Hook 长时间未触发时补扫一次——读 Terminal Content → Filter → 变化检测 → 有实质变化才注入 Queue 评估（Example C：「config-service 上次 Hook 未触发，但 Timer 兜底扫描已更新其 Context」）。扫描同时把读证据交给实例信念状态（concepts §9a）：`Content` 刷新"活着"信念，确证 `Gone` 是死亡证据，`Error` 不是观测——**Timer 绝不用时间推断生死**；证据缺席只是把信念移向 `unknown`。

**开关**：`timer.interval_ms ≤ 0 = 禁用`（Config，面板/CLI 可配）。真实 hook 接入初期建议禁用——只留 hook 驱动，避免全量实例周期扫描带来的 LLM 触发频率；mock 调试期用正数。

## Config 字段

全部为冷字段：重启应用后生效；agent 可见、可修改。TimerWheel / 主循环在启动时构建，不在运行中重建。

| 字段 | 默认 | 生效 | agent 访问 | 语义 |
|---|---:|---|---|---|
| `timer.interval_ms` | 300000（5 分钟） | 冷 | 可见、可修改 | 每实例兜底扫描间隔；≤ 0 = 禁用 |
| `timer.stagger_ms` | 30000（30 秒） | 冷 | 可见、可修改 | 错峰窗口：多实例到期时间在窗口内打散 |
| `timer.tick_ms` | 60000（60 秒） | 冷 | 可见、可修改 | 主循环粒度：每 tick 醒一次取到期实例（interval 小于它也最多每 tick 一扫）；合法下界 ≥ 100 |
| `timer.batch` | 2 | 冷 | 可见、可修改 | 每 tick 最多扫描实例数（限流）；合法下界 ≥ 1 |

## 调度（TimerWheel）

```rust
struct TimerWheel {
    interval_ms: i64,               // 兜底扫描间隔（Config，默认 5 分钟）
    stagger_ms: i64,                // 错峰窗口（Config，默认 30s）
    next_due: HashMap<String, i64>, // instance → 下次到期时间
}
```

- **错峰**：`due(instance) = now + interval + hash(instance) % stagger`。确定性哈希（实例名）保证同一实例每次偏移相同，不同实例天然错开，避免同时扫描（concepts §1a「错峰分布」）。
- **Hook 到达 → reset**：`handle_hook` 里对触发实例 `reset(now)`（重新计 interval + 错峰偏移）——Hook 是主通道，近期有 Hook 的实例不该被补扫。
- **到期提取**：`due(now, batch)` 返回到期实例（一次最多 batch 个），取走即重排 `now + interval + stagger`；剩余保持到期，下一 tick 再取——批量上限也是错峰。

## 扫描动作（Terminal Adapter 读取）

扫描读通道 = Terminal Adapter（docs/terminal-adapter.md）：`locate(instance) → read(tab)` 读到当前 Terminal Content；读不到返回 None。各终端实现（WtAdapter / MapAdapter / Composite 分发）与装配门控（`terminal.adapter_*`）见该文档。

- case-runner 剧情面：`terminal` step 写 MapAdapter 的共享 map，**模拟「终端当前显示什么」**。与 mock hook 对称：hook 模拟推通道，terminal 剧情模拟读通道。
- `fetch_terminal` tool 读同一 adapter（读不到回退 Context 最新记录）——读通道只有一处。

## 扫描处理流程（变化检测的真实应用点）

```
tick（server 后台任务，默认 60s（config `timer.tick_ms`；case-runner 可经 AMBERY_TIMER_TICK_MS 覆盖））
  → due(now, batch)
  → adapter 按已定位 tab 读（三态：Content=证据存活 / Gone=确证不存在→closed 证据 / Error=跳过一次，信念不动，docs/storage.md）
  → 原文存 terminal-content.jsonl → Filter.digest 归一
  → 与内存 prev 基准 detect_change（归一全文不持久化，prev 存内存重启丢）
  → Substantive：注入「{instance} 兜底扫描发现变化，Context 已更新（{len} 字）。评估是否通知。」进 Queue（source=timer_scan）→ run_trigger（归一全文本身不进 Context）
  → Minor / Unchanged：原文存档 + prev 更新，不打扰（concepts §9b 沉默精神一致）
```

注入消息与 stop hook 同构（`…，Context 已更新（N 字）。评估是否通知。`）——通知/沉默决策路径一致。
