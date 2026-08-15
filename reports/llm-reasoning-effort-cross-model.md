# LLM 跨模型推理 effort / thinking 预算 —— 统一语义档位的调研报告

- 日期：2026-08-06
- 调研范围：为 ambery 设计"领域层 effort: low|medium|high + provider 适配层翻译"方案，查证各主流端点（Anthropic / OpenAI / DeepSeek / Gemini / Ollama 等）的 thinking / reasoning 参数 wire 形态，以及成熟框架与 agent 产品（LiteLLM / OpenRouter / Vercel AI SDK / LangChain / Claude Code / Cline / opencode / OpenClaw）如何做跨模型统一。

---

## 0. 一句话结论

2026 年的业界共识就是**"领域层统一语义档位 + provider 适配层翻译"**：Vercel AI SDK 的顶层 `reasoning` 参数、OpenRouter 的 `reasoning` 对象、OpenClaw 的 `thinkingDefault`、Cline 的 `reasoning` 结构都采用"一个跨模型档位，各端翻译成自己的 wire 参数"，并且在"不支持的端点"上采用**就近归并 + 告警（coerce to nearest + warn）**而非报错。ambery 候选的 3 档 `low|medium|high` 是所有端点的最小公倍数子集，方向正确、可落地。

---

## 1. 各端点官方参数

### 1.1 Anthropic / Claude

**字段名与取值形态**（两个时代并存，且 2026 正在迁移）：

| 形态 | wire 参数 | 取值 | 状态 |
|---|---|---|---|
| 手动 extended thinking（旧） | `thinking` | `{"type":"enabled","budget_tokens":N}` | Claude 4.6 起弃用；Claude 4.7+ 直接 400 拒绝 |
| adaptive thinking（新） | `thinking` | `{"type":"adaptive"}` | Claude 4.6+ 推荐；Claude 4.5 及更早会 400 拒绝 `type:"adaptive"` |
| effort（新，推荐控制手段） | `output_config` | `{"effort":"low"\|"medium"\|"high"\|"xhigh"\|"max"}` | 默认 `high`；**不需要开启 thinking 也能用** |

- `budget_tokens` 约束：最小值 1024；必须 `< max_tokens`（除非 interleaved thinking）；缓存下改变 budget 会使缓存失效。
- `output_config.effort` 语义：`max`/`xhigh`/`high`（=不设）/`medium`/`low`，是**行为信号而非严格 token 预算**——低档位在简单问题上可能完全不 thinking。支持模型：claude-fable-5 / mythos-5 / mythos-preview / opus-5 / opus-4-8 / opus-4-7 / opus-4-6 / sonnet-5 / sonnet-4-6 / opus-4-5-20251101。
- 陷阱：Claude Opus 5 上 `xhigh`/`max` effort 时**不允许** `thinking: {"type":"disabled"}`，否则 400。
- 官方文档 URL：
  - https://platform.claude.com/docs/en/docs/build-with-claude/extended-thinking
  - https://platform.claude.com/docs/en/docs/build-with-claude/effort
  - https://platform.claude.com/docs/en/docs/build-with-claude/thinking

**Claude Code / Agent SDK 是否暴露 effort 档位**：是。
- Claude Code 设置键：`effortLevel`（`"low"|"medium"|"high"|"xhigh"`，经 `/effort` 命令写入，`--effort` 覆盖单次，`CLAUDE_CODE_EFFORT_LEVEL` 环境变量）；`alwaysThinkingEnabled`（布尔）；`MAX_THINKING_TOKENS=0` 环境变量关闭 thinking（Fable 5 除外，不能关）。
- 来源：https://code.claude.com/docs/en/settings
- Claude Agent SDK 是"把 Claude Code 当库用"，底层走同一 API；其 model 配置同样支持 `thinking`（`type` + `budget_tokens`）与 `output_config.effort`（API 层已确认，SDK 具体字段名待查其 API reference）。

### 1.2 OpenAI

**字段名与取值形态**：
- **Chat Completions（本项目的 wire 格式）**：顶层字符串 `reasoning_effort`，取值 `"none" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max"`。模型相关：**并非所有模型支持所有档位**，需查各模型页。
- **Responses API**：`reasoning: {"effort": "...", "summary": "..."}`（同一档位集合）。
- 默认值：随模型不同——GPT-5.5 / GPT-5.6 默认 `medium`。
- 语义（2026 文档）：
  - `none`＝延迟关键、不需要推理；`low`＝轻量推理；`medium`＝多数负载默认；`high`＝复杂调试/深度规划；`xhigh`＝长时研究/异步；`max`＝极限能力。
