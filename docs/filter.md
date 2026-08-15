# Filter Design

> Concept definition: see concepts.md §11. This document fixes the policy rules and the structure-understanding data types.
> **Structure understanding**: the rules are taken from 3 real Claude Code terminal UIA samples (UIA returns rendered plain text, no ANSI codes).

## Structure Understanding Data Types

Each time filter processes terminal text it **fills a new data record field by field**; fields that filter cannot see or cannot judge confidently are left as `None` (never hard-filled).

```rust
pub struct TerminalDigest {
    pub blocks: Vec<ContentBlock>,
}

pub enum ContentBlock {
    /// 用户输入（❯ 头，可跨折行）
    UserPrompt { text: String },
    /// 助手正文（● 头，续行缩进无 glyph，可跨多行）
    AssistantText { text: String },
    /// tool 调用：● Tool(args) 头 + ⎿ 结果 + 展开体全文 + 原文自带折叠标记
    ToolCall {
        head: String,               // ● Write(path) 整头行
        result: Option<String>,     // ⎿ 后的结果摘要（可能没有）
        body: String,               // 展开体全文（diff/编号内容行）——不做任何省略
        truncated: bool,            // 原文自带折叠尾（… +N lines，源已丢信息，仅标记）
    },
    /// 会话压缩摘要（※ recap: 头，多行）
    Recap { text: String },
    /// 无 ● 头的系统注入行（⎿ 3 skills available）
    SystemInject { text: String },
    /// 未能归类但非噪声（兜底，不丢信息）
    Info { text: String },
}
```

Note: **diff is not a separate block type** — it is the content of the ToolCall expanded body (the display form of Write/Edit), kept in full in `body`. Sample basis: the `594 +| ...` lines in win0 all hang under the Write tool.

## Processing Pipeline (In Order)

1. **R0 trim_end**: remove the UIA grid right padding.
2. **Wrapped-line joining**: physical line → logical line. Terminal width hard-wraps long lines into several physical lines; continuation lines carry alignment spaces / diff prefix columns (win0's markdown table is wrapped to pieces). Splitting rules:
   - New logical line: glyph head (`❯ ● ⎿ ※ ✻`), diff lines (`^\s*\d+\s*[+-]|`), Write expanded numbered content lines (`^\s{4,}\d+ `)
   - Other non-empty lines = continuation candidates: only when the previous physical line width ≥ 90% of the grid width is it judged a hard wrap and joined into the current logical line (hard wraps have no space); short lines stay independent logical lines (no blind joining).
3. **Denoise** (line level; rules see the policy file).
4. **Block splitting + fill TerminalDigest**: glyph head starts a new block, indented continuation lines belong to the current block.
5. **render**: digest → normalized text — **keep everything, never omit** (design decision: the limited-quantity problem is not solved; folding = omitting content for the LLM, also not done). Source-native fold markers (`… +N lines`) are preserved as-is in body.
6. **detect_change**: operates on the rendered text, line-set Jaccard (≥0.8 Minor / otherwise Substantive). Scroll false positives are accepted (design decision).

## Policy Files (concepts §11 Replaceable Policies)

```
core/src/filter/
  mod.rs      # trait + TerminalDigest/ContentBlock + render + detect_change 默认实现 + by_name
  claude.rs   # Claude Code 规则
  opencode.rs # OpenCode 规则（glyph 表）
```

trait:

```rust
pub trait Filter {
    /// 去噪 + 归一文本（策略必须实现）
    fn apply(&self, raw: &str) -> String;
    /// 去噪 + 折行合并 + 块切分 + 填 TerminalDigest（默认实现 = apply + 整篇 Info）
    fn digest(&self, raw: &str) -> TerminalDigest;
    /// 变化检测（作用于 render 文本，默认实现可共享）
    fn detect_change(&self, prev: &str, next: &str) -> Change;
}
```

Filter selects the per-instance hook `kind` (docs/hook.md §Payload); currently supported: `"claude"` / `"opencode"`; a missing or unsupported kind is rejected directly before the instance state update, Terminal Content read, Filter, and Queue.

## claude.rs Noise List (From Real Samples)

| Noise | Example | Why it changes |
|---|---|---|
| Trailing right padding | UIA returns whole rows per the terminal grid | Width-related, no semantics |
| spinner/duration lines | `✻ Crunched for 22s`, `Thought for 7s, ran 1 shell command`, braille-character lines | Changes every frame |
| **Scheduled-task line** | `✻ Running scheduled task (Jul 26 8:30pm)` | Has no `for Xs` suffix; different shape from duration lines |
| Bottom separator | `─────── npc-prof ──` | Width-related |
| Empty prompt | `❯` (no text) | Always present |
| Model/cost line | `deepseek-v4-pro  $12.34` | Cost accumulation changes |
| git status line | `●● on  master` (whole line + suffix variants) | Workspace changes |
| Permission hint line | `⏵⏵ bypass permissions on (shift+tab to cycle)` | Mode switching |
| token hint line | `/clear to save 255.1k tokens` | Token count changes every round |

**`※ recap:` is not noise** — it is the Recap block type (session compression summary, informative).

## Application Points

The three content entries are unified as `digest() → render()`:

| Call site | Chain |
|---|---|
| Hook (session_start/stop) | Raw archive terminal-content.jsonl → digest → render stored in Context + injected into Queue |
| Timer scan | Raw archive → digest → render → detect_change → inject into Queue only if Substantive |
| `fetch_terminal` tool | Raw archive → digest → render (full) returned to the LLM |

The digest itself is not persisted (rebuildable from the raw text; the view is volatile, docs/storage.md philosophy); Context stores the rendered text, log format unchanged.

## Test Fixtures

`core/tests/fixtures/`: `processing.txt` / `idle.txt` are **synthetic** samples (constructed from real noise patterns, containing no real user data) — real collected samples contain work content and are not checked in. Synthetic fixtures cover: wrapped-line joining, recap block, scheduled task, ToolCall fold rendering.
