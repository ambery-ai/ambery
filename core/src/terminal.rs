//! Terminal Adapter与 Platform Primitives
//! 概念的可实例化落地。
//!
//! TerminalAdapter 是纯 L1（enumerate + read）：只产载体数据，不识实例——
//! 实例 → tab 的 join 在消费方（join_instance 种子 → 综合查询管线）。一个实现
//! 对应一个终端类型（WtAdapter = wt 经 C# sidecar；MapAdapter = map 支撑终端，
//! case-runner 的 terminal/terminal_gone 剧情源）。多终端兼容 = Composite 聚合分发。
//! PlatformPrimitives 是平台特定能力抽象组（虚拟桌面切换等 OS 层能力），
//! 被跳转功能（切桌面后读）等消费方使用。

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 实例在终端会话载体中的位置
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TabRef {
    pub hwnd: i64,
    pub index: i64,
}

/// 枚举产出的载体属性（L1 → 上层的纯数据；字段全 Option，缺键优雅降级）
#[derive(Debug, Clone)]
pub struct TabInfo {
    pub tab: TabRef,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub command: Option<String>,
    pub focused: Option<bool>,
    pub extras: HashMap<String, String>,
}

/// 读取的三态观察（信念检查载体）：Content = 证据存活 / Gone = 正向确证该
/// tab 已不存在（死亡强证据）/ Error = 瞬时失败，无观察、信念不动，永不致死。
/// 契约：Gone 只在 adapter 内部正向确证后返回（Zellij = list-panes 无此 pane
/// id；Map = 内容表已移除；Wt 的 tab 位置会漂移、无位置级确证手段，恒不产
/// Gone——其判死由消费方枚举对账承担）；不确定一律 Error。复核责任在
/// adapter，消费方零复核。
#[derive(Debug)]
pub enum ReadOutcome {
    Content(String),
    Gone,
    Error(String),
}

/// Terminal Adapter 接口（纯 L1：只产载体数据，不识实例）
pub trait TerminalAdapter: Send + Sync {
    /// 枚举：遍历该终端的 tab/pane，返回载体属性。
    /// None = 枚举失败（无观察；判死不得以此为据——「空」与「失败」必须可分）
    fn enumerate(&self) -> Option<Vec<TabInfo>>;
    /// 读取：读该位置的终端文字，三态观察（契约见 ReadOutcome）
    fn read(&self, tab: &TabRef) -> ReadOutcome;
}

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

/// MapAdapter：共享 map 支撑的终端适配器。case-runner 的 terminal/terminal_gone
/// 剧情用它当终端源。
/// 合成 hwnd 取负数段，与真实 hwnd 域隔离。
pub struct MapAdapter {
    contents: Arc<Mutex<HashMap<String, String>>>,
    tabs: Mutex<HashMap<String, TabRef>>,
    next_hwnd: Mutex<i64>,
}

impl MapAdapter {
    pub fn new(contents: Arc<Mutex<HashMap<String, String>>>) -> Self {
        Self {
            contents,
            tabs: Mutex::new(HashMap::new()),
            next_hwnd: Mutex::new(-1),
        }
    }
}

impl TerminalAdapter for MapAdapter {
    /// contents 键即 tab（共享 map 的载体模型），title = 键名；
    /// 未分配过的键现分配 TabRef（tabs 表持久，同键稳定回到同一 TabRef）
    fn enumerate(&self) -> Option<Vec<TabInfo>> {
        let contents = self.contents.lock().ok()?;
        let mut tabs = self.tabs.lock().ok()?;
        let mut next = self.next_hwnd.lock().ok()?;
        let mut out = Vec::new();
        for name in contents.keys() {
            let tab = *tabs.entry(name.clone()).or_insert_with(|| {
                let t = TabRef { hwnd: *next, index: 0 };
                *next -= 1;
                t
            });
            out.push(TabInfo {
                tab,
                title: Some(name.clone()),
                cwd: None,
                command: None,
                focused: None,
                extras: HashMap::new(),
            });
        }
        Some(out)
    }