- 与 o 系列关系：`reasoning_effort` 最早随 o1/o3 引入（当时只有 low/medium/high）；2026 已扩为 7 档并在 GPT-5.x 全系使用，o 系列只支持子集。
- 官方文档 URL：
  - https://developers.openai.com/api/docs/api-reference/chat/create
  - https://developers.openai.com/api/docs/guides/reasoning
- 陷阱：o 系列推理模型不允许设置 `temperature`/`top_p`（会报错或忽略），这与本项目默认发 `temperature` 冲突，见 §3.5。

### 1.3 DeepSeek

**当前模型（2026）**：`deepseek-v4-pro`、`deepseek-v4-flash`（`deepseek-chat`/`deepseek-reasoner` 为旧版别名/历史命名）。

**字段名与取值形态**（API reference 为准）：
- `thinking`: `{"type":"enabled"|"disabled"}`，**默认 enabled**（即不传也 thinking）。
- `thinking.reasoning_effort`: `"low" | "high" | "max"`，**默认 high**。
  - `deepseek-v4-flash`：支持全部 3 档。
  - `deepseek-v4-pro`：暂只支持 `high`/`max`（`low` 按 `high` 处理，`xhigh` 按 `max` 处理）。
- **注意**：官方 reasoning_model 指南的 curl 示例把 `reasoning_effort` 写成**顶层字段**（与 OpenAI 兼容风格），而 API reference 明确它在 `thinking` 对象**内部**。两处文档不一致，落地时需实测（本项目已接 DeepSeek，可直接验证）。
- 旧版 `deepseek-reasoner` 时代的 `budget_tokens` 在当前 v4 文档中已不出现（历史参数，未在当前 schema 中）。
- 官方文档 URL：
  - https://api-docs.deepseek.com/api/create-chat-completion
  - https://api-docs.deepseek.com/guides/reasoning_model

### 1.4 Google Gemini

**字段名与取值形态**（两代并存）：
- Gemini 3.x+：`generation_config.thinking_level`: `"minimal" | "low" | "medium" | "high"`。各模型默认：gemini-3.6-flash 默认 medium；gemini-3.1-pro-preview 默认 high；gemini-2.5-pro/flash 默认 on。
- Gemini 2.5：`generation_config.thinkingConfig.thinkingBudget`（整数 token 预算）。
- 计费：thinking tokens 与输出 tokens 合并计费。
- 官方文档 URL：https://ai.google.dev/gemini-api/docs/thinking

### 1.5 Ollama / vLLM / 其他端点

- **Ollama**：项目走 `http://localhost:11434/v1`（OpenAI 兼容层）。Ollama 原生推理参数与 OpenAI 兼容层是否透传 `reasoning_effort` **未证实/待实测**。OpenClaw 文档称 Ollama 的 `/think` 档位映射到原生 `think: "high"` 等（见 §2.6），但这是 OpenClaw 的实现，不是 Ollama 官方承诺。
- **vLLM**（Qwen3 / DeepSeek 等本地推理）：有独立的 thinking budget 控制（`reasoning_outputs` 功能）。来源：https://docs.vllm.ai/en/stable/features/reasoning_outputs/
- **xAI Grok**：Vercel AI SDK 将 xAI 列为支持顶层 `reasoning` 档位翻译的 provider（具体 wire 参数未查证，按 AI SDK 文档引用）。
- **Moonshot / Kimi**：项目默认 `kimi-k2`。OpenClaw 映射文档称 K3 恒为 `max`、K2.7 Code 仅二进制 `on`（即无档位可设）——此为 OpenClaw 的实现认知，**官方未证实**。
- **Z.ai GLM（zhipu）**：Cline 支持 `glm-thinking` 推理输出格式；Z.ai 的 effort/thinking 参数形态**未证实/待查**。

---

## 2. 跨模型统一是怎么做的

### 2.1 统一模式：领域层档位 + provider 翻译（业界共识）

2026 年主流做法高度收敛：**定义一个跨模型语义档位（ladder），在应用层选一档，由适配层翻译成目标端点的原生参数**。ladder 的"最大公约数"是：

```
none < minimal < low < medium < high < xhigh < max
（另有两个特殊档位：off/disabled、adaptive/auto）
```

