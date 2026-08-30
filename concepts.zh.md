# Concepts

[English](concepts.md) | 中文

## Concept List

### 1. AmberyBackend（后端）— system
后端进程。管理被监控会话的生命周期：接收外部输入、读取来源内容、存入 Context。执行 pet（LLM）发出的 tool call、管理持久化、运行触发循环。启动时加载 Config，运行时监听 HTTP Hook 端口。

### 2. pet（宠物）— ui
Ambery 的人机界面，内置 LLM，存在于自身的浮动窗口中（pill 形状，always-on-top，可拖拽；窗内仅显示颜文字，无其他 UI 元素，缩放由 Config 的 view_scale 控制）。通过颜文字表达状态，通过 Component 展示信息。用户可与之打字聊天——纯自然语言，无指令。pet 理解用户意图、分析 Context、决定表达方式。不修改代码文件，权限边界由 Harness 的 Tool Set 限定。

### 3. Surface（界面层）— ui
用户可见、可隐藏、可恢复的逻辑界面：Chat 与 Card 为 Managed Surface（显示 / 隐藏 / 恢复语义统一）；pet 自身是锚点与交互入口，不属于 Surface；OS Window 只是 Surface 的物理投影；Cards Shelf 与 Menu 是瞬时弹出层，不属于 Surface。

#### 3a. Chat Panel（聊天面板）— 子概念
用户与 pet 打字聊天的界面。由 pet 右键唤出，不是 Component。包括输入框和对话历史（从 Context 读取）。用户输入写入 Queue 排队，放行后作为 `user` role 消息入 Context。assistant 回复的流式增量显示：SSE 片段经 StreamingChannel 直推前端逐片渲染，纯显示优化——不经 Queue/Context，完整回复最后才写 Context。

### 4. Autonomy（自主行为层）— system
pet 的自主行为引擎。控制面部表情切换、pet 窗口的漂浮移动。两路控制：默认自动行为由 system prompt 中的颜文字映射表定义（如 Processing → `(ˇωˇ」∠)_` + 缓慢浮动，有通知 → `✧*｡٩(ˊᗜˋ*)و✧*｡` + 跳动），不依赖 LLM；pet 也可通过 `set_autonomy` tool call 主动覆盖（如突然跳一下、换个表情卖萌）。

Autonomy 的顶层状态是表情与动画。状态用 key，不加颜文字本体——格式 `[face: idle, motion: still]`，4 个单词 + 5 个符号，约 6-7 token。与 Queue 无关——无论状态变化与否，每轮直接附加到请求上下文末端，持久于 Context（每轮一条记录），不落 Queue。量级极小，不存在 cache 担忧。

> **Autonomy 是自有引擎，独立于 AmberyBackend。** 状态不经过 Queue，直接进入上下文。
>
> **Autonomy 不读取被监控会话。** 状态 key 的切换（Processing → Idle 等）由 AmberyBackend 根据外部输入（Hook / 扫描等输入通路）驱动，Autonomy 只按当前 key 输出对应的表情与动作。

### 5. Component（组件）— ui
预定义的前端可视化卡片类型，用于信息展示。由 pet 通过 Tool Set 调用，以 pet 为中心向合适方位偏移弹出。Component 协议定义两层：① system prompt 描述类型/参数/使用场景/文本量级 ② Tool Set 中 CLI 风格的 function call，spec 参数即为上下文内容。用户交互事件（关闭、跳转、勾选等）不写 `user` role、不经 Queue，写 Harness 的 Event Buffer——Queue 放行时附带入 Context。

### 10. Harness（上下文管理器）— system
pet 运行所需的数据与能力层。Queue 是全局串行化入口——AmberyBackend 将外部输入和 user 消息写入 Queue。Queue 串行放行→Context 写输入→LLM→Context 写输出→放行下一条。Event Buffer 是与 Queue 平行的独立输入通道，收 Component 交互事件。Context 更新和 Compression 由 Harness 内部负责，不经后端直接操作。pet 的 LLM 基于 OpenAI Chat Completions API 模型：

- **messages**：消息数组，每条含 `role`（`system` / `user` / `assistant` / `tool`）和 `content`
- **tools**：可用的 function definitions 列表，LLM 通过 emit `tool_calls` 调用
- **tool result**：AmberyBackend 执行 tool 后，以 `role: "tool"` message 追加回 messages

#### 10a. Tool Set（工具集）— 子概念
pet 可调用的 function definitions（CLI 风格命名）：`call_component` / `fetch_terminal` / `set_autonomy` / `edit_config` / `read_memory` / `write_memory` / `cron_create` / `cron_delete` / `sleep`。pet 自身不能执行任何操作——它只能 emit `tool_calls`，由 AmberyBackend 执行后将 result 以 `tool` role message 追加回 Context。Tool Set 也是 pet 的权限边界：❌ 修改代码文件。

