# Agent Loop 设计

> 概念定义见 concepts.md §2/§9b/§10a。本文档定 LLM 抽象、Tool Set 协议与 mock hook 契约。


## 原则

> **不提供不必要存在的代码逻辑**——预算耗尽后仍沿用既有的 tool result → LLM 收尾链路：以空 tools 正常请求一次最终文字回复；不额外注入终止 system 记录，不自动开启新 turn。
> **本文档范围**——本文定义触发生命周期、LLM 请求、工具调用预算及耗尽收尾；Config 的 migration、map reconcile 与 descriptor 机制见 `docs/config.md`。

## LLM 抽象

```rust
trait Llm {
    async fn complete(&self, messages: &[ContextMessage], tools: &[ToolDef]) -> LlmOutput;
}
struct LlmOutput { content: Option<String>, tool_calls: Vec<ToolCall> }
```

实现与装配（`LlmBackend::from_config`）：
- `active: "debug"` → `DebugAgent`：纯 mock，零逻辑。将全量 Context 交给外部决策源（沉默/脚本/HTTP brain），原样返回决策源给的 `LlmOutput`（docs/debug-agent.md）。
- `active: "<provider 名>"` → `OpenAiClient`（OpenAI 兼容端点），**失败时降级 DebugAgent**（初始化失败：env 未设/provider 不存在 → 整体回退；调用失败：HTTP/超时/解析 → 当轮回退）。失败不再静音：降级同时产出 `llm_error` effect，pet 渲染「LLM 调用失败」错误帧卡片（同 id `llm-error` 原地更新）。

**Config v2（多 profile + active 选择器）**：providers 存各家 `base_url/model/api_key_env/temperature`，切换只改 `active` 不丢配置；key 本体只在环境变量里。首次启动 config.json 不存在时写入默认预设（deepseek/moonshot/zhipu/openai/ollama 公开厂商）；私有 provider 由用户自行加入本地 config.json。

**thinking 模型的 reasoning_content 回传要求**：thinking 模型开 thinking 模式时，历史消息里任何带 `tool_calls` 的 assistant 消息**必须带回 `reasoning_content` 字段**，否则 400（空串可过，`thinking:{type:"disabled"}` 不被接受）。因此 ContextMessage 持久化 `reasoning_content`（旧记录无此字段，回放补空串），AmberyBackend 在每轮 assistant tool_calls 消息上存思维链。

**记录 ≠ 回放**：纯文本回复的 reasoning_content **同样全保真落盘**（context.jsonl 可审计、debug/case 可查思维链），但**不回放**——deepseek 官方要求多轮不回传 reasoning（每轮思维一次性），build_body 只在 tool_calls 分支写 reasoning_content，纯文本回复不花 token、无 400 风险。

## Tool Set 设计原则

> **不做特殊规则，用语义明确行为**——tool 参数由 schema 完全定义，禁止隐式约定（缺参切换模式、空值触发查询等）。实现单源见 `core/src/llm.rs` `tool_set()`。

> **渐进披露，按需查**——Config 多层嵌套，LLM 通过 tool 调用-反馈逐层发现 path 和类型，不依赖外部 Schema 注入。

## Tool Set 协议（concepts §10a）

九个 function definitions，CLI 风格命名，AmberyBackend 执行后以 `tool` role message 追加 result：

| tool | 参数 | 执行 | result |
|---|---|---|---|
| `call_component` | `spec: ComponentSpec`（docs/components.md 协议） | Tauri 事件推送渲染指令给前端；同 id = 创建/原地更新 | `{ok, rendered/updated/closed: id}` |
| `fetch_terminal` | `{instance, vd_switch}`（vd_switch 必填，docs/hook.md §VD 切换能力） | 经 Terminal Adapter 读 Terminal Content（docs/terminal-adapter.md）；读不到回退 Context 最新记录 | `{instance, content}` |
| `set_autonomy` | `{key?, motion?, ttlMs?, once?}` | Tauri 事件推送表情覆盖（语义 docs/autonomy.md） | `{ok}` |
| `edit_config` | `{action, ...}`（`grep` / `query` / `update`） | 受限 Config 投影中的发现、读取与修改；完整 schema 见 docs/toolset.md | action 对应的结果 |
| `read_memory` | `{...}` | 读取 Harness 管理的持久化理解 Markdown；`index.md` / Memory `AGENTS.md` 默认只读 | `{ok, ...}` |
| `write_memory` | `{...}` | 创建或完整替换一条碎片化记忆；必须带 description，受单文件长度上限约束 | `{ok, ...}` |
| `cron_create` | `{...}` | 新建 Harness 的持久化计划或延时调度 | `{ok, ...}` |
| `cron_delete` | `{...}` | 删除一个持久化计划 | `{ok, ...}` |
| `sleep` | `{...}` | 经同一 Harness 调度器等待后继续既定工具序列 | `{ok}` |

