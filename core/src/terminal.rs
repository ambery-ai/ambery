//! Terminal Adapter与 Platform Primitives
//! 概念的可实例化落地。
//!
//! TerminalAdapter 向 Code CLI 实例提供「定位、读取、遗忘」统一接口——一个实现
//! 对应一个终端类型（WtAdapter = wt 经 C# sidecar；MapAdapter = map 支撑终端，
//! case 剧情与 /debug/terminal 注入共用）。多终端兼容 = Composite 按 locate 首中分发。
//! PlatformPrimitives 是平台特定能力抽象组（虚拟桌面切换等 OS 层能力），
//! 被 Terminal Adapter（目标不可见时切桌面后读）与跳转功能共用。

use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// 实例在终端会话载体中的位置
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TabRef {
    pub hwnd: i64,
    pub index: i64,
}

/// Terminal Adapter 接口
pub trait TerminalAdapter: Send + Sync {
    /// 定位：instance → 它在终端会话中的位置
    fn locate(&self, inst: &str) -> Option<TabRef>;
    /// 读取：读该位置的终端文字
    fn read(&self, tab: &TabRef) -> Option<String>;
    /// 遗忘：定位缓存清除（instance 会话结束/判死）
    fn forget(&self, inst: &str);
}

/// Platform Primitives 接口
pub trait PlatformPrimitives: Send + Sync {
    /// 切到目标窗口所在虚拟桌面（读取前置 / 跳转共用）
    fn switch_vd(&self, hwnd: i64) -> bool;
}

/// WtAdapter：Windows Terminal 经 C# sidecar
/// 独立进程访问（UIA 定位 + TermControl TextPattern 读取，协议见）。
/// 定位缓存与 #10 hwnd 回收验证在本层（SidecarClient 是纯协议客户端）。
pub struct WtAdapter {
    sidecar: Arc<crate::sidecar::SidecarClient>,
    /// 定位缓存：实例名 → TabRef（forget / 自愈驱逐）
    cache: Mutex<HashMap<String, TabRef>>,
}

impl WtAdapter {
    pub fn new(sidecar: Arc<crate::sidecar::SidecarClient>) -> Self {
        Self {
            sidecar,
            cache: Mutex::new(HashMap::new()),
        }
    }

    fn find_tab(&self, inst: &str) -> Option<TabRef> {
        let r = self.sidecar.call(&json!({ "cmd": "find_tab", "name": inst }))?;
        Some(TabRef {
            hwnd: r["hwnd"].as_i64()?,
            index: r["index"].as_i64()?,
        })
    }
}

impl TerminalAdapter for WtAdapter {
    fn locate(&self, inst: &str) -> Option<TabRef> {
        if let Some(t) = self.cache.lock().ok()?.get(inst) {
            return Some(*t);
        }
        let tab = self.find_tab(inst)?;
        self.cache.lock().ok()?.insert(inst.to_string(), tab);
        Some(tab)
    }

    fn read(&self, tab: &TabRef) -> Option<String> {
        let text = self.sidecar.read_tab(tab.hwnd, tab.index)?;
        // #10 验证：hwnd 可能被回收——按缓存反查实例名，find_tab 复核 hwnd 一致
        let name = self
            .cache
            .lock()
            .ok()?
            .iter()
            .find(|(_, t)| t.hwnd == tab.hwnd)
            .map(|(n, _)| n.clone());
        let Some(name) = name else {
            return Some(text); // 无缓存映射（调用方直传 TabRef）：读到即返回
        };
        let verified = self
            .find_tab(&name)
            .map(|t| t.hwnd == tab.hwnd)
            .unwrap_or(false);
        if verified {
            return Some(text);
        }
        // 自愈：驱逐陈旧缓存 → 重找 → 重读并刷新缓存
        self.cache.lock().ok()?.remove(&name);
        let fresh = self.find_tab(&name)?;
        let text = self.sidecar.read_tab(fresh.hwnd, fresh.index)?;
        self.cache.lock().ok()?.insert(name, fresh);
        Some(text)
    }

    fn forget(&self, inst: &str) {
        if let Ok(mut c) = self.cache.lock() {
            c.remove(inst);
        }
    }
}

