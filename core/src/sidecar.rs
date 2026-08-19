//! SidecarClient：stdio JSON Lines 调 C# UIA sidecar。
//! Mutex 串行化请求（UIA 切 Tab 是全局状态，不可并行）。
//! 常驻简化语义：进程死了即弃，下次请求现拉起（冷启 ~200ms）——无保活预检、无心跳。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// tab 切换限流：全局 5 秒内最多一次
const SWITCH_THROTTLE: Duration = Duration::from_secs(5);

/// 纯协议客户端（stdio JSONL 往返 + 进程冷启 + tab 切换限流）。
/// 定位缓存与 #10 hwnd 回收验证在 WtAdapter 层（core/terminal.rs）。
pub struct SidecarClient {
    exe: PathBuf,
    proc: Mutex<Option<Proc>>,
    last_switch: Mutex<Option<Instant>>,
}

struct Proc(Child, ChildStdin, BufReader<ChildStdout>);

impl SidecarClient {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self {
            exe: exe.into(),
            proc: Mutex::new(None),
            last_switch: Mutex::new(None),
        }
    }

    /// 公开请求口（启动扫描等 core 逻辑用）
    pub fn call(&self, req: &Value) -> Option<Value> {
        self.request(req)
    }

    fn request(&self, req: &Value) -> Option<Value> {
        let mut g = self.proc.lock().ok()?;
        for _ in 0..2 {
            if g.is_none() {
                *g = spawn(&self.exe);
            }
            let p = g.as_mut()?;
            match roundtrip(p, req) {
                Some(v) => return Some(v),
                None => *g = None, // 写断/读空/解析失败 = 进程死了：丢弃，下圈重拉
            }
        }
        None
    }

    /// read_tab（切 tab 读全文）：限流 5s 一次；WtAdapter 读取原语
    pub fn read_tab(&self, hwnd: i64, index: i64) -> Option<String> {
        {
            let mut last = self.last_switch.lock().ok()?;
            if let Some(t) = *last {
                let remain = SWITCH_THROTTLE.saturating_sub(t.elapsed());
                if !remain.is_zero() {
                    std::thread::sleep(remain);
                }
            }
            *last = Some(Instant::now());
        }
        let resp = self.request(&json!({ "cmd": "read_tab", "hwnd": hwnd, "index": index }))?;
        Some(resp["text"].as_str()?.to_string())
    }

    /// read_active_tab：不切换、非侵入只读当前活动 tab（调试 / stop 自动读的非打断路径）
    pub fn read_active_tab(&self, hwnd: i64) -> Option<String> {
        let resp = self.request(&json!({ "cmd": "read_active_tab", "hwnd": hwnd }))?;
        Some(resp["text"].as_str()?.to_string())
    }

}

fn roundtrip(p: &mut Proc, req: &Value) -> Option<Value> {
    writeln!(p.1, "{req}").and_then(|_| p.1.flush()).ok()?;
    let mut line = String::new();
    let n = p.2.read_line(&mut line).ok()?;
    if n == 0 {
        return None;
    }
    serde_json::from_str(&line).ok()
}

fn spawn(exe: &Path) -> Option<Proc> {
    let mut c = Command::new(exe);
    // GUI 进程（无控制台）拉起控制台子进程时，Windows 默认会为其分配一个
    // 可见控制台窗口（Win11 默认终端 = WT 时表现为「闪窗」）。CREATE_NO_WINDOW 抑制之。
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    let mut child = c
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdin = child.stdin.take()?;
    let stdout = BufReader::new(child.stdout.take()?);
    Some(Proc(child, stdin, stdout))
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 需要真实 sidecar exe + WT 窗口（手动跑）：
    /// `set AMBERY_SIDECAR=...\ambery-uia-sidecar.exe && cargo test -- --ignored`
    #[test]
    #[ignore = "需要真实 sidecar exe 与 WT 窗口，手动跑"]
    fn sidecar_read_real() {
        use crate::terminal::{TerminalAdapter, WtAdapter};
        let exe = std::env::var("AMBERY_SIDECAR").expect("AMBERY_SIDECAR not set");
        let adapter = WtAdapter::new(std::sync::Arc::new(SidecarClient::new(exe)));
        // 任取一个已知存在的 tab 名片段（环境相关，打印人工核对）
        let text = adapter
            .locate("PowerShell")
            .and_then(|tab| adapter.read(&tab));
        println!("read(PowerShell) → {:?}", text.map(|t| t.len()));
    }

    /// 假 sidecar 进程验证 read_active_tab 协议：非侵入只读命令，不携带 index。
    #[cfg(unix)]
    #[test]
    fn read_active_tab_uses_non_switching_protocol() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("ambery-sidecar-active-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let log = dir.join("requests.log");
        let script = dir.join("fake-sidecar.sh");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nwhile IFS= read -r line; do echo \"$line\" >> '{}'; echo '{{\"ok\":true,\"text\":\"ACTIVE\"}}'; done\n",
                log.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();

        let client = SidecarClient::new(&script);
        let text = client.read_active_tab(42).unwrap();
        assert_eq!(text, "ACTIVE");
        let requests = std::fs::read_to_string(&log).unwrap();
        assert!(requests.contains("\"cmd\":\"read_active_tab\""), "{requests}");
        assert!(requests.contains("\"hwnd\":42"), "{requests}");
        assert!(!requests.contains("\"index\""), "read_active_tab 不得带 index: {requests}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
