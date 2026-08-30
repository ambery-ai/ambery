# Concepts

[English](concepts.md) | 中文

## Concept List

### 1. pet（宠物）— ui
Ambery 的人机界面，内置 LLM，住在自己的浮动窗口中——窗内只有颜文字。通过颜文字表达状态，通过 Component 展示信息。用户可与之打字聊天——纯自然语言，无指令。pet 理解用户意图、分析 Context、决定表达方式与何时值得打扰用户。不修改代码文件，权限边界由 Harness 的 Tool Set 限定。

#### 1a. Autonomy（自主行为层）— 子概念
pet 的自主行为引擎：面部表情切换与 pet 窗口的漂浮移动。两路控制——system prompt 定义的默认颜文字映射（不经 LLM），与 pet 经 `set_autonomy` tool call 的主动覆盖。其状态是表情与动画，以 key 表示（如 `[face: idle, motion: still]`），附加进每轮请求、持久于 Context。

> **Autonomy 是自有引擎**——状态绕过 Queue——且**不读取被监控会话**：key 的切换（Processing → Idle 等）由 AmberyBackend 依据外部输入驱动，Autonomy 只按当前 key 输出对应的表情与动作。

### 2. Surface（表达界面）— ui
表达与界面层：用户可见、可隐藏、可恢复的逻辑界面。pet 自身是锚点与交互入口，不属于 Surface；OS Window 只是 Surface 的物理投影；Menu 是瞬时弹出层，不属于 Surface。Surface 之下有三个家族：Chat Panel（对话）、Card（被渲染的信息单元）、Cards Shelf（卡的全景）。

#### 2a. Chat Panel（聊天面板）— 子概念
用户与 pet 打字聊天的界面。由 pet 右键唤出，不是 Component。包括输入框和对话历史（从 Context 读取）。用户输入写入 Queue，放行后作为 `user` role 消息入 Context；assistant 回复边生成边流式给用户——显示优化——完整回复最后写入 Context。

#### 2b. Card（卡片）— 子概念
Card 是被渲染信息的 Surface 侧身份：一块 Managed Surface（显示 / 隐藏 / 恢复语义统一），Component 的内容在其上变为用户可见。用户在卡上的交互事件（关闭、跳转、勾选等）不写 `user` role、不经 Queue，写 Harness 的 Event Buffer。Card 的持久与状态身份——它的文件、稳定 id、跨重启的生命周期——住在 Component State；两者是一体两面。

#### 2c. Cards Shelf（卡片架）— 子概念
从 pet 唤出的卡全景管理面：列出、显示、隐藏、恢复所有卡。其真相源是持久化的卡集合（见 Component State），不持有自己的持久状态；Shelf 失焦即关。

### 3. Component（组件）— ui
agent 可调用的结构化内容：预定义的前端卡片类型，用于信息展示。Component 是数据面，不是界面本身；被渲染时，其内容以 Card 的形态出现在 Surface 上。

#### 3a. Component State（组件状态）— 子概念
被渲染 Component 的持久身份。一个稳定 id 贯穿完整生命周期：首次调用创建，同 id 后续调用原位更新，显式调用关闭。卡文件是跨重启真相，同位存放 agent 可更新的 spec 与用户的 Surface 管理态——agent 能更新内容，但不能悄悄推翻用户的显示选择。作为持久工作工件，卡同时是 memory 侧的对象——被 notes 引用，但不是普通 Memory note，不经 note 工具管理。

### 4. Harness（上下文管理器）— system
pet 运行其上的外部数据与能力层——外在于 pet，不属于它的脸。它承载 Tool Set 协议（pet 能做什么的边界），内含五个域：Context（数据面）、Agent Loop（机制面）、Memory（持久理解）、Timer（调度）、Perception（感知）。pet 的 LLM 说 OpenAI Chat Completions API；数据模型与处理链路归设计文档。

#### 4a. Tool Set（工具集）— 子概念
pet 可调用的 function definitions，CLI 风格命名（完整工具表归设计文档）。pet 自身不能执行任何操作——它只能 emit `tool_calls`，由 AmberyBackend 执行后将 result 以 `tool` role message 追加回 Context。Tool Set 也是 pet 的权限边界：❌ 修改代码文件。其中感知类工具（perception tools）是 agent 主动获取外部内容的途径——Tool Set 伸向协议层的手。

#### 4b. Context（上下文）— 子概念
Harness 的数据面：完整消息数组（对标 OpenAI messages），LLM 请求的上下文源，也是完整对话的持久化存档。每条记录带时间戳。pet 通过 Context 获得被监控会话状态、任务背景等非对话信息。Context 是数据不是机制——驱动与维护它的机制住在 Agent Loop 与 Compression。

