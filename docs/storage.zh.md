# Storage 设计

[English](storage.md) | 中文

> 概念定义见 concepts.md §12/§13。本文档定目录布局、各文件语义、记录格式与生命周期。

## 布局：两个域

```
%USERPROFILE%\.config\ambery\   （AMBERY_CONFIG_DIR 可覆盖，core/paths.rs 解析）
  config.json              # Config 域：启动配置（concepts §12）
  AGENTS.md                # Config 域：pet 身份提示词——配置性质，非 session 数据
  storage/                 # Storage 域：世界状态 + 上下文日志（AMBERY_STORAGE_DIR 可覆盖，concepts §13）
    queue.jsonl            # Queue 输入排队记录（concepts §10c）
    terminal-content.jsonl # Terminal Content 原文存档（Filter 前）
    context.jsonl          # 统一全保真日志：对话 + Autonomy + 请求头快照 + 归一全文 + 压缩边界
    work-agents.jsonl      # 实例生命周期永久事件日志
    effect.jsonl           # 前后端统一动作流（Effect：后端副作用 + 非只读 Tauri 运行时动作）
    memory/                # Memory Workspace：长期理解 + 持久工作产物
      AGENTS.md            # 只读：工作空间导航信息
      index.md             # 只读：自动汇总 notes/ 的名称与 description
      notes/               # Agent 的长期理解（当前不再细分目录）
        *.md               # 短小、碎片化的普通 note
      cards/               # 持久 Component / 工作产物
        <id>.card.json     # 文件即 Card：完整 JSON，内容 + Surface 意图 + 空间布局
    cron.jsonl             # Harness 的持久化计划与延时调度
```

**哲学**（与 Claude Code session 格式相同）：**日志神圣，视图易失**。一切进文件、
append-only、永不改写；请求上下文只是日志的投影。目标：**OpenAI API 上下文几乎完全可复原**——
对话（含 tool_calls/reasoning_content）、事件、autonomy、实例、终端内容全部留痕，
压缩是标记不是删除；请求头装配结果也留快照——比 CC 更完整（CC 不版本化 CLAUDE.md）。

## config.json（Config 域）

concepts §12。启动配置：timer 参数、compression 阈值与保留目标、系统/用户表情池、
LLM profiles + active 选择器、view_scale、set_autonomy_default_ttl_ms、stop_hook_mode、
theme/themes、ui_language/harness_language、name、工具调用预算。
（hook 端口不是 Config 字段：默认 127.0.0.1:47600，`AMBERY_PORT` 显式覆盖——换端口须同步 hook 配置；docs/core-server.md §端口语义。）

- 写：bootstrap 写默认 / 统一 Config 修改入口写回。读：启动加载 + 运行中外部文件自动载入。
- key 本体只在环境（provider 的 `api_key_env`），不入文件。**应用级 env 层**：`env`（0600，`KEY=value` 行）是应用级环境变量层，*覆盖*系统环境——解析顺序为 env 文件 → 进程环境（先命中者胜）。env 文件是应用内 key 存储位（引导 modal 写入处）；它不是 `config.json` 的一部分，永不包含 Config 域数据。见 docs/llm-setup.md §key 存储模型。

## AGENTS.md（Config 域）

pet 身份提示词，与 base_prompt 拼接进**每轮现拼的请求头**（不落 Context）。

- bootstrap：不存在则写内置默认（仅一次）；用户可手编。
- **每次 LLM 请求装配时现读**（热生效：改完下一次请求就用）；运行中读不到时保持已加载的 live 内容，并在反射 Config UI 显示加载错误；不以默认内容覆盖。
- 放在 Config 域的理由：稳定、可编辑、跨会话不变——与 config.json 同类，一个存参数，一个存身份。

## terminal-content.jsonl（原文存档，Filter 前）

Terminal Content **原文**（ANSI/spinner 全在，concepts §8 瞬时全量文字）。每次读取写一条：

```json
{"instance":"demo-webapp","raw":"…原文…","source":"hook","ts":1784952913010}
```

- `source`: `hook | timer | fetch_terminal`。
- 写入时机：每次读取**先写原文、再过滤**——崩在中间也至少有原文。
- 角色：ground truth 存档。Filter 规则迭代时拿原文回放验证；debug「Filter 滤掉了什么」。**平时不读，启动不 replay**。

## context.jsonl（统一全保真日志）