/// MapAdapter：共享 map 支撑的终端适配器。case-runner 的 terminal/terminal_gone
/// 剧情与 debug server 的 /debug/terminal 注入共用它当终端源。
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
    fn locate(&self, inst: &str) -> Option<TabRef> {
        if !self.contents.lock().ok()?.contains_key(inst) {
            return None;
        }
        let mut tabs = self.tabs.lock().ok()?;
        if let Some(t) = tabs.get(inst) {
            return Some(*t);
        }
        let mut next = self.next_hwnd.lock().ok()?;
        let tab = TabRef {
            hwnd: *next,
            index: 0,
        };
        *next -= 1;
        tabs.insert(inst.to_string(), tab);
        Some(tab)
    }

    fn read(&self, tab: &TabRef) -> Option<String> {
        let inst = self
            .tabs
            .lock()
            .ok()?
            .iter()
            .find(|(_, t)| t.hwnd == tab.hwnd)
            .map(|(n, _)| n.clone())?;
        self.contents.lock().ok()?.get(&inst).cloned()
    }

    fn forget(&self, inst: &str) {
        if let Ok(mut tabs) = self.tabs.lock() {
            tabs.remove(inst);
        }
    }
}

/// ZellijAdapter：zellij 复用器经 `zellij action` CLI
/// 进程内直调，无独立进程。locate 经 `list-panes -a --json` 匹配 marker（pane 是读取单元，
/// tab 只是容器），read 经 `dump-screen -p <pane_id>` 读内容；forget 清定位缓存。
///
/// 合成 hwnd 取 `-(100000 + pane_id)` 深负段——与 MapAdapter 的小负数段（-1..-N）隔离，
/// 避免 Composite::read 按 TabRef 精确路由时两个 adapter 产出相同 TabRef 产生歧义。
pub struct ZellijAdapter {
    runner: Arc<dyn ZellijRunner>,
    /// 定位缓存：实例名 → TabRef（forget 驱逐；read 直接用 TabRef.index 承载的 pane id，
    /// 不缓存反查——pane 关闭时 dump-screen 失败返回 None，交由调用方重定位）
    cache: Mutex<HashMap<String, TabRef>>,
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
        Self {
            runner,
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// 定位：list-panes 枚举 pane，TYPE=terminal（is_plugin=false）且 TITLE contains marker 者
    fn find_pane(&self, inst: &str) -> Option<TabRef> {
        let out = self.runner.run(&["action", "list-panes", "-a", "--json"])?;
        let panes: Vec<serde_json::Value> = serde_json::from_str(&out).ok()?;
        for p in panes {
            // 只认真实终端 pane，跳过 plugin pane（tab-bar / status-bar / zellij:link）
            if p["is_plugin"].as_bool() == Some(false) {
                let title = p["title"].as_str().unwrap_or("");
                // Contains 匹配（✳ 前缀与 | 描述后缀不影响命中）
                if title.contains(inst) {
                    let id = p["id"].as_i64()?;
                    return Some(TabRef {
                        hwnd: -(100_000 + id),
                        index: id,
                    });
                }
            }
        }
        None
    }
}

impl TerminalAdapter for ZellijAdapter {
    fn locate(&self, inst: &str) -> Option<TabRef> {
        if let Some(t) = self.cache.lock().ok()?.get(inst) {
            return Some(*t);
        }
        let tab = self.find_pane(inst)?;
        self.cache.lock().ok()?.insert(inst.to_string(), tab);
        Some(tab)
    }

    fn read(&self, tab: &TabRef) -> Option<String> {
        // dump-screen -p <pane_id>（裸数字等价 terminal_<id>，跨版本稳定）
        self.runner
            .run(&["action", "dump-screen", "-p", &tab.index.to_string()])
    }