权限边界：Tool Set 即全部能力，无修改代码文件的 tool（concepts §10a ❌ 项不存在于定义表）。

## 一条 Queue 输入的完整 turn（docs/harness.md §触发模型 的执行器）

一个 turn 由 Queue 放行一条输入驱动，串行执行——当前 turn 未完不放行下一条（concepts §10c 不可并行）：

1. Queue 放行一条输入（附带 merge Event Buffer → 合并为一条，有则）→ Context 写输入
2. 现拼 system prompt 请求头（base_prompt + AGENTS.md + 系统表情池，不落 Context；用户表情池按需经 `edit_config` 查询；concepts §12）
3. Compression 检查（auto-compact：Context 超阈值 → 专项摘要 + shaking + 归零重 diff）
4. LLM（请求 = 请求头 + Context 全部消息）→ 有 tool_calls：追加 assistant(tool_calls) + 按声明顺序执行 + 追加对应 tool results → 再调用；无 tool_calls：content 非空才追加 assistant 消息，结束。工具调用预算见下。
5. 副作用（Effect）经 Tauri 事件广播给前端；本轮完毕，Queue 放行下一条

### 工具调用预算

两个本地 Config 运行预算：

| 字段 | 默认 | 生效 | agent 访问 | 含义 |
|---|---:|---|---|---|
| `max_tool_calls_in_one_response` | 10 | 冷 | `no_llm_visible` | 一次 LLM response 最多执行的 tool call 数（≥ 1） |
| `max_tool_calls_per_turn` | 50 | 冷 | `no_llm_visible` | 一条已放行输入处理期间累计最多执行的 tool call 数（≥ 1） |

工具 call 仍按 response 中的声明顺序串行执行。超出任一预算的 calls 不执行，但每个 call 仍写入对应的失败 tool result；已提出的 calls（包括未执行者）都计入本 turn 预算。单 response 超额时，预算以内的 calls 正常执行，后续 calls 返回预算错误。

当本 turn 预算耗尽，已执行与未执行 calls 的 tool results 照常写入 Context；后端随后以空 tools 正常请求一次 LLM，使其基于这些结果生成最终文字回复。该收尾请求不追加特殊 system 记录，也不能再发起 tool call；回复后本 turn 正常结束。

**沉默语义**（设计决定）：LLM 返回空 content 且无 tool_calls = 决定沉默——Context 不追加任何 assistant 消息（「pet 可以醒了、读了、觉得不需要打扰，沉默」concepts §9b）。

## Mock Hook 契约（HTTP）

> **真实契约见 docs/hook.md**（事件分层 / session_id 身份 / marker 定位 / 启动扫描）。
> 本节的 mock 契约保留为 **debug 手段**（不装 hook 时手动驱动链路）。

```
POST /hook
{
  "event": "session_start" | "stop",
  "instance": "demo-webapp",        // Code CLI 实例名（tab 名）
  "project": "ambery",       // 项目名
  "content": "……",                      // stop 时：模拟读到的 Terminal Content（真实由 sidecar 读）
  "last_assistant_message": "……"        // stop 时：模拟 hook payload 自带字段
}
```

处理：
- `session_start` → 实例注册（status=idle）+ Event Buffer 静默簿记「新实例 {instance} 已注册」——不触发，pet 不醒；下次 Queue 放行时附带入 Context
- `stop` → 实例更新（status=idle）+ Queue system 输入「{instance} 完成（{len} 字）。评估是否通知。」→ 放行后写 Context → 触发

## 读通道 MapAdapter（case-runner 剧情面）

与 mock hook 对称的读通道模拟：case-runner 的 `terminal` step 写 MapAdapter 共享 map（`docs/terminal-adapter.md` §实现），Timer 兜底扫描与 `fetch_terminal` 都读它。生产/默认构建不含该注入面。

## Tauri IPC 协议（前后端通信）

前端与 core 通信走 Tauri 原生 IPC，仅外部 hook 脚本走 HTTP。

```
Tauri command（前端 → Rust）：
  invoke("get_state")          → TopState（instances + pendingNotifications）
  invoke("get_context")        → ContextMessage[]（对话历史投影）
  invoke("append_user", {text}) → 用户输入入 Queue 排队 → 放行后写 Context user role + 触发
  invoke("push_event", {action, card_id?, ...}) → 结构化用户动作（写 Event Buffer，docs/effect-reporting.md）
  invoke("get_config")          → AppConfig

Tauri event（Rust → 前端）：单事件 `listen("effect", {kind, ...})`，按 kind 判别：
  render_component {spec} / close_component {id} / set_autonomy {face?, motion?, ttlMs?}
  top_state {state} / context_changed {} → 前端收到后重新 invoke("get_context")
  config {} → 裸信号，按需重拉 invoke("get_config")

HTTP（127.0.0.1:47600，仅外部 hook 脚本使用）：
  POST /hook         → hook 脚本触发（fire-and-forget）
```