#### 10b. Context（上下文）— 子概念
pet 的外部信息注入通道，持久化在 Harness 中。每条记录带时间戳。Context 是完整消息数组（对标 OpenAI messages），是 LLM 请求的上下文源，也是完整对话的持久化存档。数据模型与处理链路归设计文档。pet 通过 Context 获得被监控会话状态、任务背景等非对话信息。



#### 10c. Queue（输入串行化关口）— 子概念
所有输入的串行排队器，持久化在 Harness 中。外部输入、user 消息、Event Buffer 合并事件全部先入 Queue 排队。**Queue 是系统处理节奏的控制器**——每条输入放行后，必须等整轮处理完毕（输入写 Context → LLM 调用 → tool 执行 → assistant 输出写 Context）才放行下一条，不可并行调用 OpenAI API。LLM 的 assistant 回复和 tool 结果不走 Queue，直接入 Context。

```
输入1 → Queue → Context → LLM → tool_calls? → 再调 LLM ─→ 输出 → Context ✓
输入2 → Queue（排队等待）─────────────────────────────────────→ 放行 → Context → LLM → ...
```

#### 10d. Compression（上下文压缩）— 子概念
Context 超 token 阈值时触发（auto-compact）：以 usage 真值计量、按模型 `context_window` 标定阈值、专项 LLM 调用生成摘要 + shaking 保留完整 turn + 归零重 diff。确保每轮 LLM 调用的上下文始终在预算内。

#### 10e. Event Buffer（交互事件缓冲区）— 子概念
Component 交互事件的独立输入通道，与 Queue 平行：交互以短自然语言（可附结构化快照）写入，LLM 触发时合并为一条 `system` message 附带入 Context 后清空，永不写 `user` role。

#### 10f. Memory（持久化理解 buffer）— 子概念
Agent 用来持续记录和恢复自身理解的持久化 buffer，替代 Agent 直接操作文件系统写笔记。Memory 与 Context、压缩摘要、来源内容存档分别独立：前者是 Agent 主动维护的长期理解，后者是运行记录或参考数据。它由 Harness 持久化管理，后端与用户可管理，Agent 通过读写 Memory tools 调整。

#### 10g. Cron（持久化计划与延时调度）— 子概念
Harness 的持久化计划与延时调度能力。它记录未来工作，例如每天夜间向 Agent 发出日报提示；也支持短暂等待后继续既定动作，例如先等待数秒再调用 `set_autonomy`。后端、用户和 Agent 都可查看或调整 Cron；Agent 通过两个 Cron tools 管理，另有 `sleep` tool 表达短暂等待。Cron 与 sleep 共用同一个 Harness 调度实现。

```
Component 交互 ─→ Event Buffer（积压）
                     ↓ Queue 放行时附带入
                   Context（合并为 system 消息）
                     ↓
                   LLM ─→ Context
```

| | Queue | Event Buffer |
|---|---|---|
| 输入源 | 外部输入、user 消息 | Component 交互事件 |
| 处理方式 | 串行排队，逐个处理 | 积压，LLM 触发时合并为一条 |
| 持久化 | append-only queue.jsonl | 不持久化（合并后清空） |
| 输出方向 | Context 存档 + LLM 请求体 | 合并后注入 LLM 请求上下文 |
| 角色定位 | 输入串行化关口 | 低优先级事件的合并暂存区 |

### 12. Config（配置）— system
AmberyBackend 和 pet 的持久化配置（LLM profiles、Compression、表情池、pet 外观、主题、语言、pet 名称与工具预算等）。运行时加载。pet 可通过 `edit_config` 在受限 Config 投影中按需查询与修改；system prompt 现拼规则归设计文档。

### 13. Storage（持久化存储）— system
运行时数据的持久化层。与 Config 同类型（持久化文件），用途不同：Config 存启动配置，Storage 存 session 数据与 Harness 持久化状态（读写）。布局与文件语义归设计文档。跨 AmberyBackend 生命周期保留，重启后恢复完整对话和未来计划。

### 15. Platform Primitives（平台原语）— system
平台特定能力的抽象组：**代 pet 在用户环境中动作的层**——虚拟桌面切换、窗口聚焦/激活等 OS 层动作。壳层为多平台做处理正是为了这个概念：Windows / macOS / Web 各有实现（Windows 走 COM）。打断用户的动作（如切到别的桌面）经显式同意门控。