    fn forget(&self, inst: &str) {
        if let Ok(mut c) = self.cache.lock() {
            c.remove(inst);
        }
    }
}

/// Composite：多 adapter 分发（「多终端兼容 = 抽象接口 +
/// 按终端分发实现」）。locate 首中者胜并记录路由；read 按路由精确回到同一 adapter
/// （TabRef 只在产出它的 adapter 上有意义）；forget 广播（各 adapter 缓存独立）。
pub struct Composite {
    adapters: Vec<Arc<dyn TerminalAdapter>>,
    /// locate 路由记录：实例名 →（adapter 序号，TabRef）
    routes: Mutex<HashMap<String, (usize, TabRef)>>,
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
    fn locate(&self, inst: &str) -> Option<TabRef> {
        for (i, a) in self.adapters.iter().enumerate() {
            if let Some(t) = a.locate(inst) {
                self.routes.lock().ok()?.insert(inst.to_string(), (i, t));
                return Some(t);
            }
        }
        None
    }

    fn read(&self, tab: &TabRef) -> Option<String> {
        let (i, _) = self
            .routes
            .lock()
            .ok()?
            .values()
            .find(|(_, t)| t == tab)
            .cloned()?;
        self.adapters.get(i)?.read(tab)
    }

    fn forget(&self, inst: &str) {
        if let Ok(mut r) = self.routes.lock() {
            r.remove(inst);
        }
        for a in &self.adapters {
            a.forget(inst);
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
    fn map_adapter_locate_read_forget() {
        let map = Arc::new(Mutex::new(HashMap::from([(
            "ft".to_string(),
            "终端内容".to_string(),
        )])));
        let a = MapAdapter::new(map.clone());
        // 未收录实例定位失败
        assert_eq!(a.locate("ghost"), None);
        // 收录实例：locate → read 配对
        let tab = a.locate("ft").expect("locate");
        assert!(tab.hwnd < 0, "合成 hwnd 取负数段");
        assert_eq!(a.read(&tab).as_deref(), Some("终端内容"));
        // 同实例再定位 = 同一 TabRef（缓存稳定）
        assert_eq!(a.locate("ft"), Some(tab));
        // forget 清定位缓存（内容不动，由 terminal_gone 剧情负责）
        a.forget("ft");
        let tab2 = a.locate("ft").expect("重新定位");
        assert_ne!(tab2, tab, "遗忘后重新分配");
        // 内容移除后定位失败
        map.lock().unwrap().remove("ft");
        assert_eq!(a.locate("ft"), None);
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
    fn zellij_adapter_locate_read_forget() {
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
        // 未收录实例定位失败
        assert_eq!(a.locate("ghost"), None);
        // 收录实例：跳过 plugin pane，Contains 匹配 marker → locate → read 配对
        let tab = a.locate("ft").expect("locate");
        assert!(tab.hwnd < -(100_000 + 2), "合成 hwnd 取深负段，与 MapAdapter 小负段隔离");
        assert_eq!(tab.index, 3, "index 承载真实 pane id");
        assert_eq!(a.read(&tab).as_deref(), Some("终端内容"));
        // 同实例再定位 = 同一 TabRef（缓存稳定）
        assert_eq!(a.locate("ft"), Some(tab));
        // forget 清定位缓存（重定位仍可读）
        a.forget("ft");
        let tab2 = a.locate("ft").expect("重新定位");
        assert_eq!(tab2, tab, "pane id 稳定，重定位回到同一 TabRef");
        assert_eq!(a.read(&tab2).as_deref(), Some("终端内容"));
    }

    #[test]
    fn composite_first_hit_routes_read_and_broadcasts_forget() {
        let empty = Arc::new(Mutex::new(HashMap::new()));
        let filled = Arc::new(Mutex::new(HashMap::from([(
            "ft".to_string(),
            "内容".to_string(),
        )])));
        let first = Arc::new(MapAdapter::new(empty));
        let second = Arc::new(MapAdapter::new(filled));
        let c = Composite::new(vec![first, second]);
        // 首中者胜：第一个 adapter 定不到 → 第二个命中
        let tab = c.locate("ft").expect("locate");
        assert_eq!(c.read(&tab).as_deref(), Some("内容"), "read 路由到产出 adapter");
        // forget 广播后重定位成功（缓存已清，重新分配）
        c.forget("ft");
        let tab2 = c.locate("ft").expect("重新定位");
        assert_ne!(tab, tab2);
        // 全 adapter 定不到 → None
        assert_eq!(c.locate("ghost"), None);
    }
}