各实现只取子集：

| 实现 | 档位数 | 语义 ladder |
|---|---|---|
| OpenAI | 7 | none/minimal/low/medium/high/xhigh/max |
| Anthropic effort | 5 | low/medium/high/xhigh/max |
| Vercel AI SDK | 6+default | provider-default/none/minimal/low/medium/high/xhigh |
| OpenRouter | 7 | none/minimal/low/medium/high/xhigh/max |
| OpenClaw | 9 | off/minimal/low/medium/high/xhigh/adaptive/max/ultra |
| Cline | 3 | low/medium/high（另有 budgetTokens） |
| ambery 候选 | 3 | low/medium/high |

### 2.2 Vercel AI SDK / AI Gateway（最接近"领域层统一"的范本）

- AI SDK 提供**顶层 `reasoning` 参数**：`'provider-default' | 'none' | 'minimal' | 'low' | 'medium' | 'high' | 'xhigh'`。文档原话："Each provider translates the value to its native reasoning API."（每个 provider 把这个值翻译成自己的原生 reasoning API）。
- 翻译规则（文档明示）：
  - **effort-based 端点**（OpenAI、Anthropic 4.6+）：直接透传档位；模型只支持子集时 **coerce 到最近档位 + 告警**。
  - **budget-based 端点**（Gemini 2.5、Anthropic 4.5-）：把档位映射为 **model 最大输出 tokens 的百分比**：`none`=关，`minimal`≈10%，`low`≈20%，`medium`≈50%，`high`≈80%，`xhigh`≈95%。
  - **不支持 reasoning 的端点**（Mistral / Perplexity / Cohere）：**忽略 + `unsupported` 告警**（不是报错）。
- 优先级规则：顶层 `reasoning` 与 providerOptions 里的 reasoning 参数**永不合并**；providerOptions 一旦设置就完全覆盖顶层。
- AI Gateway 还允许在 `/v1/chat/completions` 直接传 `reasoning: {"effort": "high"}` 或 `{"max_tokens": N}`，或 Anthropic Messages 格式传 `thinking: {type, budget_tokens}`，网关跨 provider 翻译（包括把 budget 换算成 effort）。
- URL：https://ai-sdk.dev/docs/ai-sdk-core/reasoning 、https://vercel.com/docs/ai-gateway/models-and-providers/reasoning

### 2.3 OpenRouter

- 提供**统一的 `reasoning` 对象**（body 参数）：`effort`（max/xhigh/high/medium/low/minimal/none）、`max_tokens`、`exclude`（内部用但响应不返回）、`enabled`、`context`（跨 turn 保留：auto/all_turns/current_turn）、`mode`（standard/pro）。
- 翻译到各端点：OpenAI→`effort`；Anthropic→`max_tokens`；Gemini 3→`thinkingLevel`；Alibaba Qwen→`thinking_budget`。
- **不支持时行为**：文档原话"OpenRouter will map your requested effort to the nearest supported level"——就近归并，**不报错**。
- 另有 `:thinking` 模型变体后缀（如 `deepseek/deepseek-r1:thinking`）开启扩展推理。
- URL：https://openrouter.ai/docs/guides/best-practices/reasoning-tokens 、https://openrouter.ai/docs/guides/routing/model-variants/thinking

### 2.4 LiteLLM（只统一响应，不统一请求）

- **请求侧不建统一抽象**：各 provider 用自己的参数透传——OpenAI/DeepSeek 用 `reasoning_effort`，Anthropic 用 `thinking`（含 `budget_tokens`），Responses API 用 `reasoning_effort={"effort":..., "summary":...}`。
- **响应侧统一**：把各家的 thinking 归一成 `reasoning_content` 与 `thinking_blocks`。
- **不支持时行为**：提供 `drop_params=True` 让 LiteLLM **丢弃不支持的参数**（"swap from Anthropic to Deepseek models"时 drop `thinking`）；`thinking` 与 `tool_calls` 冲突时自动 drop。
- 特有映射：`anthropic_effort` 参数把 `reasoning_effort` 映射为 Anthropic 的 `output_config={"effort":...}`（值 high/medium/low/max；Opus 4.5 需 `effort-2025-11-24` beta header，LiteLLM 自动注入）。
- URL：https://docs.litellm.ai/docs/reasoning_content 、https://docs.litellm.ai/docs/providers/anthropic_effort

### 2.5 LangChain（无统一抽象，每 provider 各写各的）

