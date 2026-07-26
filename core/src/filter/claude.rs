//! claude 策略（docs/filter.md）：Claude Code 终端的结构理解。
//! 规则取自真实 Claude Code 标签页的 UIA 文本样本（UIA 返回渲染后纯文本，无 ANSI 码）。
//! 管线：trim_end → 折行合并 → 去噪 → 块切分填 TerminalDigest。

use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

use super::{Change, ContentBlock, Filter, TerminalDigest};

/// claude 策略（docs/filter.md §claude.rs 噪音清单）
pub struct ClaudeFilter {
    /// Jaccard 相似度阈值，≥ 判 Minor
    pub similarity_threshold: f64,
}

impl Default for ClaudeFilter {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.8,
        }
    }
}

/// 耗时行：动词一般过去式泛化（v2 样本抓到 Baked/Churned/Worked 漏网）+ 无 ✻ 的 Thought 系
static SPINNER_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*[✻✽✶✢✳]?\s*([A-Z][a-z]+ed|Thought|Thinking|Working|Reading|Running|Searching)\b.*\bfor \d+(\.\d+)?(s|m| (pattern|files?|director\w+|shell commands?|matches))\b",
    )
    .unwrap()
});
/// 计划任务行（v2 新增：无 "for Xs"，v1 漏网）
static SCHEDULED_TASK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[✻✽✶✢✳]\s*Running scheduled task\b").unwrap());
static PERMISSION_HINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*⏵⏵").unwrap());
static GIT_STATUS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*●*\s*on\s+\S+\s*$").unwrap());
static MODEL_FEE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\S+(-\S+)+ {2,}\$\d+(\.\d+)?\s*$").unwrap());
static EMPTY_PROMPT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*❯\s*$").unwrap());
static TOKEN_HINT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"/clear to save [\d.]+k tokens").unwrap());
/// 右对齐状态片段会粘在同一 UIA 行内容文本之后（2+ 空格分隔），剥掉后缀
static GIT_STATUS_SUFFIX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s{2,}●+\s*on\s+\S+$").unwrap());
/// diff 行（Write/Edit 展开体）：行号 + +/- 前缀（`      594 +| ...`）
static DIFF_ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*\d+\s*[+-]\|?").unwrap());
/// Write 展开的编号内容行（`      1 # Issues`）
static NUMBERED_ROW: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s{4,}\d+\s").unwrap());
/// 内置折叠尾（`… +5 lines`）
static FOLD_TAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*…\s*\+\d+ lines?\b").unwrap());

/// R1：braille spinner 字符（U+2800–U+28FF）
fn is_braille(line: &str) -> bool {
    line.chars().any(|c| ('\u{2800}'..='\u{28ff}').contains(&c))
}

/// R3：分隔线行（去空格后 ─ 占比 ≥ 60% 且长度 ≥ 20，允许中间夹实例名）
fn is_separator(line: &str) -> bool {
    let compact: Vec<char> = line.chars().filter(|c| !c.is_whitespace()).collect();
    if compact.len() < 20 {
        return false;
    }
    let dashes = compact.iter().filter(|&&c| c == '─').count();
    dashes * 5 >= compact.len() * 3
}

fn is_noise(line: &str) -> bool {
    is_braille(line)
        || SPINNER_TIME.is_match(line)
        || SCHEDULED_TASK.is_match(line)
        || is_separator(line)
        || PERMISSION_HINT.is_match(line)
        || GIT_STATUS.is_match(line)
        || MODEL_FEE.is_match(line)
        || EMPTY_PROMPT.is_match(line)
        || TOKEN_HINT.is_match(line)
}

/// 块头 glyph（这些行永远开新逻辑行/新块，不参与折行合并）
fn is_glyph_head(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with(['❯', '●', '⎿', '※', '✻', '✽', '✶', '✢', '✳'])
}