##### 4b-1. Compression（压缩）— 子概念
作用于 Context 的预算机制：usage 真值显示超出预算时，专项 LLM 调用总结历史，shaking 只保留最近的完整 turn。Digest 是它在理解侧的对手戏——理解先落袋进 Memory，之后抖掉细节才不心疼。

#### 4c. Agent Loop（智能体循环）— 子概念
Harness 的机制面：把一条放行的输入变成完整一轮的引擎——拼装请求头、调 LLM、按序执行 tool calls、广播副作用、放行下一条。词借自业界（agentic loop）。它的单元与闸门是 Queue、Event Buffer 与 Turn。

##### 4c-1. Queue（输入队列）— 子概念
所有输入的串行闸门，持久化在 Harness 中。外部输入与 user 消息入队；Event Buffer 条目不入队——它在放行时刻附带（见 Event Buffer）。用户提问以优先级高于后台输入排队；每条输入带 `source` 字段。Queue 是系统处理节奏的控制器：每条输入放行后，必须等整个 turn 处理完毕才放行下一条——OpenAI API 绝不并行调用。LLM 的 assistant 回复和 tool 结果不走 Queue，直接入 Context。

```
输入1 → Queue → Context → LLM → tool_calls? → 再调 LLM ─→ 输出 → Context ✓
输入2 → Queue（排队等待）─────────────────────────────────────→ 放行 → Context → LLM → ...
```

##### 4c-2. Event Buffer（事件缓冲）— 子概念
Component 交互事件与静默簿记的独立输入通道，与 Queue 平行：条目积压在 Buffer 自己这里，放行时刻以 `system` 消息附带——从不进 Queue，永不写 `user` role。

##### 4c-3. Turn（轮次）— 子概念
由一条输入定义的完整处理单元：从 Queue 放行一条输入开始，到这条输入引发的一切——LLM 响应、tool 调用与结果、最终回复——全部结束为止。一个 turn 属于它的那条输入；工具预算按 turn 计数，Compression 只在完整 turn 之间抖动，sleep 在同一 turn 内延续既定动作。

#### 4d. Memory（记忆）— 子概念
Harness 管理的持久理解域：notes/ 存 agent 的长期理解，整个 workspace 跨 turn、跨压缩、跨重启留存。它有两个入口：agent 的主动读写（`read_memory` / `write_memory`）与 Digest 的强制沉淀。Memory 独立于 Context、压缩摘要与来源内容存档——它是理解，不是运行记录或参考数据。后端与用户也可管理它。

#### 4e. Timer（定时器）— 子概念
Harness 的调度引擎——一个引擎三种用法：计划任务（经 cron 工具创建与删除）、短暂等待（`sleep`，等完继续既定动作）、补扫（Watch Schedule 的默认条目形态）。cron 工具与 `sleep` 是它的手柄。

#### 4f. Perception（感知）— 子概念
Harness 朝向 Ambery Protocol 的感觉器官。它接收协议层交付的东西——一次 Hook 推送或一次 Watch Schedule 扫描——对每条进来的来源更新流启动 Digest，并把消化结果渲染为循环可用的 agent 可消费内容。Tool Set 里的感知类工具是它在工具侧的手。

### 5. Ambery Protocol（Ambery 协议）— protocol
Ambery 对外的主契约：Ambery 对外部软件做的抽象——一个程序满足什么就能成为 Source、它的宿主如何推送事件、它的更新如何被消化、主动观测如何排程。这是本 package 的主要关切；任何具体软件的内部构造不在范围。传输载体在概念层刻意不定死。

#### 5a. Source（来源）— 子概念
被观测的外部实体在协议中的投影。一个 Source 以一个稳定 `source_id` 在 Context 中占有一席：它的更新以同一 ID 反复进入，而不是每次重新自我介绍——连续性由 ID 承载，注意力由更新流驱动。Source 有进程基础，也有语义席位，两者相关而不同：宿主可以死而席位等重连，席位可以关而宿主还活着。

##### 5a-1. Source Host（来源宿主）— 子概念
承载 Source 的进程或窗口——终端窗口、浏览器、书库应用、播放器。枚举、定位、激活、存活都在这层。不同软件以不同方式宿住 Source；每个宿主的具体接入形态属于那只眼的契约，不属于概念层。

###### 5a-1a. Hook（钩子）— 子概念
宿主软件主动向 Ambery 推送「发生了什么」的入口——协议的拍肩通道。每个真实的宿主都有一个：生命周期事件、书签变化、tab 事件都是宿主先知道、推出来的东西。Claude Code 的五个生命周期事件是第一个实例；每个宿主的 hook 形态（配置、脚本、事件集）属于该宿主的契约。

