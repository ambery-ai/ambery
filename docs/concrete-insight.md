# Concrete Insight

English | [中文](concrete-insight.zh.md)

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

In the Queue, System messages = 8 source categories. There is also one category of User messages (sent directly from the user chat panel, server.rs:199) that does not belong to the System classification but still goes through Queue admission. See [effort.md](effort.md) for the mapping between sources and effort tiers.

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
  { role: "system",  content: <request header head> },          ← assembled per round from base_prompt + `config/AGENTS.md` + kaomoji pool
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
               content: { title: "ft done", text: "Done" } }
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

## Card Size Derivation

A Card's window size is three geometric quantities stacked: line boxes inside a block, block heights, and the chrome around them. Canvas answers one question only — how wide a run of text is — and the rest is arithmetic. Sample: one 175-character English paragraph in the Card font at 13px, card width cap 480px, line height 19.5px.

```
① canvas measures every segment once, cached by (font, segment); the Card's DOM is not involved

      "The"  " "   "card"  " "   "window"  " "   "is"   " "   "sized"  " "
     22.49  4.33  26.74  4.33   45.46   4.33  9.89   4.33  31.06   4.33   px

② the line walk accumulates advances and breaks at the last break opportunity that fits

   natural width (unwrapped) 1070.42 px   >   width cap 480 px
   → card width 480, content width 458 = 480 − 2×1 border − 2×10 padding

   ├──────────────────────── 458 px ────────────────────────┤
   │The card window is sized before it is created: canvas…  │ 418.11
   │segment once, the line walk packs segments into lines…  │ 419.73
   │heights stack into the projected size.                  │ 223.92
   └────────────────────────────────────────────────────────┘
   3 line boxes × 19.5 = 58.5 px   (a DOM box of the same width and line height measures 58.50 px)
   break opportunities come from word and grapheme segmentation; a trailing space does not
   count toward the line's width

③ block heights + chrome = the projected window size

   ┌──────────────────────── 480 ───────────────────────────┐   ↑
   │ border-top                                             │   1
   │ ↕ header padding-top                                   │   8
   │ title (nowrap + ellipsis, always one line)             │  19.5
   │ ↕ header padding-bottom                                │   4
   │ ↕ body padding-top                                     │   4
   │ paragraph 58.5 (3 × 19.5)                              │  58.5
   │ ↕ body padding-bottom                                  │  10
   │ border-bottom                                          │   1
   └────────────────────────────────────────────────────────┘   ↓
                                            window = 480 × 106
```

④ the same paragraph, same content width, same line height — only the font changes

```
   font                     natural width   line boxes   height
   "PingFang SC"               1070.42          3         58.5
   Helvetica                   1010.92          3         58.5
   "Times New Roman"            905.37          2         39.0
   "Courier New"               1365.22          4         78.0
```

The metrics belong to the resolved face, not to the family list: the app's default stack measures exactly like `"PingFang SC"` on macOS, because the leading `"Segoe UI"` is absent and the metrics come from the next family; on Windows the same stack resolves to Segoe UI and the same Card sizes differently. That is why the font must be named ([docs/card-window-size.md](card-window-size.md) §Font identity). On a CJK sample the Latin families fall back to the same CJK face, so only the Latin runs change width — a family list does not make CJK metrics portable.

The chrome constants are the Card's current style constants; a change to any of them, or to the font identity, changes `layoutVersion` and recomputes every projection. The result lands in `_meta.layout.size` ([docs/card-window-size.md](card-window-size.md)), which the shell reads before creating the window.
