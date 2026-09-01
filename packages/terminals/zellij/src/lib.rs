//! ambery-terminal-zellij：zellij 复用器的终端叶子。
//!
//! 进程内直调 `zellij action` CLI（CLI 自身即传输，无 sidecar 进程）：
//! enumerate 经 `list-panes -a --json`（pane 是读取单元，tab 只是容器），
//! read 经 `dump-screen -p <pane_id>` 读内容。

use ambery_terminal_lib::{ReadOutcome, TabInfo, TabRef, TerminalAdapter};
use std::collections::HashMap;
use std::sync::Arc;

/// ZellijAdapter：zellij 复用器经 `zellij action` CLI
/// 进程内直调，无独立进程。
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

#[cfg(test)]
mod tests {
    use super::*;

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
