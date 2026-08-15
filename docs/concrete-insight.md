# Concrete Insight

Real data + diagrams demonstrating the concept chain. No abstract descriptions.

## System Message Sources in the Queue

System messages entering the Queue are classified by source. The source field is a first-class citizen of Queue input (it drives mechanisms that behave by source, such as effort tier and priority).

```
Source               Enqueue point                    Content form
────────────────────────────────────────────────────────────
hook_stop_hint      ambery.rs:1213   stop queue_only product (hint)
hook_stop_content   ambery.rs:1213   stop auto_read product (full amount after Filter)
hook_stop_report    ambery.rs:1213   stop message product (report verbatim)
hook_user_prompt    ambery.rs:1154   "[Observation] User input in {name}: <prompt>"
hook_notification   ambery.rs:1158   "[Notice] {name}: <message>"
mock_hook           ambery.rs:1290   debug/test injection
timer_scan          ambery.rs:1449   "[Scan] {name} updated ({len} characters)"
cron_tick           server.rs:521      cron scheduled due message
```

In the Queue, System messages = 8 source categories. There is also one category of User messages (sent directly from the user chat panel, server.rs:199) that does not belong to the System classification but still goes through Queue admission. See `docs/effort.md` for the mapping between sources and effort tiers.

## Context → LLM API Role Mapping

Each message admitted by the Queue and written into Context carries one of the four roles; when assembling the LLM request (`core/src/llm.rs` `build_body`), they are mapped to OpenAI role strings:

```rust
let role = match m.role {
    Role::System    => "system",
    Role::User      => "user",
    Role::Assistant => "assistant",
    Role::Tool      => "tool",
};
```

The messages actually sent are a mixture of the four categories; `role: "system"` simultaneously carries three streams of content with different natures:

```
[
  { role: "system",  content: <request header head> },          ← assembled per round from base_prompt + AGENTS.md + kaomoji pool
  { role: "system",  content: <hook input verbatim> },          ← the one admitted by the Queue and written into Context
  { role: "user",    content: "Why is that bug happening?" },   ← user history message
  { role: "assistant", content: "Big change, putting up a card" }, ← pet history reply
  { role: "tool",    tool_call_id: "...", ... },                 ← tool result
  { role: "system",  content: <autonomy state> },                ← state appended each round
]
```

```
At Queue admission level: hook/timer/cron inputs are all System
After Context assembly: system / user / assistant / tool four categories are mixed
OpenAI role:"system" carries: request header + hook input + autonomy state
```

## Queue Serialization Timing

```
Input 1: "config-service finished (4958 characters). Evaluate whether to notify."
          ↓ Queue admits
┌─────────────────────────────────────────────────────────────┐
│ Context:  [+ system "config-service finished (4958 characters). Evaluate whether to notify."] │
│ LLM:  → assistant "Big change, putting up a card"           │
│ Context:  [+ assistant "Big change, putting up a card"]     │
└─────────────────────────────────────────────────────────────┘
          ↓ this round ends

Input 2: "anim-toolkit finished (2021 characters). Evaluate whether to notify."
          ↓ Queue (waits until Input 1 is processed before admission)
┌─────────────────────────────────────────────────────────────┐
│ Context:  [+ system "anim-toolkit finished (2021 characters)..."]   │
│ LLM:  → silence (no substantive change, do not notify)       │
│ Context:  no append                                          │
└─────────────────────────────────────────────────────────────┘
```

## Event Buffer Attachment

```
Input: "ambery·0a41f6ea finished (1472 characters). Evaluate whether to notify."
Event Buffer backlog: [
  "User closed text_card \"Build result\""
  "User checked todobox item \"Run tests\""
]

          ↓ Queue admits (Event Buffer attached and merged)

┌─────────────────────────────────────────────────────────────┐
│ Context write:                                               │
│   system: "ambery·0a41f6ea finished (1472 characters).      │
│            Evaluate whether to notify.                       │
│            Component interaction events:                     │
│            - User closed text_card \"Build result\"          │
│            - User checked todobox item \"Run tests\""        │
│                                                              │
│ LLM:  → assistant "The user just closed a card and checked   │
│                    a todo, hold off for now"                 │
│                                                              │
│ Context:  [+ system] [+ assistant]                           │
└─────────────────────────────────────────────────────────────┘
```

## Complete Turn (from Input to Output)

```
── Turn 1 ──

Queue admits: "demo-webapp finished (3800 characters). Evaluate whether to notify."
  + Event Buffer: "User closed text_card \"Summary\""

  Context: [+ system "..."]
  LLM:     tool_calls: [
             set_autonomy { key: "notify", motion: "bounce" },
             call_component { id: "notify-ft", type: "text_card",
               title: "ft done", text: "Done" }
           ]
  Context: [+ assistant (tool_calls)] [+ tool { ok: true }] [+ tool { ok: true, rendered: "notify-ft" }]
  LLM:     → assistant "Card popped up (´ω`)"

── Turn 2 ──

Queue admits: "unknown·414117ff requests attention: Claude is waiting for your input"

  Context: [+ system "..."]
  LLM:     → assistant "Someone is waiting for your input, go take a look?"

── When the Event Buffer is empty ──

Queue admits: "ambery·0a41f6ea finished. Evaluate whether to notify."
  Event Buffer: (empty)

  Context: [+ system "ambery·0a41f6ea finished. Evaluate whether to notify."]
  LLM:     → silence
```