**一个文件装下复原 OpenAI 上下文所需的一切**（CC 单文件 + type 判别同风格），append-only、永不改写。
每行统一信封 `{type, ts, ...}`：

| type | 载荷 | 进请求？ |
|---|---|---|
| `message` | ContextMessage 全保真：{role, content, tool_calls, tool_call_id, reasoning_content} | ✓ Context 主体 |
| `autonomy` | {content: `[face: key, motion: key]`}，每轮一条（无论变化与否） | ✓ 最新一条挂请求末端 |
| `head` | {content: 装配完成的请求头}，**变化才写**（diff） | ✓ 最新一条作请求头 |
| `usage` | {prompt_tokens, completion_tokens}——每次 LLM 调用的真值（cache 分项恒 0 不存） | ✓ 最新一条为 compression 真值基准 |
| `compact_boundary` | {summary, pre_tokens, post_tokens, duration_ms} | 视图投影标记 |
| `session` | {sessionId}，每次启动一条 | 会话分界 |

> **`filtered_content` 行型**：归一全文**不持久化**——它可由
> terminal-content.jsonl 原文 digest 重算，持久化是冗余。旧文件中的 `content` 行 replay 时忽略。
> 变化检测的 prev（每实例上次归一全文）存**内存**（scan 后更新，重启丢：重启后首轮 scan
> 必报变化一次，接受的代价）；`fetch_terminal` 回退/追问从原文现算；observe 的
> `filtered_content` 项同样现算（docs/case-runner.md §可观测体系）。

- **`message`**：Context 每追加一条（Queue 放行的输入 + LLM 的 assistant/tool 输出）同步落一行——对话全保真，
  assistant 的 tool_calls 与 reasoning_content 一字不落（thinking 模型回放刚需；纯文本回复的思维链同样全保真，
  记录≠回放：回放仅 tool_calls 消息带 reasoning，见 docs/agent-loop.md）。
- **`autonomy`**：concepts §4 定的每轮一条；装配时取最新。
- **`head`**：base_prompt + AGENTS.md + 系统表情池的装配结果**变化才写**——请求头历史也可复原（设计决定）；用户表情池不自动注入请求头，按需经 `edit_config` 查询。
- **`usage`**：token 计量的**唯一权威源**。每次 LLM 调用（含 tool 循环每轮、Compression 摘要调用）
  写一条；读取语义是**覆盖**——最新一条的 `prompt_tokens` 即「上次请求体全量（head+messages+autonomy）
  的精确 token 数」（opencode 同构：step 级留痕、最新值代表当前 context 占用）。cache 细分实测
  恒 0，不存。
- **`compact_boundary`**：压缩是标记不是删除——内存视图 shaking，文件全保留，压缩可审计
  （summary 与原文都在）。
- **`session`**：每次启动写一条。复原某次运行 = 两条 session 标记间切片。

### 视图重建（OpenAI 上下文投影规则）

1. 取目标 session 区间（默认最新一个）。
2. 区间内最后一条 `compact_boundary`：其 summary 为首条 system 消息 + 其后 `message` 行为 Context；
   无 boundary 则取全部 `message` 行。
3. 区间终点时刻最新的 `head` 作请求头；`autonomy` 在请求装配时现算（现读 agents/config，不建 replay 索引）——投影规则无需为它留索引（docs/window-follow.md 一致性剖析同类：effect 只审计动作不反推当前态）。

= 该时刻的完整请求；任意历史时刻同理（用当时的 head / autonomy / boundary 切片）。

## work-agents.jsonl（实例生命周期永久事件日志）

**永久记录**：不折叠、不轮转、不清理。每个 agent 每次生命周期一个 hash
（名字会重复——昨天关掉的 tab 和今天新开的同名 tab 是两条命）：

```json
{"hash":"a1b2c3d4","name":"demo-webapp","project":"nap","status":"processing","ts":1784952913010}
{"hash":"a1b2c3d4","name":"demo-webapp","project":"nap","status":"idle","ts":1784953125000}
{"hash":"e5f6a7b8","name":"demo-webapp","project":"nap","status":"processing","ts":1785050000000}
```

