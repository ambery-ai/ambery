# Streaming Delta

LLM 回复的流式传输——assistant 输出以增量片段逐条推送到前端，而非完整一次性交付。

## 概念

| 概念 | 说明 |
|------|------|
| **Delta** | LLM 输出的一小段文本，不等完整回复 |
| **StreamingChannel** | Delta 的推送管道，与 Queue/Context/Event Buffer 均无关 |
| **ThinkingBubble** | 前端透明气泡，显示 reasoning_content 时的动画 "…" |
| **ThinkingModal** | 点击 ThinkingBubble 展开的弹窗，流式显示 reasoning_content |
| **ContentDelta** | ordinary assistant 文本增量，直接追加到 chat 气泡 |

## 链路

```
LLM SSE chunk ─→ parse ─→ Effect::AssistantDelta { content?, reasoning_content? }
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
        content 非空                     reasoning_content 非空
        直接追加 chat 气泡                ThinkingBubble + ThinkingModal
```

- Delta 不入 Queue（Queue 只管输入）
- Delta 不入 Context（仅在 LLM 完整回复后才写 Context）
- Delta 是纯显示优化——降低首字延迟

## OpenAI SSE 格式

```
{ "choices": [{ "delta": { "reasoning_content": "用户想要..." } }] }
{ "choices": [{ "delta": { "content": "好的，我来..." } }] }
{ "choices": [{ "delta": { "content": "分析一下..." } }] }
{ "choices": [{ "finish_reason": "stop" }] }
```

两个字段互斥：一个 chunk 要么是 reasoning（thinking 阶段），要么是 content（回复阶段），不会同时非空。

## 实现层

| 层 | 改动 |
|----|------|
| `Llm` trait | 新增 `complete_streaming(on_delta)` 方法，默认回退一次性回调 |
| `OpenAiClient` | override：设 `stream: true` → `resp.chunk()` → SSE 解析 |
| `Effect` | 新增 `AssistantDelta { content, reasoning_content }` + `AssistantDone` |
| `AmberyBackend` | `effect_sink: Option<Arc<dyn Fn(&Effect) + Send + Sync>>`，run_trigger 内每收到 delta 即推 |
| `Bridge` | 新增 `onAssistantDelta(cb)` + `onAssistantDone(cb)` |
| `RemoteBridge` | WS handler 新 case：`assistant_delta` / `assistant_done` |
| `chat.ts` | 收到 delta 直接追加 DOM，收到 done 时移除 loading 气泡 |
| `ThinkingBubble` | reasoning_content 非空时显示动画 "…"，点击展开 ThinkingModal |
| `ThinkingModal` | 内联弹窗，流式追加 reasoning_content |

## 前端行为

```
Delta arriving:
  content:           "好的，我来" ─→ chat 气泡追加 "好的，我来"
  content:           "分析一下"   ─→ chat 气泡追加 "分析一下"
  reasoning_content: "用户想..."  ─→ ThinkingBubble 显示 + Modal 写 "用户想..."
  content:           "结果是..."  ─→ ThinkingBubble 消失，chat 气泡追加 "结果是..."
  finish_reason:     "stop"       ─→ loading 消失，完整回复入 Context
```
