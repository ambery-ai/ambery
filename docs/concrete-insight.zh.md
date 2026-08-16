# Concrete Insight

[English](concrete-insight.md) | 中文

真实数据 + 图演示概念链路。不写抽象描述。

## Queue 中的 System 消息来源

进入 Queue 的 System 消息按来源分类。来源字段是队列输入的一等公民（驱动 effort 档位与优先级等按来源定行为的机制）。

```
来源                 入队点                              内容形态
────────────────────────────────────────────────────────────
hook_stop_hint      ambery.rs:1213   stop queue_only 产物（hint）
hook_stop_content   ambery.rs:1213   stop auto_read 产物（filter 后全量）
hook_stop_report    ambery.rs:1213   stop message 产物（汇报原文）
hook_user_prompt    ambery.rs:1154   "[观察] 用户在 {name} 输入：<prompt>"
hook_notification   ambery.rs:1158   "[通知] {name}：<message>"
mock_hook           ambery.rs:1290   debug/测试注入
timer_scan          ambery.rs:1449   "[扫描] {name} 更新（{len} 字）"
cron_tick           server.rs:521      cron 计划到期消息
```

Queue 中 System 消息 = 8 类来源。另有一类 User 消息（用户 chat 面板直接发送，server.rs:199）不属 System 分类，但同走 Queue 放行。来源与 effort 档位的映射见 `docs/effort.md`。

## Context → LLM API 的 role 映射

Queue 放行写进 Context 的每条消息带四种 role；组装 LLM 请求时（`core/src/llm.rs` `build_body`）映射为 OpenAI 的 role 字符串：

```rust
let role = match m.role {
    Role::System    => "system",
    Role::User      => "user",
    Role::Assistant => "assistant",
    Role::Tool      => "tool",
};
```

实际发送的 messages 是四类混合，`role: "system"` 同时承载三路不同性质的内容：

```
[
  { role: "system",  content: <请求头 head> },          ← 每轮现拼 base_prompt + AGENTS.md + 表情池
  { role: "system",  content: <hook 输入原文> },         ← Queue 放行写进 Context 的那条
  { role: "user",    content: "那个 bug 怎么回事？" },   ← 用户历史消息
  { role: "assistant", content: "有大变更，挂卡片" },     ← pet 历史回复
  { role: "tool",    tool_call_id: "...", ... },         ← 工具结果
  { role: "system",  content: <autonomy 状态> },         ← 每轮追加的状态
]
```

```
Queue 入队层面：hook/timer/cron 输入均为 System
Context 组装后：system / user / assistant / tool 四类混发
OpenAI 的 role:"system" 承载：请求头 + hook 输入 + autonomy 状态
```

## Queue 串行化时序


```
输入1: "config-service 完成（4958 字）。评估是否通知。"
          ↓ Queue 放行
┌─────────────────────────────────────────────────────────────┐
│ Context:  [+ system "config-service 完成（4958 字）。评估是否通知。"] │
│ LLM:  → assistant "有大变更，挂卡片"                           │
│ Context:  [+ assistant "有大变更，挂卡片"]                     │
└─────────────────────────────────────────────────────────────┘
          ↓ 本轮结束

输入2: "anim-toolkit 完成（2021 字）。评估是否通知。"
          ↓ Queue（等输入1 处理完才放行）
┌─────────────────────────────────────────────────────────────┐
│ Context:  [+ system "anim-toolkit 完成（2021 字）..."]   │
│ LLM:  → silence（无实质变更，不通知）                            │
│ Context:  无追加                                                │
└─────────────────────────────────────────────────────────────┘
```

## Event Buffer 附带入

```
输入: "ambery·0a41f6ea 完成（1472 字）。评估是否通知。"
Event Buffer 积压: [
  "用户关闭了 text_card「构建结果」"
  "用户勾选了 todobox 条目「跑测试」"
]

          ↓ Queue 放行（Event Buffer 附带合并）

┌─────────────────────────────────────────────────────────────┐
│ Context 写入:                                                  │
│   system: "ambery·0a41f6ea 完成（1472 字）。          │
│            评估是否通知。                                        │
│            Component 交互事件：                                  │
│            - 用户关闭了 text_card「构建结果」                     │
│            - 用户勾选了 todobox 条目「跑测试」"                   │
│                                                                │
│ LLM:  → assistant "用户刚关了卡片还勾了 todo，先不打扰"            │
│                                                                │
│ Context:  [+ system] [+ assistant]                              │
└─────────────────────────────────────────────────────────────┘
```

## 完整 turn（从 Input 到 Output）

```
── 第 1 个 turn ──

Queue 放行: "demo-webapp 完成（3800 字）。评估是否通知。"
  + Event Buffer: "用户关闭了 text_card「摘要」"

  Context: [+ system "..."]
  LLM:     tool_calls: [
             set_autonomy { key: "notify", motion: "bounce" },
             call_component { id: "notify-ft", type: "text_card",
               title: "ft 完成", text: "干完了" }
           ]
  Context: [+ assistant (tool_calls)] [+ tool { ok: true }] [+ tool { ok: true, rendered: "notify-ft" }]
  LLM:     → assistant "卡片已弹出 (´ω`)"

── 第 2 个 turn ──

Queue 放行: "unknown·414117ff 请求注意：Claude is waiting for your input"

  Context: [+ system "..."]
  LLM:     → assistant "有人等你输入，去看一下？"

── Event Buffer 空时 ──

Queue 放行: "ambery·0a41f6ea 完成。评估是否通知。"
  Event Buffer: (空)

  Context: [+ system "ambery·0a41f6ea 完成。评估是否通知。"]
  LLM:     → silence
```