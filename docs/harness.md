# Harness 设计

> 概念定义见 concepts.md §10 及子概念。本文档定数据模型、注入规则、触发模型与 JSONL 存储格式。

## 数据模型

```rust
// Queue 输入条目（concepts §10c，输入串行化关口——只装输入）
struct QueueInput {
  role: 'system' | 'user',     // hook 内容 = system 输入；用户消息 = user 输入
  content: String,
  ts: i64,
}

// Context 消息（OpenAI Chat Completions 对齐，concepts §10b——完整消息数组）
struct ContextMessage {
  role: 'system' | 'user' | 'assistant' | 'tool',
  content: String | null,        // assistant 发起 tool_calls 时 content 可为 null
  tool_calls: ToolCall[] | null, // 仅 assistant
  tool_call_id: string | null,   // 仅 tool
  ts: i64,                       // epoch ms
}

// Filtered 内容（concepts §8/§11，Filter 后归一全文，agent 实际读到的终端内容）
// 不持久化——从 terminal-content.jsonl 原文 digest 现算（docs/storage.md §filtered_content 退役）
struct FilteredContent {
  instance: string,              // Code CLI 实例名
  filtered_content: string,      // Filter 后的 Terminal Content 全文
  source: 'hook' | 'timer' | 'fetch_terminal',
  ts: i64,
}

// Code CLI 实例清单（concepts §9/§13）
struct AgentEntry {
  hash: string,                  // sid8(session_id)；mock/扫描回退见 docs/hook.md §marker 定位
  name: string, project: string, // display 名 = <project>·<sid8>（即 tab 定位 marker）
  kind: string | null,           // CLI 种类（Filter 按它选择，docs/filter.md）
  tab: TabRef | null,            // tab 定位快照（session_end 的 closed 快照 tab 为 null）
  status: 'idle' | 'processing' | 'unknown' | 'closed',
  first_seen: i64, last_seen: i64,
}
```

## Queue 规则（concepts §10c）

1. **Queue 只装输入**：hook 内容（system 输入）、user 消息（user 输入）。assistant / tool 输出**不走 Queue**，直接入 Context。
2. **每条输入带来源字段**：来源 = 触发这次输入的语义原因（`user_chat` / `hook_stop_hint` / `hook_stop_content` / `hook_stop_report` / `hook_user_prompt` / `hook_notification` / `mock_hook` / `timer_scan` / `cron_tick`，完整集合与入队点见 `docs/concrete-insight.md §Queue 中的 System 消息来源`）。来源是驱动 effort 档位、优先级等按来源定行为机制的一等公民；`release_one` 把它传进 `run_trigger`，工具循环内后续调用沿用。
3. **串行放行 + 双队列**：每条输入放行后，必须等整轮处理完毕（输入写 Context → LLM → tool 执行 → 输出写 Context）才放行下一条——不可并行调用 LLM。输入到来时若正在处理中，在 Queue 中排队等待。Queue 分两队：`high_q` 装 `user_chat`（用户直接问 pet），`normal_q` 装其余全部；放行时 `high_q` 非空先放 `high_q`（FIFO），空则放 `normal_q`（FIFO）——用户直接问 pet 时优先处理，两队列内部各自保持到达顺序。
4. system prompt **不是** Queue 输入也不是 Context 消息——它是每次 LLM 调用时现拼的请求头（base_prompt + AGENTS.md + 系统表情池；用户表情池按需查询），内容稳定、天然 cache 友好，不落盘（head 快照见 docs/storage.md）。
5. Hook 触发 → AmberyBackend 向 Queue 注入 `system` 输入（如「config-service 完成（4958 字）。评估是否通知。」）。
6. **diff 事件化**：实例注册/状态翻转的簿记事件不走 Queue——走 Event Buffer 静默附带（§10e）；LLM 从 Context 中的事件流重建全景，不按轮注入快照。

## Event Buffer 规则（concepts §10e）

- 与 Queue 平行的独立输入通道，存取 Component 交互与静默簿记。
- 每条记录携带两部分载荷：**自然语言**（必填，操作过程描述）和 **结构化状态快照**（可选，仅 todobox 类交互时附带）。
- 同 card 在单次 flush 内的结构化快照**去重合并**为一份最终状态（连续勾选三条 todo → 只发一份 items 全量）。
- Queue 放行某条输入时：Buffer 全部条目（自然语言 + 结构化快照）与该输入**合并为一条** `system` message 入 Context，然后清空（附带语义，不产生独立消息）。
- 永不写 `user` role；原始条目不持久化（合并后的消息落 Context 日志）。

## Context 规则（concepts §10b）

- Context = 完整消息数组（OpenAI messages 对齐）：Queue 放行写输入，LLM 回复写 assistant，tool 执行写 tool——LLM 请求的上下文源，也是完整对话的持久化存档。
- 终端内容：Hook 触发 → AmberyBackend 读 Terminal Content → **原文先存 terminal-content.jsonl** → Filter → 归一结果更新内存变化检测基准；放行后注入 Context 的是评估提示（「{name} 完成，Context 已更新（N 字）。评估是否通知。」形态）——归一全文本身不进 Queue/Context。
- 归一全文**不持久化**：变化检测的 prev（每实例上次归一全文）存内存（重启丢）；「那个 bug 具体怎么回事」类追问与 `fetch_terminal` 回退从 terminal-content.jsonl 原文 digest 现算。
- Autonomy 状态记录（type=autonomy）每轮一条也写 context.jsonl；装配请求时取最新一条挂请求末端（concepts §4 / docs/storage.md）。

## Compression（concepts §10d，auto-compact）

