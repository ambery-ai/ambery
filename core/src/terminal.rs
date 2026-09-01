//! Terminal Adapter 的 core 侧出口。
//!
//! 契约类型（TabRef / TabInfo / ReadOutcome / TerminalAdapter / Composite /
//! MapAdapter / PlatformPrimitives）由 ambery-terminal-lib 契约 crate 持有，
//! 本模块原样重导出，既有 `crate::terminal::X` 路径不受影响；叶子实现
//! （wt / zellij / …）在各自包，由 binary/config 层装配注入。
//! 本模块持有消费方 join（join_instance 种子 → 综合查询管线）。

pub use ambery_terminal_lib::{
    Composite, MapAdapter, PlatformPrimitives, ReadOutcome, TabInfo, TabRef, TerminalAdapter,
};

/// 实例 → tab 的 join（消费方种子形态：enumerate 全量 + title 含 marker 的
/// 确凿单条件匹配，首中者胜；打分/歧义消解随综合查询管线扩展）。
/// None 不可区分「枚举失败」与「marker 缺席」——判死语义在消费方（枚举对账），
/// 本函数只做发现。
pub fn join_instance(terminal: &dyn TerminalAdapter, inst: &str) -> Option<TabRef> {
    terminal
        .enumerate()?
        .into_iter()
        .find(|t| {
            t.title
                .as_deref()
                .map(|s| s.contains(inst))
                .unwrap_or(false)
        })
        .map(|t| t.tab)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[test]
    fn join_instance_matches_title_contains() {
        let map = Arc::new(Mutex::new(HashMap::from([(
            "ft".to_string(),
            "内容".to_string(),
        )])));
        let a = MapAdapter::new(map);
        assert!(join_instance(&a, "ft").is_some());
        assert!(join_instance(&a, "ghost").is_none(), "marker 缺席");
    }
}