- 无跨模型统一档位。Python `BaseChatOpenAI.reasoning_effort` 支持 `minimal/low/medium/high`；Google 集成用 `thinking_budget`；langchain-aws 的 `reasoning_effort` 甚至还是待实现 feature request。
- 结论：LangChain 走"每 provider 各自字段"，**不做领域层归一**——这正是 ambery 想避免的老路。
- URL：https://reference.langchain.com/python/langchain-openai/chat_models/base/BaseChatOpenAI/reasoning_effort

### 2.6 agent 产品：Claude Code / Cline / opencode / OpenClaw

| 产品 | 统一方式 | 关键配置 |
|---|---|---|
| **Claude Code** | 无跨模型（只服务 Claude），但暴露档位 | `effortLevel`(low/medium/high/xhigh)、`alwaysThinkingEnabled`、`MAX_THINKING_TOKENS=0`、`/effort`、`--effort`、`CLAUDE_CODE_EFFORT_LEVEL` |
| **Cline** | **语义档位 + 每 provider 字段** | `GatewayStreamRequest.reasoning: {enabled, effort(low/medium/high), budgetTokens}`；Anthropic 系把档位换算成 budget 占比：low 20% / medium 50% / high 80% of max budget；推理输出格式：`anthropic-thinking` / `glm-thinking` / `minimax-thinking` |
| **opencode** | 每 provider 字段 + 内置 variant | OpenAI：`options.reasoningEffort`（none..xhigh）；Anthropic：`options.thinking: {type:"enabled", budgetTokens:N}`；提供 preset variants（Anthropic 的 high/max、OpenAI 的 none..xhigh） |
| **OpenClaw** | **领域层统一档位 + 每 provider 映射（与候选方案最像）** | `agents.defaults.thinkingDefault` / `agents.entries.*.thinkingDefault`（全局/每 agent）；9 个规范档位 `off/minimal/low/medium/high/xhigh/adaptive/max/ultra`；解析优先级：inline 指令 > session override > per-agent 默认 > 全局默认 > provider 兜底。每 provider 映射：Anthropic 4.6→`adaptive`+`output_config.effort:"xhigh"`；DeepSeek V4 `/think xhigh\|max`→`reasoning_effort:"max"`；OpenAI `/think off`→`reasoning.effort:"none"`；Ollama `max`→原生 `think:"high"`；Gemini `/think adaptive`→provider 自有动态 thinking；Moonshot K3 恒 `max`、K2.7 仅 `on` |

来源：
- Cline：https://deepwiki.com/cline/cline/4.5-advanced-provider-features
- opencode：https://opencode.ai/docs/models/
- OpenClaw：https://docs.openclaw.ai/tools/thinking
- Claude Code：https://code.claude.com/docs/en/settings

### 2.7 业界共识结论

1. **领域层统一档位 + provider 适配翻译是 2026 的主导模式**（Vercel AI SDK、OpenRouter、OpenClaw 都是；Cline 是"档位 + 每 provider 字段"的混合）。
2. **不支持时的默认行为 = 就近归并 + 告警，而不是报错**（Vercel、OpenRouter 一致；LiteLLM 用 drop_params 丢弃）。直接透传不认识的参数到某些严格端点会 400（见 §3.5）。
3. **budget-based 端点（Anthropic ≤4.5、Gemini 2.5）的换算比例存在跨框架共识**：`low≈20% / medium≈50% / high≈80% of max output tokens`（Vercel 与 Cline 给出几乎相同的表）。
4. **语义档位已是行为信号而非严格 token 预算**（Anthropic 明确）；档位 ladder 已收敛为 `none/minimal/low/medium/high/xhigh/max`，各厂商只取子集。

---

## 3. 对 ambery 的建议

### 3.1 现状核对（代码事实，2026-08-06）

- `core/src/llm.rs`：`OpenAiClient` 只有 `temperature` 可选参数；`build_body` 仅发 `model / messages / tools / temperature?`；流式已收 `reasoning_content` 两路，但**请求侧不发任何 thinking / effort 参数**。
- `core/src/config.rs`：`LlmProvider { base_url, model, api_key_env, temperature?, context_window?, compression_reserve? }`，**没有 effort 字段**。默认 5 个 provider 全部是 OpenAI 兼容 `chat/completions` 端点（deepseek / moonshot / zhipu / openai / ollama）。
- 即：适配层只需在 `build_body` 里按 provider 翻译一个 effort 字段，改动面小。

