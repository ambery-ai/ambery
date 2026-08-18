# Effort 设计

[English](effort.md) | 中文

> 跨模型 thinking / reasoning 预算的行业调研见 `reports/llm-reasoning-effort-cross-model.md`。

## 定义

effort 是**单次 LLM 调用的思考预算**，独立于 temperature（随机性）与工具调用预算（执行上限）。它决定这次调用花多少推理深度。

领域层只有统一语义档位，各 provider 适配层负责翻译成自己的 wire 参数：

```text
effort: low | medium | high | None
                          └─ None = 不设置，用该 provider 端点默认
```

| 端点 | wire 参数 | low | medium | high |
|---|---|---|---|---|
| OpenAI | `reasoning_effort` | `"low"` | `"medium"` | `"high"` |
| Anthropic 4.6+ | `output_config:{effort}` | `"low"` | `"medium"` | `"high"` |
| Anthropic 4.5- | `thinking:{type:"enabled", budget_tokens:N}` | ≈20%×max_tokens | ≈50% | ≈80% |
| DeepSeek v4 | `thinking:{type:"enabled"}` + 顶层 `reasoning_effort` | `"low"` | `"high"`（无 medium） | `"max"` |
| Gemini 3.x | `generation_config.thinking_level` | `"low"` | `"medium"` | `"high"` |
| 不支持的端点 | — | 忽略，不发送 | 忽略 | 忽略 |

**不支持时行为**：就近归并 + 告警，不报错（Vercel / OpenRouter / LiteLLM 一致）。绝不能把不认识的参数塞进 body——某些端点会 400。

## 档位来源：Queue 消息来源（不是 provider）

effort 由**触发这次 LLM 调用的 Queue 消息来源**决定。provider 只翻译、不决定档位。来源字段是 Queue 输入的一等公民，完整集合见 `docs/concrete-insight.md §Queue 中的 System 消息来源`。

### 当前映射（配置可覆盖，默认 medium）

配置预置三个直接值；未显式列出的来源一律使用默认 `medium`：

| 来源 | effort | Queue 优先级 | 理由 |
|---|---|---|---|
| `user_chat` | `low` | **高**（插队到最前） | 用户此刻盯着 pet 等回复 |
| `hook_stop_content`（stop auto_read 模式，带过滤后全量内容） | `high` | 默认（FIFO） | 有实质内容需要仔细读、判断 |
| 其他来源（`hook_stop_hint` / `hook_stop_report` / `hook_user_prompt` / `hook_notification` / `timer_scan` / `cron_tick` / `mock_hook`） | `medium`（默认） | 默认（FIFO） | 默认档位，不特殊对待 |

"快"由**优先级**保证（user_chat 插队最前），"认真"由 **effort** 保证（hook_stop_content 用 high）。两个旋钮各管各的：user_chat 即便插队，也不因插队调低或调高 effort。

### 实现落点

`QueueInput` 的来源字段与双队列放行机制见 `docs/harness.md §Queue 规则`。effort 侧只需在 `run_trigger` 的 LLM 调用点按来源解析档位：`user_chat` → low、`hook_stop_content` → high、其余 → medium。工具循环内后续调用沿用本次触发的来源与 effort。

## 匹配关键词

自动规则：用户 chat 消息命中配置的关键词时，把这次 `user_chat` 的 effort 临时改写。示例语义：

```text
"仔细想想" → effort 升到 high
"快点"     → effort 保持/降为 low
```

它是"输入 → 决定 effort"映射的一部分，与三分类默认属同一机制；关键词表走 Config 可配置，不硬编码。
