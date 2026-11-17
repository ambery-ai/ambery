# Filter Design

English | [中文](filter.zh.md)

> - **Scope of this document** — Concept layer has no separate Filter entry — filtering is access-side detail. This document fixes the policy rules and the structure-understanding data types.
> - **Structure understanding** — the rules are taken from 3 real Claude Code terminal UIA samples (UIA returns rendered plain text, no ANSI codes).

## Structure Understanding Data Types

Each time filter processes terminal text it **fills a new data record field by field**; fields that filter cannot see or cannot judge confidently are left as `None` (never hard-filled).

```rust
pub struct TerminalDigest {
    pub blocks: Vec<ContentBlock>,
}

pub enum ContentBlock {
    /// user input (❯ head, may span wrapped lines)
    UserPrompt { text: String },
    /// assistant body text (● head, continuation lines are indented with no glyph, may span several lines)
    AssistantText { text: String },
    /// tool call: ● Tool(args) head + ⎿ result + full expanded body + source-native fold markers
    ToolCall {
        head: String,               // the whole ● Write(path) head line
        result: Option<String>,     // result summary after ⎿ (may be absent)
        body: String,               // full expanded body (diff / numbered content lines) — never omitted
        truncated: bool,            // source-native fold tail (… +N lines; the source already lost information, marked only)
    },
    /// session compaction summary (※ recap: head, multiple lines)
    Recap { text: String },
    /// system injection lines without a ● head (⎿ 3 skills available)
    SystemInject { text: String },
    /// neither a known block nor noise (fallback, loses no information)
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

## Policy Files (Replaceable Policies)

```
core/src/filter/
  mod.rs      # trait + TerminalDigest/ContentBlock + render + detect_change default impl + by_name
  claude.rs   # Claude Code rules
  opencode.rs # OpenCode policy (glyph table; unregistered = no effect)
```

trait:

```rust
pub trait Filter {
    /// denoise + normalize text (a policy must implement this)
    fn apply(&self, raw: &str) -> String;
    /// denoise + join wrapped lines + split blocks + fill TerminalDigest (default impl = apply + whole-text Info)
    fn digest(&self, raw: &str) -> TerminalDigest;
    /// change detection (operates on render text; the default impl is shareable)
    fn detect_change(&self, prev: &str, next: &str) -> Change;
}
```

Filter selects the per-instance hook `kind` ([docs/agents/claude/hook.md](claude/hook.md) §Payload) through the `by_name` registry — the seam where per-harness policies plug in. A harness strategy lives in its own module (claude.rs, opencode.rs, …) and takes effect by registration; an unregistered kind is unsupported and rejected directly before the instance state update, Terminal Content read, Filter, and Queue — no fallback, no partial support. The registry is the single admission point; the generic layer never hardcodes a harness.

## Boundaries

- **Filter ↔ TerminalAdapter** — the adapter reads raw text (locate + read); filter does structure understanding (digest + detect_change). One-way dependency adapter → filter. The adapter abstraction is not code-cli-specific: future terminals may serve purposes beyond a code CLI (see [docs/terminal/terminal-adapter.md](../terminal/terminal-adapter.md)).
- **Filter ↔ hook** — the hook manages lifecycle state (register / close / status); filter manages content understanding. Orthogonal, no overlap.
- **Hardcoded inside the seam** — per-harness rules are hardcoded in the policy file (claude.rs), never in the generic layer (mod.rs). Adding a harness = new module + register in `by_name`; core untouched.

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

The digest itself is not persisted (rebuildable from the raw text; the view is volatile, [docs/storage.md](../storage.md) philosophy); Context stores the rendered text, log format unchanged.

## Test Fixtures

`core/tests/fixtures/`: `processing.txt` / `idle.txt` are **synthetic** samples (constructed from real noise patterns, containing no real user data) — real collected samples contain work content and are not checked in. Synthetic fixtures cover: wrapped-line joining, recap block, scheduled task, ToolCall fold rendering.