### 3.2 候选方案确认

**确认可行**。领域层 `effort: low|medium|high` + provider 适配层翻译 = 业界主导模式（§2.1/2.7），且 3 档是所有目标端点的最大公约子集：
- OpenAI：三档全支持 ✓
- DeepSeek v4：低/高/max，`medium` 需映射 ✓（有损但安全）
- Gemini 3.x：minimal/low/medium/high，三档全支持 ✓
- Anthropic：low/medium/high 全支持（走 `output_config.effort`）✓
- Ollama / Moonshot / zhipu：无档位或二进制——忽略或降级（见 §3.4）

### 3.3 档位 → 各端点 wire 映射表

领域档位 `low` / `medium` / `high`（`None` = 不设置，用 provider 默认）：

| 端点（vendor） | wire 参数 | low | medium | high | 依据 |
|---|---|---|---|---|---|
| OpenAI (chat/completions) | 顶层 `reasoning_effort` | `"low"` | `"medium"` | `"high"` | 文档证实 [1.2] |
| Anthropic Claude 4.6+ | `output_config: {effort}`（可选 + `thinking:{type:"adaptive"}`） | `"low"` | `"medium"` | `"high"` | 文档证实 [1.1] |
| Anthropic Claude 4.5-（budget-based） | `thinking:{type:"enabled", budget_tokens:N}` | N≈20%×max_tokens | N≈50%×max_tokens | N≈80%×max_tokens | 推断（Vercel/Cline 共识比例）[2.2][2.6] |
| DeepSeek v4 | `thinking:{type:"enabled", reasoning_effort:...}`（嵌套，[1.3] 与指南顶层写法不一致，待实测） | `"low"` | `"high"`（无 medium，取最近档位） | `"max"` | 文档证实 + 就近归并（推断）[1.3] |
| Google Gemini 3.x | `generation_config.thinking_level` | `"low"` | `"medium"` | `"high"` | 文档证实 [1.4] |
| Google Gemini 2.5 | `generation_config.thinkingConfig.thinkingBudget` | ≈20%×max_tokens | ≈50%×max_tokens | ≈80%×max_tokens | 推断（同 Anthropic budget 换算）[1.4][2.2] |
| Ollama (OpenAI 兼容 /v1) | 不支持档位 → 忽略 | — | — | — | 未证实/待实测 [1.5] |
| Moonshot / Kimi K2 | 二进制 on → 三档均视为开启 | — | — | — | 未证实（OpenClaw 认知）[2.6] |
| Z.ai GLM / zhipu | 无 effort 档位 → 忽略 | — | — | — | 未证实/待查 [1.5] |

> 标注说明：**文档证实**＝官方文档有明确字段与取值；**推断**＝按框架共识比例换算或就近归并策略；**未证实/待查**＝无官方文档，需实测。

### 3.4 不支持时的处理建议

- **原则：就近归并 + 告警，不报错**（Vercel / OpenRouter / LiteLLM 一致行为）。
- 具体策略（按 vendor 能力表）：
  1. 端点明确支持 effort → 翻译发送。
  2. 端点只支持子集（如 DeepSeek v4-pro：low→high）→ 归并到最近档位，`eprintln!` 告警。
  3. 端点不支持（Ollama / Moonshot / zhipu / 未知 vendor）→ **静默不发送 effort 字段**，仅打一条 debug 日志。**不要**把不认识的参数塞进 OpenAI 兼容 body——某些端点（Anthropic 4.7+ 的 `type:"enabled"`、严格兼容网关）会 400。
  4. 领域层应允许"不设置"（None）＝用 provider 默认，与"显式 low/medium/high"区分开。
- 建议在 `LlmProvider` 增加一个可选的 `vendor`（或 `kind`）枚举：`openai | deepseek | anthropic | gemini | ollama | other`，用默认 preset 填充；`other` 一律不发送 effort。

### 3.5 陷阱与反例（调研中发现的关键坑）