##### 5a-2. Context Slot（上下文席位）— 子概念
席位本身：Context 中一个稳定的 `source_id`，更新流是它的进出口。Slot 是协议对主动通知的建模单元——同一个 ID 反复出现，模型才能把一个外部事物当作连续的存在。会话身份、卡 id、页码位置都是同一模式的实例。

##### 5a-3. Digest（消化）— 子概念
Source 更新流的强制加工环节。当一个观测动作交付了新内容——Hook 推送或 Watch Schedule 扫描——Perception 启动 Digest；产物有两份：一条 Memory note（理解在压缩抖掉细节之前先落袋）与积累的 per-Source 理解（反哺 Watch Schedule 的调整）。Digest 是强制的，不交给 agent 自行裁量。

##### 5a-4. Watch Schedule（观测计划表）— 子概念
agent 为自己的主动观测撰写的计划集：看哪些 Source、看什么粒度、何时看、以什么触发条件。兜底巡逻只是默认条目形态——给 hook 冷清太久的 Source 补看；事件驱动、降频、暂停观察都是别的形态。agent 依据 Digest 的 per-Source 理解、hook 的节律与用户的提示调整计划；Harness Timer 是它的执行引擎。

### 6. AmberyBackend（后端）— system
承载运行时的后端进程：接收外部输入、执行 pet（LLM）发出的 Tool Set 调用、管理持久化。它是 Harness 的机制与 pet 的循环所骑乘的进程——承载者，不是独立的领域。

### 7. Config（配置）— system
AmberyBackend 和 pet 的持久化配置（LLM profiles、外观、语言、工具预算等）。运行时加载。pet 可通过 `edit_config` 在受限 Config 投影中按需查询与修改；system prompt 现拼规则归设计文档。

### 8. Storage（持久化存储）— system
运行时数据的持久化层。与 Config 同类型（持久化文件），用途不同：Config 存启动配置，Storage 存 session 数据与 Harness 持久化状态（读写）。布局与文件语义归设计文档。跨 AmberyBackend 生命周期保留，重启后恢复完整对话和未来计划。

### 9. Session（会话）— system
pet 自己的生命周期单元：一次 AmberyBackend 启动打开一个 session，该次运行产生的一切记录都归属它——边界标记存在 Storage。日志神圣不可改写；session 因此也是回放单元——重建一段运行就是在两个 session 标记之间切片。重启后的状态恢复跨边界读取（Memory、卡、实例清单），但新记录属于新 session；旧 session 的日志原样保留。


### 10. Platform Primitives（平台原语）— system
Ambery 自身的平台能力层——与 Tauri 强相关，不属于协议的外部抽象：Ambery 自己的窗口与环境的一切（自身 Surface 的定位、尺寸、聚焦、桌面切换），需要壳层按平台实现。这个概念正是壳层做多平台处理的原因：Windows / macOS / Web 各有实现与能力边界（Web 受限）。对外部来源的接入（定位与读取别的软件）不在这里——它属于 Ambery Protocol 侧。

---

## Examples

### Example A: 一个 Claude Code 会话收工——终端玩法

Claude Code 跑在终端 pane 里；pane 的窗口是 **Source Host**，会话本身是一个 **Source**，以稳定会话 id 占有一个 **Context Slot**。会话干完活，宿主的 **Hook**（Stop）触发，把事件推给 **AmberyBackend**。**Perception** 接收交付，对该会话的更新流启动 **Digest**：理解落袋为 Memory 沉淀，消化后的更新进入 **Queue**。**Agent Loop** 放行它——一个 **Turn**：请求从 **Context** 拼装，LLM 判断值得说，经 **Tool Set** 发出 tool call；**pet** 的 **Autonomy** 翻成跳动表情，回复渲染为 **Surface** 上的 **Card**——一个 **Component**，其稳定 id 意味着后续更新原位落地，**Component State**（含用户的显示选择）完好。用户在 **Chat Panel** 追问；交互事件进 **Event Buffer**，下次放行时附带。

### Example B: 一本书记住读到哪——读书玩法

用户在书库应用里读小说；应用是 **Source Host**，书占一个 **Context Slot**（`book:<书名>`）——每次翻页都是同一 ID 下的更新。agent 早已写好的一条 **Watch Schedule** 条目在章节间查看这本书；每次交付，**Perception** 启动 **Digest**，消化产物往 **Memory** 存一条短 note（情节位置、用户的批注想法）。几天后用户在 **Chat Panel** 问 pet：「我读到哪了？」——pet 从 **Memory** 作答，一页都没重读。