- 每行 = 一次状态变更后的**完整快照**（自包含，查日志不用找注册行）。
- 字段（docs/hook.md）：`{hash, name, project, kind, status, tab, first_seen, last_seen}`
  - `hash` = **sid8(session_id)**——session_id 前 8 位（同名不同命：同项目重开 = 新生命周期；docs/hook.md §marker 定位同源）；mock hook 无 session_id 时回退 `short_hash(name + project + first_seen)`。
  - `name` = `<project>·<sid8>`，display 名**同时就是 tab 定位 marker**（一名两用）。
  - `kind` = CLI 种类（`"claude"`，per-instance filter 策略输入，docs/filter.md）。
  - `tab` = `{hwnd, index}` 定位结果。**与 status 同等待遇：只是快照的一个字段**——定位成功后的事件快照带上它，「重找」= 再 append 一条新快照，当前值永远由投影（每 hash 最新行）得出；无原地更新。session_end 的 closed 快照 tab 为 null。
  - `first_seen` / `last_seen` = 后端初见/最近事件时刻（backend 只知自己什么时候见的）。
- Status 信念状态机（concepts §9a）：`idle | processing | unknown | closed`。
  信念只随具体证据移动：hook 事件、终端读、进程检查。`closed` 为终态——退出活跃集——在确证关闭证据下（SessionEnd Hook；或"确证不存在"：reader NotFound / 进程检查查无进程），
  或因**长期 unknown 退休**（失去联系；确证死亡与失去联系不分开维护）；Timer 扫描是交付证据的观测循环，不做时间推断判死。
  瞬时读失败永不判死。永久日志仍须退休 unknown 实例，否则全景无限累积尸体。
- **注册表（当前状态）= 日志投影**：replay 按 hash 折叠取最新。
- 启动归零重同步的全景 = 投影中 `status ≠ closed` 的集合；unknown 条目按「未确认」呈现，不当作活体。

## Context：内存视图，日志在 context.jsonl（concepts §10b）

Context（完整消息数组）是 context.jsonl 的**内存投影**（视图重建规则见上节）。运行中 append 双写：
内存 Context + context.jsonl 的 `message` 行。

- 重启：写 `session` 标记 → 空 Context + **启动归零重同步**（存活实例全景一条 system 消息，
  同样落 `message` 行）——与 Compression 归零重 diff 同一机制（docs/harness.md）。
- 默认不 resume；历史完整在案，`--resume` 即投影规则的应用（设计决定）。
- Event Buffer 随放行输入附带合并后的 system 消息落 `message` 行（原始条目不存，暂存区语义，崩溃丢失可接受）。

## Queue：输入排队器，日志 queue.jsonl（concepts §10c）

Queue 装待处理输入（hook 内容、user 消息），串行放行（放行后原文入 Context 的 `message` 行）。
append-only queue.jsonl 逐条留痕入队输入——它是**排队轨迹**，非对话本体。

- 崩溃丢失未放行输入可接受：hook 是瞬时信号（Terminal Content 还在屏幕上，可重读），
  user 消息由面板重发。
- 旧版 queue.jsonl（消息日志时代语义）与改名前的 agents.jsonl 不迁移（设计决定）。

## Memory Workspace（Harness 持久工作空间）

概念与读写边界见 concepts §10f / docs/harness.md。`storage/memory/` 是唯一的 Memory Workspace 根，不要求扁平：

- `notes/`：Agent 的长期理解；普通 note 是受长度上限约束的 `.md` 文件，当前不再细分目录。`index.md` 自动按表汇总 notes 的名称与每次写入必带的 description。
- `cards/`：持久 Component / 工作产物；一个 `<id>.card.json` 文件就是一个 Card。文件为完整 JSON，其 Component 内容、Surface 意图与空间布局同位（文件契约见 docs/components.md §Card 文件）。
- `AGENTS.md`：整个工作空间的导航信息；`index.md` 与 `AGENTS.md` 默认只读。它们不是 Config 域中用作系统提示词的 `AGENTS.md`。

notes 可用 `cards/<id>.card.json` 稳定相对路径引用 Card；Card 不是普通 note，不参与 note 索引，也不经 `read_memory` / `write_memory` 管理。Memory Workspace 跨重启保留，服务整个 Ambery 的跨项目理解与工作产物，不是 Context、压缩摘要或 Terminal Content 的副本。

#### ⟡ 一致性剖析

