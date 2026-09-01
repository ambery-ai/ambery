//! ambery-terminal-lib：终端访问契约 crate（纯 L1）。
//!
//! TerminalAdapter 是纯 L1（enumerate + read）：只产载体数据，不识实例——
//! 实例 → tab 的 join 在消费方（core 侧）。Composite 聚合多 adapter 分发；
//! MapAdapter 是共享 map 支撑的测试桩（case-runner 的 terminal/terminal_gone
//! 剧情源）。叶子实现（wt / zellij / …）在各自包，只依赖本 crate。

use serde::{Deserialize, Serialize};
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

    /// 恒失败的 adapter（枚举失败注入）
    struct FailingAdapter;
    impl TerminalAdapter for FailingAdapter {
        fn enumerate(&self) -> Option<Vec<TabInfo>> {
            None
        }
        fn read(&self, _tab: &TabRef) -> ReadOutcome {
            ReadOutcome::Error("failing stub".into())
        }
    }

    #[test]
    fn composite_enumerate_fails_when_any_child_fails() {
        let ok = Arc::new(MapAdapter::new(Arc::new(Mutex::new(HashMap::new()))));
        let c = Composite::new(vec![ok, Arc::new(FailingAdapter)]);
        assert!(c.enumerate().is_none(), "任一子枚举失败 = 整体无观察");
    }
}