/// 折行合并（UIA wrap）：物理行 → 逻辑行。
/// 终端宽度把长行硬折成多行；续行拼回上一行——但仅当上一行填满了网格宽（≥90%）
/// 才判硬折（短行结尾 = 有意换行，保留为独立逻辑行）。
/// width = UIA 网格宽（未 trim 的行宽，右填充如实告知宽度）
fn unwrap(width: usize, rows: &[&str]) -> Vec<String> {
    let threshold = ((width as f64) * 0.9) as usize;
    let mut out: Vec<String> = Vec::new();
    for r in rows {
        let starts_new = is_glyph_head(r) || DIFF_ROW.is_match(r) || NUMBERED_ROW.is_match(r);
        if starts_new || out.is_empty() {
            out.push((*r).to_string());
            continue;
        }
        let prev_full = out.last().map(|p| p.chars().count() >= threshold).unwrap_or(false);
        if prev_full {
            out.last_mut().unwrap().push_str(r.trim_start());
        } else {
            out.push((*r).to_string());
        }
    }
    out
}

/// 块切分：glyph 头开新块；ToolCall = ● 头 + ⎿ 结果 + 展开体行数 + 折叠尾
fn blockify(rows: &[String]) -> Vec<ContentBlock> {
    let mut blocks = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        let t = rows[i].trim_start();
        if let Some(text) = t.strip_prefix('❯') {
            let mut text = text.trim().to_string();
            i += 1;
            while i < rows.len() && !is_glyph_head(&rows[i]) {
                text.push('\n');
                text.push_str(&rows[i]);
                i += 1;
            }
            blocks.push(ContentBlock::UserPrompt { text });
        } else if let Some(text) = t.strip_prefix('※') {
            let mut text = text.trim().to_string();
            i += 1;
            while i < rows.len() && !is_glyph_head(&rows[i]) {
                text.push('\n');
                text.push_str(&rows[i]);
                i += 1;
            }
            blocks.push(ContentBlock::Recap { text });
        } else if let Some(text) = t.strip_prefix('⎿') {
            blocks.push(ContentBlock::SystemInject {
                text: text.trim().to_string(),
            });
            i += 1;
        } else if let Some(head) = t.strip_prefix('●') {
            let head = format!("● {}", head.trim());
            // ToolCall：下一逻辑行是 ⎿ 结果；否则助手正文
            if i + 1 < rows.len() && rows[i + 1].trim_start().starts_with('⎿') {
                let result = rows[i + 1]
                    .trim_start()
                    .strip_prefix('⎿')
                    .unwrap()
                    .trim()
                    .to_string();
                let mut j = i + 2;
                let mut body_lines = 0;
                let mut truncated = false;
                while j < rows.len() && !is_glyph_head(&rows[j]) {
                    if FOLD_TAIL.is_match(&rows[j]) {
                        truncated = true;
                    } else {
                        body_lines += 1;
                    }
                    j += 1;
                }
                blocks.push(ContentBlock::ToolCall {
                    head,
                    result: Some(result),
                    body_lines,
                    truncated,
                });
                i = j;
            } else {
                let mut text = head.trim_start_matches('●').trim().to_string();
                i += 1;
                while i < rows.len() && !is_glyph_head(&rows[i]) {
                    text.push('\n');
                    text.push_str(&rows[i]);
                    i += 1;
                }
                blocks.push(ContentBlock::AssistantText { text });
            }
        } else {
            blocks.push(ContentBlock::Info {
                text: rows[i].clone(),
            });
            i += 1;
        }
    }
    blocks
}

impl Filter for ClaudeFilter {
    fn apply(&self, raw: &str) -> String {
        self.digest(raw).render()
    }

