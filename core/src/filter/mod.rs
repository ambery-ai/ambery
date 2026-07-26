//! Filter（concepts §11，docs/filter.md）：终端文本结构理解。
//! 策略文件：claude.rs（Claude Code）；opencode.rs（骨架，待真实样本）。

pub mod claude;

use serde::{Deserialize, Serialize};

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

/// 按 Config.filter_strategy 选择实现（concepts §11 可替换策略）；
/// "default" 兼容映射 claude
pub fn by_name(name: &str) -> Box<dyn Filter + Send> {
    match name {
        "claude" | "default" => Box::new(claude::ClaudeFilter::default()),
        _ => Box::new(claude::ClaudeFilter::default()), // 未知策略回退 claude
    }
}
