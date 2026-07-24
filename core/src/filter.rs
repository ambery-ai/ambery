//! Filter（concepts §11，docs/filter.md）：去噪 + 归一 + 变化检测。
//! 规则取自真实 Claude Code 标签页的 UIA 文本样本（UIA 返回渲染后纯文本，无 ANSI 码）。

use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Change {
    Unchanged,
    /// 相似度（spinner 残留、计数器微变级别）
    Minor(f64),
    /// 相似度（实质内容变化，值得通知）
    Substantive(f64),
}

pub trait Filter {
    /// 去噪 + 归一
    fn apply(&self, raw: &str) -> String;
    /// 变化检测（作用于归一后文本）
    fn detect_change(&self, prev: &str, next: &str) -> Change;
}

/// default 策略（docs/filter.md §规则）
pub struct DefaultFilter {
    /// Jaccard 相似度阈值，≥ 判 Minor
    pub similarity_threshold: f64,
}

impl Default for DefaultFilter {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.8,
        }
    }
}

static SPINNER_TIME: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*[✻✽✶✢✳]?\s*(Thought|Searched|Crunched|Brewed|Cooked|Thinking|Working|Reading|Running|Searching)\b.*\bfor \d+(\.\d+)?(s| (pattern|files?|director\w+|shell commands?|matches))\b",
    )
    .unwrap()
});
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
        || is_separator(line)
        || PERMISSION_HINT.is_match(line)
        || GIT_STATUS.is_match(line)
        || MODEL_FEE.is_match(line)
        || EMPTY_PROMPT.is_match(line)
        || TOKEN_HINT.is_match(line)
}

impl Filter for DefaultFilter {
    fn apply(&self, raw: &str) -> String {
        raw.lines()
            .map(|l| GIT_STATUS_SUFFIX.replace(l, "").into_owned()) // 剥离粘附的右对齐状态后缀
            .map(|l| str::trim_end(l.as_str()).to_string()) // R0：去 UIA 网格右填充
            .filter(|l| !l.is_empty() && !is_noise(l))
            .collect::<Vec<_>>()
            .join("\n")
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

/// 按 Config.filter_strategy 选择实现（concepts §11 可替换策略）
pub fn by_name(name: &str) -> Box<dyn Filter + Send> {
    match name {
        "default" => Box::new(DefaultFilter::default()),
        _ => Box::new(DefaultFilter::default()), // 未知策略回退 default
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

    const PROCESSING: &str = include_str!("../tests/fixtures/processing.txt");
    const IDLE: &str = include_str!("../tests/fixtures/idle.txt");

    fn filter() -> DefaultFilter {
        DefaultFilter::default()
    }

    #[test]
    fn processing_denoise_keeps_only_content() {
        let out = filter().apply(&pad(PROCESSING));
        assert_eq!(
            out,
            "● Update(settings.json)\n  ⎿  Added 3 lines\n      13 +  \"hooks\": {\n      14 +    \"SessionStart\": []\n      15 +  }"
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
}