    fn digest(&self, raw: &str) -> TerminalDigest {
        // R0 + 后缀剥离（物理行级）
        let physical: Vec<String> = raw
            .lines()
            .map(|l| GIT_STATUS_SUFFIX.replace(l, "").into_owned())
            .map(|l| l.trim_end().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        // 网格宽：未 trim 的行宽（右填充如实反映终端宽度）
        let width = raw.lines().map(|l| l.chars().count()).max().unwrap_or(1);
        // 折行合并 → 去噪 → 块切分
        let refs: Vec<&str> = physical.iter().map(String::as_str).collect();
        let logical = unwrap(width, &refs);
        let clean: Vec<String> = logical.into_iter().filter(|l| !is_noise(l)).collect();
        TerminalDigest {
            blocks: blockify(&clean),
        }
    }

    fn detect_change(&self, prev: &str, next: &str) -> Change {
        if prev == next {
            return Change::Unchanged;
        }
        let a: HashSet<&str> = prev.lines().collect();
        let b: HashSet<&str> = next.lines().collect();
        let inter = a.intersection(&b).count() as f64;
        let union = a.union(&b).count() as f64;
        let sim = if union == 0.0 { 1.0 } else { inter / union };
        if sim >= self.similarity_threshold {
            Change::Minor(sim)
        } else {
            Change::Substantive(sim)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 模拟 UIA 网格：每行右填充到 80 列
    fn pad(raw: &str) -> String {
        raw.lines()
            .map(|l| format!("{l:<80}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    const PROCESSING: &str = include_str!("../../tests/fixtures/processing.txt");
    const IDLE: &str = include_str!("../../tests/fixtures/idle.txt");

    fn filter() -> ClaudeFilter {
        ClaudeFilter::default()
    }

    #[test]
    fn processing_toolcall_folded() {
        let d = filter().digest(&pad(PROCESSING));
        assert_eq!(d.blocks.len(), 1);
        match &d.blocks[0] {
            ContentBlock::ToolCall {
                head,
                result,
                body_lines,
                truncated,
            } => {
                assert_eq!(head, "● Update(settings.json)");
                assert_eq!(result.as_deref(), Some("Added 3 lines"));
                assert_eq!(*body_lines, 3);
                assert!(!truncated);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        assert_eq!(
            d.render(),
            "● Update(settings.json)\n  ⎿  Added 3 lines\n  … (3 行)"
        );
    }

    #[test]
    fn idle_keeps_user_prompt_and_reply() {
        let out = filter().apply(&pad(IDLE));
        assert!(out.contains("❯ 帮我确认一下端口")); // 有文字的 prompt = 用户输入，保留
        assert!(out.contains("● 完成。hooks 已配置："));
        assert!(!out.contains("❯\n")); // 空 prompt 已去
        assert!(!out.contains('$')); // 费用行已去
        assert!(!out.contains("⏵⏵"));
        assert!(!out.lines().any(|l| l.ends_with(' ')));
    }

    #[test]
    fn idle_block_types() {
        let d = filter().digest(&pad(IDLE));
        assert!(d
            .blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::UserPrompt { text } if text.contains("帮我确认一下端口"))));
        assert!(d
            .blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::AssistantText { text } if text.contains("完成。hooks 已配置"))));
    }

    #[test]
    fn change_processing_to_idle_is_substantive() {
        let f = filter();
        let prev = f.apply(&pad(PROCESSING));
        let next = f.apply(&pad(IDLE));
        match f.detect_change(&prev, &next) {
            Change::Substantive(sim) => assert!(sim < 0.8 && sim > 0.0),
            other => panic!("expected Substantive, got {other:?}"),
        }
    }

    #[test]
    fn change_fee_only_is_unchanged() {
        let f = filter();
        let a = f.apply(&pad(IDLE));
        // 费用变化：$12.99 → $13.40（噪音行已被过滤，归一后无差异）
        let b = f.apply(&pad(&IDLE.replace("$12.99", "$13.40")));
        assert_eq!(f.detect_change(&a, &b), Change::Unchanged);
    }

    #[test]
    fn change_spinner_frames_is_unchanged() {
        let f = filter();
        let a = f.apply(&pad(PROCESSING));
        let b = f.apply(&pad(&PROCESSING.replace("Crunched for 12s", "Crunched for 13s")));
        assert_eq!(f.detect_change(&a, &b), Change::Unchanged);
    }

    #[test]
    fn git_status_suffix_stripped_from_content_line() {
        // 右对齐 git 状态粘附在内容行尾（真实样本观察）
        let raw = "  继续讨论还是进 spec？  ●● on  master";
        let out = filter().apply(raw);
        assert_eq!(out, "  继续讨论还是进 spec？");
    }

    #[test]
    fn change_one_line_of_many_is_minor() {
        let f = filter();
        let a = (1..=10)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let b = a.replacen("line 10", "line ten", 1);
        match f.detect_change(&a, &b) {
            Change::Minor(sim) => assert!(sim >= 0.8),
            other => panic!("expected Minor, got {other:?}"),
        }
    }

    // ── v2 新增（样本驱动） ──

    #[test]
    fn spinner_past_tense_verbs_are_noise() {
        // v1 漏网：Baked/Churned/Worked 不在动词清单（真实样本形态）
        let raw = "✻ Baked for 3m 40s\n✻ Churned for 19s\n✻ Worked for 42s\n● 正文";
        let out = filter().apply(raw);
        assert_eq!(out, "● 正文");
    }

    #[test]
    fn scheduled_task_is_noise() {
        // v1 漏网：无 "for Xs" 的计划任务行（真实样本形态）
        let raw = "✻ Running scheduled task (Jul 26 8:30pm)\n● 正文";
        let out = filter().apply(raw);
        assert_eq!(out, "● 正文");
    }

    #[test]
    fn recap_is_block_not_noise() {
        let raw = "※ recap: NapSrc 日 cron 已运行\n  刚登记了 issue #1\n● 正文";
        let d = filter().digest(raw);
        assert!(d.blocks.iter().any(
            |b| matches!(b, ContentBlock::Recap { text } if text.contains("NapSrc 日 cron") && text.contains("issue #1"))
        ));
    }

    #[test]
    fn system_inject_block() {
        let raw = "❯ 先读 usergoals\n  ⎿  3 skills available";
        let d = filter().digest(raw);
        assert!(d
            .blocks
            .iter()
            .any(|b| matches!(b, ContentBlock::SystemInject { text } if text.contains("3 skills"))));
    }

    #[test]
    fn unwrap_glues_hard_wrapped_row() {
        // 40 列网格：逻辑行 52 字被硬折成两段（第一段 39 字 ≥ 阈值 → 续行拼回）
        let row1 = format!("{:<40}", format!("❯ {}", "长".repeat(37)));
        let row2 = format!("{:<40}", "  尾段");
        let raw = [row1, row2].join("\n");
        let d = filter().digest(&raw);
        match &d.blocks[0] {
            ContentBlock::UserPrompt { text } => {
                assert_eq!(text, &format!("{}尾段", "长".repeat(37)));
            }
            other => panic!("expected UserPrompt, got {other:?}"),
        }
    }

    #[test]
    fn short_rows_not_glued() {
        // 短行结尾 = 有意换行，不合并（win2 助手多行回复形态，80 列网格）
        let raw = pad("● 没问题. 三个词都是名词修饰关系.\n  意思是\"沙盒安全登记表\".");
        let d = filter().digest(&raw);
        match &d.blocks[0] {
            ContentBlock::AssistantText { text } => {
                assert!(text.contains('\n'), "短行应保留为独立行: {text}");
            }
            other => panic!("expected AssistantText, got {other:?}"),
        }
    }

    #[test]
    fn toolcall_with_fold_tail() {
        let raw = "● Write(docs/issues.md)\n  ⎿  Wrote 11 lines\n      1 # Issues\n      2 \n      3 ***\n  … +5 lines\n● 后续";
        let d = filter().digest(raw);
        match &d.blocks[0] {
            ContentBlock::ToolCall {
                body_lines,
                truncated,
                ..
            } => {
                assert_eq!(*body_lines, 3);
                assert!(*truncated);
            }
            other => panic!("expected ToolCall, got {other:?}"),
        }
        assert!(d.render().contains("(3 行, 原文有折叠)"));
    }
}