1. **Anthropic 4.7+ 拒绝 `thinking:{type:"enabled"}`（400）**——适配层必须按模型代际分支：4.6+ 用 `adaptive` + `output_config.effort`，4.5- 用 `budget_tokens`。**文档证实** [1.1]。
2. **Anthropic Opus 5：`xhigh`/`max` effort 下不允许 `thinking:{type:"disabled"}`（400）**。若 domain 有"关 thinking"档位，此组合要拦。**文档证实** [1.1]。
3. **OpenAI o 系列不允许 `temperature`**；DeepSeek 推理模型对 `temperature` 的容忍度未证实。本项目现在**无条件发 `temperature:0.3`**，接 reasoning 模型时可能冲突——建议 effort 档位为 high 时对已知 reasoning 模型跳过 temperature。**部分证实 / 需实测** [1.2]。
4. **DeepSeek 文档自相矛盾**：指南 curl 把 `reasoning_effort` 放顶层，API reference 放 `thinking` 内部。落地前必须对真实端点各打一发验证。**待实测** [1.3]。
5. **DeepSeek 无 `medium` 档**，且 `v4-pro` 的 `low` 按 `high` 处理——domain 三档映射到 DeepSeek 会有"medium 与 high 撞车"的现象，需要接受或有损。**文档证实** [1.3]。
6. **Anthropic budget 变动使 prompt cache 失效**；`output_config.effort` 同理（effort 被渲染进 prompt）。多轮长会话中**不要中途改档位**。**文档证实** [1.1][effort 页]。
7. **未知参数不全是"忽略"**：OpenAI/DeepSeek/Anthropic 对不认识的字段可能 400（Anthropic 已实证拒绝未知 `thinking.type`）。所以"传了不认识的参数会 400"是真实风险，§3.4 的静默策略就是为此。**部分证实**。
8. **Moonshot / Ollama 档位不可设**：与其发参数赌它兼容，不如按 vendor 表明确跳过。**未证实**。

### 3.6 实现落点（对现有代码的最小改动）

- `core/src/config.rs`：`LlmProvider` 加 `#[serde(default)] pub reasoning: Option<Effort>`（领域枚举 `Low|Medium|High`）或更宽的 ladder；默认 preset 里可不填（None）。
- `core/src/llm.rs` `OpenAiClient`：
  - `from_provider` 把 `p.reasoning` 与 `p.vendor` 存下；
  - `build_body` 末尾按 vendor 表把 domain effort 翻译进 body（§3.3）。
- 告警走既有 `eprintln!("[llm] ...")` 风格；归并/忽略逻辑收敛到一个 `fn apply_reasoning(body, vendor, effort)` 纯函数，便于单测（仓库现有测试风格可照 `openai_body_maps_tool_flow` 写）。
- 渲染层（`app/src/windows/chat.ts`）已支持 `reasoning_content` 流式展示，无需改。

---

## 4. 参考链接汇总

- Anthropic extended thinking：https://platform.claude.com/docs/en/docs/build-with-claude/extended-thinking
- Anthropic effort：https://platform.claude.com/docs/en/docs/build-with-claude/effort
- Anthropic thinking 总览：https://platform.claude.com/docs/en/docs/build-with-claude/thinking
- Claude Code settings：https://code.claude.com/docs/en/settings
- OpenAI reasoning 指南：https://developers.openai.com/api/docs/guides/reasoning
- OpenAI chat/create API 参考：https://developers.openai.com/api/docs/api-reference/chat/create
- DeepSeek API reference：https://api-docs.deepseek.com/api/create-chat-completion
- DeepSeek reasoning model 指南：https://api-docs.deepseek.com/guides/reasoning_model
- Gemini thinking：https://ai.google.dev/gemini-api/docs/thinking
- Vercel AI SDK reasoning：https://ai-sdk.dev/docs/ai-sdk-core/reasoning
- Vercel AI Gateway reasoning：https://vercel.com/docs/ai-gateway/models-and-providers/reasoning
- OpenRouter reasoning tokens：https://openrouter.ai/docs/guides/best-practices/reasoning-tokens
- OpenRouter thinking variant：https://openrouter.ai/docs/guides/routing/model-variants/thinking
- LiteLLM reasoning_content：https://docs.litellm.ai/docs/reasoning_content
- LiteLLM anthropic_effort：https://docs.litellm.ai/docs/providers/anthropic_effort
- LangChain BaseChatOpenAI.reasoning_effort：https://reference.langchain.com/python/langchain-openai/chat_models/base/BaseChatOpenAI/reasoning_effort
- Cline advanced provider features：https://deepwiki.com/cline/cline/4.5-advanced-provider-features
- opencode models：https://opencode.ai/docs/models/
- OpenClaw thinking：https://docs.openclaw.ai/tools/thinking
- vLLM reasoning outputs：https://docs.vllm.ai/en/stable/features/reasoning_outputs/