    fn read(&self, tab: &TabRef) -> ReadOutcome {
        // 反查失败（未知 tab）或内容已移除 = 载体确证消失 → Gone
        //（Map 无传输层，无 Error 态）
        let inst = self.tabs.lock().ok().and_then(|tabs| {
            tabs.iter()
                .find(|(_, t)| t.hwnd == tab.hwnd)
                .map(|(n, _)| n.clone())
        });
        let Some(inst) = inst else {
            return ReadOutcome::Gone;
        };
        match self.contents.lock().ok().and_then(|c| c.get(&inst).cloned()) {
            Some(text) => ReadOutcome::Content(text),
            None => ReadOutcome::Gone,
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

/// Composite：多 adapter 分发（「多终端兼容 = 抽象接口 +
/// 按终端分发实现」）。enumerate 聚合全部子 adapter 并重建路由
/// （TabRef 只在产出它的 adapter 上有意义）；任一子枚举失败 = 整体 None
/// （部分枚举不可作判死依据）。read 按路由精确回到同一 adapter。
pub struct Composite {
    adapters: Vec<Arc<dyn TerminalAdapter>>,
    /// 枚举路由记录：TabRef → adapter 序号（每次 enumerate 重建）
    routes: Mutex<HashMap<TabRef, usize>>,
}

impl Composite {
    pub fn new(adapters: Vec<Arc<dyn TerminalAdapter>>) -> Self {
        Self {
            adapters,
            routes: Mutex::new(HashMap::new()),
        }
    }
}

impl TerminalAdapter for Composite {
    fn enumerate(&self) -> Option<Vec<TabInfo>> {
        let mut routes = self.routes.lock().ok()?;
        routes.clear();
        let mut out = Vec::new();
        for (i, a) in self.adapters.iter().enumerate() {
            for info in a.enumerate()? {
                routes.insert(info.tab, i);
                out.push(info);
            }
        }
        Some(out)
    }

    fn read(&self, tab: &TabRef) -> ReadOutcome {
        let route = self.routes.lock().ok().and_then(|r| r.get(tab).copied());
        match route {
            Some(i) => match self.adapters.get(i) {
                Some(a) => a.read(tab),
                None => ReadOutcome::Error("route adapter missing".into()),
            },
            None => ReadOutcome::Error("no route for tab".into()),
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

    #[test]
    fn map_adapter_enumerate_and_read() {
        let map = Arc::new(Mutex::new(HashMap::from([(
            "ft".to_string(),
            "终端内容".to_string(),
        )])));
        let a = MapAdapter::new(map.clone());
        // enumerate：contents 键即 tab，title = 键名；合成 hwnd 取负数段
        let infos = a.enumerate().expect("enumerate");
        assert_eq!(infos.len(), 1);
        let tab = infos[0].tab;
        assert_eq!(infos[0].title.as_deref(), Some("ft"));
        assert!(tab.hwnd < 0, "合成 hwnd 取负数段");
        assert!(matches!(a.read(&tab), ReadOutcome::Content(ref s) if s == "终端内容"));
        // 同键再枚举 = 同一 TabRef（tabs 表持久）
        assert_eq!(a.enumerate().expect("再枚举")[0].tab, tab);
        // 内容移除：枚举不再含此 tab；按旧 TabRef 读 = Gone（确证消失）
        map.lock().unwrap().remove("ft");
        assert!(a.enumerate().expect("移除后枚举").is_empty());
        assert!(matches!(a.read(&tab), ReadOutcome::Gone));
    }

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

    #[test]
    fn composite_enumerate_aggregates_and_routes_read() {
        let empty = Arc::new(Mutex::new(HashMap::new()));
        let filled = Arc::new(Mutex::new(HashMap::from([(
            "ft".to_string(),
            "内容".to_string(),
        )])));
        let first = Arc::new(MapAdapter::new(empty));
        let second = Arc::new(MapAdapter::new(filled));
        let c = Composite::new(vec![first, second]);
        // enumerate 聚合全部子 adapter 并记录路由
        let infos = c.enumerate().expect("enumerate");
        assert_eq!(infos.len(), 1);
        let tab = infos[0].tab;
        assert!(matches!(c.read(&tab), ReadOutcome::Content(ref s) if s == "内容"), "read 路由到产出 adapter");
        // 未枚举过的 TabRef 无路由
        let foreign = TabRef { hwnd: -999, index: 0 };
        assert!(matches!(c.read(&foreign), ReadOutcome::Error(_)), "无路由 = Error");
    }

    /// 恒失败的 zellij runner（枚举失败注入）
    struct FailingRunner;
    impl ZellijRunner for FailingRunner {
        fn run(&self, _args: &[&str]) -> Option<String> {
            None
        }
    }

    #[test]
    fn composite_enumerate_fails_when_any_child_fails() {
        let ok = Arc::new(MapAdapter::new(Arc::new(Mutex::new(HashMap::new()))));
        let failing = Arc::new(ZellijAdapter::new(Arc::new(FailingRunner)));
        let c = Composite::new(vec![ok, failing]);
        assert!(c.enumerate().is_none(), "任一子枚举失败 = 整体无观察");
    }
}
