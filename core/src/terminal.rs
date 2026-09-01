//! Terminal Adapter 的 core 侧装配。
//!
//! 契约类型（TabRef / TabInfo / ReadOutcome / TerminalAdapter trait /
//! Composite / MapAdapter）由 ambery-terminal-lib 契约 crate 持有，本模块
//! 原样重导出，既有 `crate::terminal::X` 路径不受影响。本模块持有：
//! 消费方 join（join_instance 种子 → 综合查询管线）、内置叶子实现
//! （WtAdapter = wt 经 C# sidecar；ZellijAdapter 经 CLI 进程内直调）、
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

/// ZellijAdapter：zellij 复用器经 `zellij action` CLI
/// 进程内直调，无独立进程。enumerate 经 `list-panes -a --json`（pane 是读取单元，
/// tab 只是容器），read 经 `dump-screen -p <pane_id>` 读内容。
///
/// 合成 hwnd 取 `-(100000 + pane_id)` 深负段——与 MapAdapter 的小负数段（-1..-N）隔离，
/// 避免 Composite::read 按 TabRef 精确路由时两个 adapter 产出相同 TabRef 产生歧义。
pub struct ZellijAdapter {
    runner: Arc<dyn ZellijRunner>,
}

/// zellij CLI 执行器（系统边界，可注入 mock）
pub trait ZellijRunner: Send + Sync {
    fn run(&self, args: &[&str]) -> Option<String>;
}

/// 生产实现：直调 `zellij` 进程（进程内 Rust 直调 CLI）
pub struct ProcessZellijRunner;

impl ZellijRunner for ProcessZellijRunner {
    fn run(&self, args: &[&str]) -> Option<String> {
        let out = std::process::Command::new("zellij")
            .args(args)
            .output()
            .ok()?;
        if out.status.success() {
            Some(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            None
        }
    }
}

impl ZellijAdapter {
    pub fn new(runner: Arc<dyn ZellijRunner>) -> Self {
        Self { runner }
    }
}

impl TerminalAdapter for ZellijAdapter {
    /// list-panes 枚举 pane，只认真实终端 pane（is_plugin=false；
    /// tab-bar / status-bar / zellij:link 跳过）
    fn enumerate(&self) -> Option<Vec<TabInfo>> {
        let out = self.runner.run(&["action", "list-panes", "-a", "--json"])?;
        let panes: Vec<serde_json::Value> = serde_json::from_str(&out).ok()?;
        let mut infos = Vec::new();
        for p in panes {
            if p["is_plugin"].as_bool() == Some(false) {
                let id = p["id"].as_i64()?;
                infos.push(TabInfo {
                    tab: TabRef {
                        hwnd: -(100_000 + id),
                        index: id,
                    },
                    title: p["title"].as_str().map(String::from),
                    cwd: None,
                    command: None,
                    focused: p["focused"].as_bool().or_else(|| p["is_focused"].as_bool()),
                    extras: HashMap::new(),
                });
            }
        }
        Some(infos)
    }

    fn read(&self, tab: &TabRef) -> ReadOutcome {
        // dump-screen -p <pane_id>（裸数字等价 terminal_<id>，跨版本稳定）
        if let Some(text) = self
            .runner
            .run(&["action", "dump-screen", "-p", &tab.index.to_string()])
        {
            return ReadOutcome::Content(text);
        }
        // 确证复核：list-panes 查无此 pane id → Gone（pane id 稳定，纯位置判定）；
        // 复核本身失败或 pane 仍在 → Error（信念不动）
        match self.runner.run(&["action", "list-panes", "-a", "--json"]) {
            Some(out) => {
                let absent = serde_json::from_str::<Vec<serde_json::Value>>(&out)
                    .map(|panes| panes.iter().all(|p| p["id"].as_i64() != Some(tab.index)))
                    .unwrap_or(false);
                if absent {
                    ReadOutcome::Gone
                } else {
                    ReadOutcome::Error("dump-screen failed but pane still listed".into())
                }
            }
            None => ReadOutcome::Error("dump-screen and list-panes both failed".into()),
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

    /// Stub zellij CLI runner（系统边界 mock，仅在系统边界打桩）。
    /// 复刻真实命令形态：list-panes -a --json（数组）/ dump-screen -p <id>。
    struct StubZellij {
        /// (pane_id, title, is_plugin)
        panes: Vec<(i64, String, bool)>,
        /// pane_id → 内容
        contents: HashMap<i64, String>,
    }

    impl ZellijRunner for StubZellij {
        fn run(&self, args: &[&str]) -> Option<String> {
            match args {
                ["action", "list-panes", "-a", "--json"] => {
                    let arr: Vec<serde_json::Value> = self
                        .panes
                        .iter()
                        .map(|(id, title, is_plugin)| {
                            serde_json::json!({
                                "id": id,
                                "title": title,
                                "is_plugin": is_plugin,
                            })
                        })
                        .collect();
                    Some(serde_json::to_string(&arr).unwrap())
                }
                ["action", "dump-screen", "-p", id] => {
                    let id: i64 = id.parse().ok()?;
                    self.contents.get(&id).cloned()
                }
                _ => None,
            }
        }
    }

    #[test]
    fn zellij_adapter_enumerate_and_read() {
        // zellij pane title 带 marker 前缀与描述后缀；
        // plugin pane（tab-bar）混入以验证 TYPE 过滤
        let z = Arc::new(StubZellij {
            panes: vec![
                (0, "(.) - zellij:link".into(), true),
                (3, "✳ ft | 收尾中".into(), false),
            ],
            contents: HashMap::from([(3, "终端内容".to_string())]),
        });
        let a = ZellijAdapter::new(z);
        let infos = a.enumerate().expect("enumerate");
        // 跳过 plugin pane，只认真实终端 pane
        assert_eq!(infos.len(), 1);
        let tab = infos[0].tab;
        assert_eq!(infos[0].title.as_deref(), Some("✳ ft | 收尾中"));
        assert!(tab.hwnd < -(100_000 + 2), "合成 hwnd 取深负段，与 MapAdapter 小负段隔离");
        assert_eq!(tab.index, 3, "index 承载真实 pane id");
        assert!(matches!(a.read(&tab), ReadOutcome::Content(ref s) if s == "终端内容"));
    }

    #[test]
    fn zellij_adapter_read_gone_when_pane_absent() {
        // pane 既不在 list-panes 也 dump 不到 = 纯位置确证消失 → Gone
        let z = Arc::new(StubZellij {
            panes: vec![],
            contents: HashMap::new(),
        });
        let a = ZellijAdapter::new(z);
        let tab = TabRef {
            hwnd: -(100_000 + 3),
            index: 3,
        };
        assert!(matches!(a.read(&tab), ReadOutcome::Gone));
    }
}