Card 文件是当前持续工作产物的真相，完整 JSON 同位保存 Component 内容、Surface 意图与空间布局；它不需要像 `cards.jsonl` 那样依靠最后一行折叠出当前状态。`effect.jsonl` 则只回答“什么动作发生过”：不 replay、不承担 Card 真相，也不能从窗口 opened / closed 事件反推哪些 Card 仍存在。文件即 Card 让工作产物有稳定地址，Memory note 可以引用它，而动作审计仍保持 append-only。

## Cron（Harness 持久化计划与延时调度）

概念与边界见 concepts §10g / docs/harness.md。`cron.jsonl` 持久化未来计划与延时调度，重启后 replay 折叠恢复；后端、用户和 Agent 都可管理。`cron_create` / `cron_delete` 与 `sleep` 共用其底层调度实现（waiters 不持久化）。append-only 事件行格式（create / fire / delete）与折叠规则见 **docs/cron.md**。

## effect.jsonl（前后端统一动作流）

Effect 动作流的 append-only 日志（docs/case-runner.md §可观测体系 / case-eval-system.md）：**记录动作，不驱动渲染**。后端副作用与前端 UI/运行时动作统一记录——包括前端执行过的 UI 动作（渲染气泡、显示 banner）。effect **不驱动**渲染：前端按自己的本地逻辑渲染（乐观气泡、自检测 banner），渲染后经前端上报通道记录该动作。Tauri 运行时动作涵盖 WebView 的 `@tauri-apps/api` 与 Rust 壳的 `tauri` API，均经运行时动作层在成功后记录。

```json
{"type":"effect","origin":"backend","kind":"render_component","payload":{...},"ts":1785600000000}
{"type":"effect","origin":"frontend","kind":"window_moved","payload":{"x":100,"y":200},"ts":1785600001000}
{"type":"effect","origin":"frontend","kind":"error_bubble","payload":{"message":"..."},"ts":1785600002000}
```

- `origin`：frontend / backend（发起者）
- `kind`：动作类型——后端（render_component / close_component / set_autonomy / config_changed / assistant_delta / assistant_done / llm_error）；前端（user_message / user_bubble / error_bubble / setup_banner / interaction / config_update / window_opened / window_closed / window_resized / window_moved / window_drag / window_visible / window_hidden / event_emit）
- `payload`：载荷（snake_case 字段名，storage 约定；与 WS 下发的 camelCase 形态无关）
- 高频（onMoved 拖动）打包成一条
- observe 的 `effects` 项（路径类）读它

**记录点（后端，单变体单记录点防双写）**：

| 变体 | 记录点 | 覆盖 |
|------|--------|------|
| config_changed | `edit_config_update` 收尾（LLM tool 路径，origin=backend） | LLM edit_config |
| config_update | 端点收尾（server post_config / Tauri set_config，origin=frontend） | 前端设置面板 |
| render_component / close_component / set_autonomy | `execute_tool` 收尾（内含早退，经 inner 包装单点记录） | LLM tool 循环 / case tool_call step / 测试 |
| assistant_delta / assistant_done / llm_error | `run_trigger` 的 sink 调用点 | 流式与非流式收尾 / LLM 失败降级 |
| user_message | `enqueue`（role==User 时，origin=frontend） | post_user / append_user / case user step |
| user_bubble / error_bubble / setup_banner | 前端 `reportEffect`（record_effect command / POST /effect，origin=frontend） | 前端渲染了用户气泡 / 错误气泡 / 未配置 banner |
| interaction | 端点（server post_event / Tauri push_event，origin=frontend） | 前端组件交互 |
| window_* / event_emit | Tauri 运行时动作层（WebView `record_effect` command / POST `/effect`，Rust 壳同一记录入口） | WebView / Rust 壳的非只读 Tauri 运行时动作（docs/effect-reporting.md） |

记录尽力而为不阻断主流（与 effect_sink 语义一致）；`Effect::effect_kind_payload` 是
kind/payload 的**穷尽 match**——新增 Effect 变体此处编译错（编译期强制进动作流）。

## 启动 replay 流程

1. `work-agents.jsonl` 逐行折叠 → 注册表投影。
2. `context.jsonl` 流式建索引：最新 `head`、最新 `autonomy`
   （归一全文不 replay——无持久存档；`terminal-content.jsonl` 不 replay）。
3. 写 `session` 标记；空 Context + 归零重同步：一条全景 system 消息（存活实例），落 `message` 行。
4. `AGENTS.md` 不存在 → bootstrap 写默认。
