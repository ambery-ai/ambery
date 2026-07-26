//! opencode 策略（docs/filter.md）：骨架——待真实 OpenCode 终端 UIA 样本调规则。
//! 噪声谱、块 glyph 表与 Claude Code 不同，无样本不臆造：
//! 当前只做 R0 trim_end + 空行剔除，digest 走默认实现（整篇 Info，不丢信息）。
//! TODO(样本)：noise 清单 / 块切分 / 折行合并参数（采集后比照 claude.rs 填）。

use super::{Change, Filter};

/// opencode 策略骨架
pub struct OpenCodeFilter {
    /// Jaccard 相似度阈值（与 claude 同款默认）
    pub similarity_threshold: f64,
}

impl Default for OpenCodeFilter {
    fn default() -> Self {
        Self {
            similarity_threshold: 0.8,
        }
    }
}

impl Filter for OpenCodeFilter {
    fn apply(&self, raw: &str) -> String {
        raw.lines()
            .map(str::trim_end)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn detect_change(&self, prev: &str, next: &str) -> Change {
        if prev == next {
            return Change::Unchanged;
        }
        let a: std::collections::HashSet<&str> = prev.lines().collect();
        let b: std::collections::HashSet<&str> = next.lines().collect();
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

    #[test]
    fn skeleton_passthrough_trims_only() {
        let f = OpenCodeFilter::default();
        let out = f.apply("  hello   \n\n  world  ");
        assert_eq!(out, "  hello\n  world");
    }
}
