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

Queue 中 System 消息 = 8 类来源。另有一类 User 消息（用户 chat 面板直接发送，server.rs:199）不属 System 分类，但同走 Queue 放行。来源与 effort 档位的映射见 [effort.md](effort.md)。

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
               content: { title: "ft 完成", text: "干完了" } }
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

## Card 尺寸推导

一张 Card 的窗口尺寸是三个几何量叠出来的：块内的行盒、块高、以及包在它们外面的 chrome。canvas 只回答一个问题——一段文字有多宽——其余都是算术。样本：Card 字体 13px 下的一段 79 字文本，卡宽上限 480px，行高 19.5px。

```
① canvas 对每个分段只测一次，按（字体, 分段）缓存；卡片的 DOM 不参与

      "构"   "建"   "完成"   "："   "3"   " "   "个"   "包"   "更新"   "，"
     13.00  13.00  26.00  13.00  7.80  4.33  13.00 13.00  26.00  13.00   px

② 行走一遍 advance，在最后一个放得下的断点处折行

   自然宽（不折行）950.04 px   >   卡宽上限 480 px
   → 卡宽 480，内容宽 458 = 480 − 2×1 border − 2×10 padding

   ├──────────────────────── 458 px ────────────────────────┤
   │构建完成：3 个包更新，core 有 2 个文件变更、app…        │ 451.31
   │中在 Card 尺寸投影与 markdown 渲染两处，需要…           │ 446.73
   │口尺寸。                                                │  52.00
   └────────────────────────────────────────────────────────┘
   3 个行盒 × 19.5 = 58.5 px   （同宽同行高的 DOM 盒子实测 58.50 px）
   断点来自分词与字素分段；行尾空格不计入行宽

③ 块高 + chrome = 投影出的窗口尺寸

   ┌──────────────────────── 480 ───────────────────────────┐   ↑
   │ border-top                                             │   1
   │ ↕ header padding-top                                   │   8
   │ 标题（nowrap + 省略号，恒为一行）                      │  19.5
   │ ↕ header padding-bottom                                │   4
   │ ↕ body padding-top                                     │   4
   │ 段落 58.5（3 × 19.5）                                  │  58.5
   │ ↕ body padding-bottom                                  │  10
   │ border-bottom                                          │   1
   └────────────────────────────────────────────────────────┘   ↓
                                           窗口 = 480 × 106
```

④ 同一段英文样本、同一内容宽、同一行高——只有字体变了（中文样本换字体行数不变，看不出差别，故用英文样本）

```
   字体                      自然宽     行盒数   高度
   "PingFang SC"            1070.42       3     58.5
   Helvetica                1010.92       3     58.5
   "Times New Roman"         905.37       2     39.0
   "Courier New"            1365.22       4     78.0
```

度量属于解析后的那个字体，不属于字体列表：应用默认字体栈在 macOS 上量出的结果与 `"PingFang SC"` 完全一样，因为排在第一的 `"Segoe UI"` 不存在，度量来自下一个族；同一个字体栈在 Windows 上解析到 Segoe UI，同一张 Card 就会算出不同尺寸。这就是字体必须命名（docs/card-window-size.zh.md §字体标识）的原因。中文样本上拉丁字体会回退到同一个中文字体，只有拉丁部分宽度变化——字体列表并不能让中文度量变得可移植。

chrome 常量取自 Card 当前的样式常量；它们或字体标识一变，`layoutVersion` 就变，全部投影重算。结果落到 `_meta.layout.size`（docs/card-window-size.zh.md），壳在建窗前读它。