- 触发（usage 真值系）：**最近一次 `usage.prompt_tokens` + 其后新增消息的 est 增量 >
  `effective_compression_limit()`**（active profile 的 `context_window − reserve`；
  reserve 缺省 = 全局 `compression_reserve_default` 10K）。**无 context_window = 不压缩**（显式不猜）。
  usage 行是权威真值（docs/storage.md §usage）；est（chars/4，标注失真）仅用于无真值时
  （首轮/重启无 usage 行）与增量估算，不做主源。本地 BPE 分词器不引入
  （opencode / Claude Code 均以 API 真值为准）。
- 摘要：**专项 LLM 调用**生成（配置的模型把历史压缩为一条 `system` 摘要；DebugAgent 模式回退确定性 stub，保证测试确定性）。
- shaking：按完整 turn 边界保留原始消息，不能按单条 message 任意截尾。一个 turn 就是一条 Queue 输入从放行开始，到其触发的全部 LLM response、tool calls、tool results 与最终回复结束的完整集合。

### Config 字段

| 字段 | 默认 | 生效 | 语义 |
|---|---:|---|---|
| `context_compression_keep_recent_messages` | 24 | 冷 | 已完成历史 turns 的原始 message 保留目标；从最近已完成的 turns 向前保留至少该数，只能在完整 turn 之间截断；当前正在处理的 turn 始终完整保留，即使超过该数。assistant 的 tool_calls message 与每条 tool result 各算一条；一次上限 10 calls 的完整工具交互为 11 条，24 可保留约两批此类交互及最终文字回复，避免完整 query 真值过早被压缩摇掉。 |
- **归零**：diff 基准清空，所有已有实例视为刚发现，一次 diff 进一条 system 消息——压缩不丢实例认知。

## Memory（concepts §10f）

Memory 是 Harness 管理、Agent 主动维护的**持久化理解 buffer**：它替代 Agent 依赖文件系统记录理解，不是 Context、压缩摘要或终端内容存档。它在跨 turn、压缩与重启后保留；后端、用户与 Agent 都可管理，其中 Agent 通过 `read_memory` / `write_memory` 调整。

- Memory 是整个 Ambery 共享的一套理解，不按被监工 project 隔离；它可记录跨项目计划、工作关系与用户协作偏好。
- 持久化形态为一个 Memory Workspace（notes/ 长期理解 + cards/ 持久工作产物）：目录结构、索引、写入规则与只读契约见 docs/memory.md。

两个 tool 的参数 schema、校验与返回结构、index/AGENTS.md 生成契约见 **docs/memory.md**（已定稿）。

#### ⟡ 一致性剖析

Memory Workspace、Cron 与 Card 都是 Harness 管理的跨重启概念：各自从持久载体恢复为运行期投影，受控地被用户、后端或 Agent 的相应入口管理，并可被 observe。它们不属于 Context、Queue 或 Event Buffer，也不能把真相下放给 View 或 LLM 的局部状态。三者的持久化形式可因语义不同而不同——notes / Card 用文件，Cron 用 append-only 计划日志——一致的是所有权、恢复和消费边界，而不是强行使用同一种文件格式。

## Cron（concepts §10g）

Cron 是 Harness 管理的**持久化计划与延时调度**：它记录未来工作，例如每晚发出日报提示；也能支持短暂 sleep 后继续既定行为，例如等待数秒后执行 `set_autonomy`。它跨重启保留，后端、用户与 Agent 都可管理。

Agent 经 `cron_create` / `cron_delete` 调整 Cron，并经 `sleep` 请求短暂等待；三者在底层使用同一套 Harness 调度实现（`CronScheduler`）。任务表示、cron.jsonl 格式、到点行为与三个 tool 的参数/校验/返回见 **docs/cron.md**。

## Storage（concepts §13，spec：JSONL）

布局、各文件语义与记录格式见 **docs/storage.md**。要点：

- 两个域：Config（`config.json` + 身份提示词 `AGENTS.md`）在根，Storage（`queue` / `terminal-content` / `context` / `work-agents` / Memory / Cron）在 `storage/`。Memory 根内的只读 `AGENTS.md` 与 Config 域身份提示词不是同一文件。
- **Context 是内存视图**：全保真日志在 context.jsonl（message / autonomy / head / filtered_content / 压缩边界统一信封）——压缩是标记不是删除，OpenAI 上下文几乎完全可复原。
- **Queue 输入留痕**：append-only queue.jsonl（入队记录；放行后原文已在 Context 的 message 行，queue.jsonl 是排队轨迹非对话本体）。
- Event Buffer 原始条目不持久化：暂存区语义，崩溃丢失可接受（合并后的 system 消息落 Context 日志）。

## 触发模型（一条 Queue 输入的完整 turn）

```
Queue 放行一条输入（附带 Event Buffer 合并为一条）
  → Context 写输入
  → 现拼 system prompt 请求头（base_prompt + AGENTS.md + 系统表情池，不落 Context）
  → Compression 检查（Context 超阈值 → 专项摘要 + shaking + 归零重 diff）
  → 附加 Autonomy 状态到请求末端
  → call LLM（请求 = 请求头 + Context 全部消息 + Autonomy 状态）
  → loop: assistant tool_calls → AmberyBackend 执行 → tool result 追加 Context → 再做 Compression 检查 → 再 call
  → assistant content 追加 Context
  → Queue 放行下一条
```

## 前后端协议

Tauri 模式：前端与 core 通信走 **Tauri 原生 IPC**（`#[tauri::command]` + `invoke()` + `app_handle.emit()` + `listen()`）。
仅外部 hook 脚本保留 HTTP `POST /hook`（进程外跨进程调，Tauri command 不可达）。

浏览器调试模式：core 以 `ambery-case serve` 完整 router 宿主运行（docs/core-server.md），前端通过 thin HTTP+WS loopback 直连。
