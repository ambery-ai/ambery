//! Terminal Adapter 的 core 侧装配。
//!
//! 契约类型（TabRef / TabInfo / ReadOutcome / TerminalAdapter trait /
//! Composite / MapAdapter）由 ambery-terminal-lib 契约 crate 持有，本模块
//! 原样重导出，既有 `crate::terminal::X` 路径不受影响；叶子实现（zellij 等）
//! 在各自包，binary/config 层装配注入。本模块持有：消费方 join
//! （join_instance 种子 → 综合查询管线）、WtAdapter（wt 经 C# sidecar）、
//! PlatformPrimitives 平台能力抽象组（虚拟桌面切换等 OS 层能力，
//! 被跳转功能（切桌面后读）等消费方使用）。

use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;

pub use ambery_terminal_lib::{Composite, MapAdapter, ReadOutcome, TabInfo, TabRef, TerminalAdapter};

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

/// Platform Primitives 接口
pub trait PlatformPrimitives: Send + Sync {
    /// 切到目标窗口所在虚拟桌面（读取前置 / 跳转共用）
    fn switch_vd(&self, hwnd: i64) -> bool;
}
/// WtAdapter：Windows Terminal 经 C# sidecar
/// 独立进程访问（UIA 枚举 + TermControl TextPattern 读取）。
/// 纯 L1：不持实例缓存、不做 marker 匹配；tab 位置漂移的自愈与判死
/// 由消费方枚举对账承担。
pub struct WtAdapter {
    sidecar: Arc<crate::sidecar::SidecarClient>,
}

impl WtAdapter {
    pub fn new(sidecar: Arc<crate::sidecar::SidecarClient>) -> Self {
        Self { sidecar }
    }
}

impl TerminalAdapter for WtAdapter {
    /// list_wt_windows → 逐窗口 list_tabs；任一环节失败 = None（无观察）
    fn enumerate(&self) -> Option<Vec<TabInfo>> {
        let wins = self.sidecar.list_wt_windows()?;
        let mut out = Vec::new();
        for (hwnd, _) in wins {
            let tabs = self.sidecar.call(&json!({ "cmd": "list_tabs", "hwnd": hwnd }))?;
            for t in tabs["tabs"].as_array()? {
                out.push(TabInfo {
                    tab: TabRef { hwnd, index: t["index"].as_i64()? },
                    title: t["name"].as_str().map(String::from),
                    cwd: None,
                    command: None,
                    focused: t["selected"].as_bool(),
                    extras: HashMap::new(),
                });
            }
        }
        Some(out)
    }

    fn read(&self, tab: &TabRef) -> ReadOutcome {
        match self.sidecar.read_tab(tab.hwnd, tab.index) {
            Some(text) => ReadOutcome::Content(text),
            // WT tab 位置会漂移，纯 L1 无正向确证手段：失败一律 Error
            //（信念不动），判死由消费方枚举对账承担
            None => ReadOutcome::Error("read_tab failed".into()),
        }
    }
}

/// SidecarPlatformPrimitives：Windows 平台原语经 C# sidecar 进程交付
/// （COM IVirtualDesktopManager 在 sidecar 内）
pub struct SidecarPlatformPrimitives {
    sidecar: Arc<crate::sidecar::SidecarClient>,
}

impl SidecarPlatformPrimitives {
    pub fn new(sidecar: Arc<crate::sidecar::SidecarClient>) -> Self {
        Self { sidecar }
    }
}

impl PlatformPrimitives for SidecarPlatformPrimitives {
    fn switch_vd(&self, hwnd: i64) -> bool {
        self.sidecar
            .call(&json!({ "cmd": "switch_to_window_desktop", "hwnd": hwnd }))
            .and_then(|r| r["switched"].as_bool())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

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
