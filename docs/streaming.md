# Streaming Delta

English | [中文](streaming.zh.md)

Streaming transport of LLM replies — assistant output is pushed to the frontend incrementally, fragment by fragment, rather than delivered all at once.

## Concepts

| Concept | Description |
|------|------|
| **Delta** | A small piece of LLM output text, without waiting for the complete reply |
| **StreamingChannel** | The push channel for Delta, independent of Queue/Context/Event Buffer |
| **ThinkingBubble** | Frontend transparent bubble; the animated "…" shown while reasoning_content is being displayed |
| **ThinkingModal** | The modal expanded when ThinkingBubble is clicked; shows reasoning_content in streaming fashion |
| **ContentDelta** | ordinary assistant text delta, directly appended to the chat bubble |

## Chain

```
LLM SSE chunk ─→ parse ─→ Effect::AssistantDelta { content?, reasoning_content? }
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
        content non-empty               reasoning_content non-empty
        directly append to chat bubble  ThinkingBubble + ThinkingModal
```

- Delta does not enter Queue (Queue only handles input)
- Delta does not enter Context (Context is written only after the complete LLM reply)
- Delta is a pure display optimization — reducing time to first token

## OpenAI SSE Format

```
{ "choices": [{ "delta": { "reasoning_content": "The user wants..." } }] }
{ "choices": [{ "delta": { "content": "Okay, let me..." } }] }
{ "choices": [{ "delta": { "content": "Let me analyze this." } }] }
{ "choices": [{ "finish_reason": "stop" }] }
```

The two fields are mutually exclusive: a chunk is either reasoning (thinking phase) or content (reply phase); they are never non-empty at the same time.

## Implementation Layers

| Layer | Change |
|----|------|
| `Llm` trait | Add a `complete_streaming(on_delta)` method, defaulting back to a one-shot callback |
| `OpenAiClient` | override: set `stream: true` → `resp.chunk()` → SSE parsing |
| `Effect` | Add `AssistantDelta { content, reasoning_content }` + `AssistantDone` |
| `AmberyBackend` | `effect_sink: Option<Arc<dyn Fn(&Effect) + Send + Sync>>`; inside run_trigger, push each delta as soon as it arrives |
| `Bridge` | Add `onAssistantDelta(cb)` + `onAssistantDone(cb)` |
| `RemoteBridge` | New WS handler cases: `assistant_delta` / `assistant_done` |
| `chat.ts` | On delta, directly append to the DOM; on done, remove the loading bubble |
| `ThinkingBubble` | When reasoning_content is non-empty, show the animated "…"; clicking it expands ThinkingModal |
| `ThinkingModal` | Inline modal; appends reasoning_content in streaming fashion |

## Frontend Behavior

```
Delta arriving:
  content:           "Okay, let me..." ─→ append "Okay, let me..." to the chat bubble
  content:           "Let me analyze this." ─→ append "Let me analyze this." to the chat bubble
  reasoning_content: "The user wants..." ─→ ThinkingBubble shows + Modal writes "The user wants..."
  content:           "The result is..." ─→ ThinkingBubble disappears, chat bubble appends "The result is..."
  finish_reason:     "stop"       ─→ loading disappears, complete reply enters Context
```
