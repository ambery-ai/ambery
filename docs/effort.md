# Effort Design

English | [中文](effort.zh.md)

> For the cross-model thinking / reasoning budget industry survey, see `reports/llm-reasoning-effort-cross-model.md`.

## Definition

effort is the **thinking budget for a single LLM call**, independent of temperature (randomness) and the tool-call budget (execution cap). It decides how much reasoning depth this call spends.

The domain layer has only unified semantic levels; each provider adapter translates them into its own wire parameters:

```text
effort: low | medium | high | None
                          └─ None = 不设置，用该 provider 端点默认
```

| Endpoint | wire parameter | low | medium | high |
|---|---|---|---|---|
| OpenAI | `reasoning_effort` | `"low"` | `"medium"` | `"high"` |
| Anthropic 4.6+ | `output_config:{effort}` | `"low"` | `"medium"` | `"high"` |
| Anthropic 4.5- | `thinking:{type:"enabled", budget_tokens:N}` | ≈20%×max_tokens | ≈50% | ≈80% |
| DeepSeek v4 | `thinking:{type:"enabled"}` + 顶层 `reasoning_effort` | `"low"` | `"high"` (no medium) | `"max"` |
| Gemini 3.x | `generation_config.thinking_level` | `"low"` | `"medium"` | `"high"` |
| Unsupported endpoints | — | Ignore, do not send | Ignore | Ignore |

**Behavior when unsupported**: coalesce to the nearest level + warn, do not error (consistent across Vercel / OpenRouter / LiteLLM). Never stuff an unrecognized parameter into the body — some endpoints will return 400.

## Level Source: Queue Message Source (Not the Provider)

effort is decided by **the Queue message source that triggered this LLM call**. The provider only translates; it does not decide the level. The source field is a first-class citizen of Queue input; for the complete set, see `docs/concrete-insight.md §Queue 中的 System 消息来源`.

### Current Mapping (Config Can Override, Default medium)

The config presets three direct values; any source not explicitly listed always uses the default `medium`:

| Source | effort | Queue priority | Rationale |
|---|---|---|---|
| `user_chat` | `low` | **High** (jumps to the front) | The user is watching pet and waiting for a reply |
| `hook_stop_content` (stop auto_read mode, with filtered full content) | `high` | Default (FIFO) | There is substantive content that needs careful reading and judgment |
| Other sources (`hook_stop_hint` / `hook_stop_report` / `hook_user_prompt` / `hook_notification` / `timer_scan` / `cron_tick` / `mock_hook`) | `medium` (default) | Default (FIFO) | Default level, no special treatment |

"Fast" is guaranteed by **priority** (user_chat jumps to the front of the queue), and "careful" is guaranteed by **effort** (hook_stop_content uses high). Each knob governs its own thing: even if user_chat jumps the queue, its effort is neither lowered nor raised because of the jump.

### Implementation Point

For the `QueueInput` source field and the dual-queue release mechanism, see `docs/harness.md §Queue 规则`. On the effort side, at the LLM call site in `run_trigger` resolve the level by source: `user_chat` → low, `hook_stop_content` → high, others → medium. Subsequent calls within the tool loop reuse the source and effort of this trigger.

## Matching Keywords

Automatic rule: when a user chat message hits a configured keyword, the effort of this `user_chat` is temporarily rewritten. Example semantics:

```text
"仔细想想" → effort 升到 high
"快点"     → effort 保持/降为 low
```

It is part of the "input → decide effort" mapping, the same mechanism as the three-category default; the keyword table goes through Config and is configurable, not hard-coded.