### Example C: 没有 hook 的播放器——巡逻只是计划条目

一个视频播放器是**不提供 Hook** 的 **Source Host**；那只眼只能轮询。agent 的 **Watch Schedule** 里因此有一条巡逻条目——每五分钟看一眼进度——由 **Timer** 在 tick 上执行。这条条目存在，恰恰因为宿主的 hook 沉默；会推送的宿主不需要巡逻。每次扫描是一个 **Turn**：进度更新落到这部电影的 **Context Slot** 上；到了值得说的节点，**pet** 决定打扰——**Autonomy** 跳动，**Card** 带着时间偏移弹出。

### Example D: 压缩与消化分工

一段长会话：**Context** 长到 usage 真值越过预算，**Compression** 在 **Agent Loop** 内触发——专项 LLM 调用做摘要，shaking 保留完整 **Turn**、抖掉其余，变化检测基线归零。要紧的没丢：**Digest** 一路把理解存进了 **Memory**。用户在 **Chat Panel** 问：「昨天那个 bug 我们得出什么结论了？」——pet 从 **Memory** 作答，不靠被抖掉的上下文，随后 **Queue** 放行下一条，循环继续。

### Example E: 启动——跨重启的状态

**AmberyBackend** 启动，加载 **Config**，从 **Storage** 恢复：**Memory** workspace（notes 与索引）、实例清单、以及每一张 **Card** 文件——内容与 **Component State** 同位，用户做过的显示选择（隐藏的卡、拖过的位置）跨重启留存。**Cards Shelf** 从卡文件重建全景；坏文件被跳过，一张卡坏了不拖垮其余。

### Example F: pet 的一次 session——这个词用在哪

用户启动 Ambery；**AmberyBackend** 打开 **session** #42：Storage 里一行标记边界，这一run的所有记录——Context 消息、Event Buffer 合并、turn——都归属它。会话中途应用崩溃重启；新 session #43 打开，恢复的状态（Memory workspace、卡文件、实例清单）带过来——对话得以继续，因为 Context 从 Storage 重载，但边界画下了：session #42 的日志完整保留供回放，重启之后的一切属于 #43。Session 是 pet 自己的生命周期单元——一次启动一个——它让「重建一段运行」成为在两个 session 标记之间切片的操作。

---

## Concept × Example Coverage

| Concept | A: 终端玩法 | B: 读书玩法 | C: 播放器巡逻 | D: 压缩×消化 | E: 启动 | F: pet session |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| pet | ✅ | ✅ | ✅ | ✅ | — | ✅ |
| Autonomy | ✅ | — | ✅ | — | — | — |
| Surface | ✅ | — | ✅ | — | ✅ | ✅ |
| Chat Panel | ✅ | ✅ | — | ✅ | — | — |
| Card | ✅ | — | ✅ | — | ✅ | ✅ |
| Cards Shelf | — | — | — | — | ✅ | — |
| Component | ✅ | — | ✅ | — | — | ✅ |
| Component State | ✅ | — | — | — | ✅ | — |
| Harness | ✅ | — | ✅ | ✅ | ✅ | — |
| Tool Set | ✅ | — | — | — | — | ✅ |
| Context | ✅ | — | — | ✅ | — | — |
| Compression | — | — | — | ✅ | — | — |
| Agent Loop | ✅ | — | ✅ | ✅ | — | — |
| Queue | ✅ | — | ✅ | ✅ | — | — |
| Event Buffer | ✅ | — | — | — | — | — |
| Turn | ✅ | — | ✅ | ✅ | — | — |
| Memory | — | ✅ | — | ✅ | ✅ | — |
| Timer | — | ✅ | ✅ | — | — | — |
| Perception | ✅ | ✅ | ✅ | ✅ | — | — |
| Ambery Protocol | ✅ | ✅ | ✅ | ✅ | — | ✅ |
| Source | ✅ | ✅ | ✅ | — | — | ✅ |
| Source Host | ✅ | ✅ | ✅ | — | — | ✅ |
| Hook | ✅ | — | ✅ | — | — | — |
| Context Slot | ✅ | ✅ | ✅ | — | — | — |
| Digest | ✅ | ✅ | ✅ | ✅ | — | — |
| Watch Schedule | — | ✅ | ✅ | — | — | — |
| AmberyBackend | ✅ | — | ✅ | — | ✅ | — |
| Config | — | — | — | — | ✅ | — |
| Storage | — | — | — | — | ✅ | — |
| Session | — | — | — | — | ✅ | ✅ |
| Platform Primitives | — | — | — | — | — | — |
