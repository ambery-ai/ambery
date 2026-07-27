//! SidecarClient（docs/sidecar.md）：stdio JSON Lines 调 C# UIA sidecar。
//! Mutex 串行化请求（UIA 切 Tab 是全局状态，不可并行）。
//! 常驻简化语义：进程死了即弃，下次请求现拉起（冷启 ~200ms）——无保活预检、无心跳。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

pub struct SidecarClient {
    exe: PathBuf,
    proc: Mutex<Option<Proc>>,
}

struct Proc(Child, ChildStdin, BufReader<ChildStdout>);

impl SidecarClient {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self {
            exe: exe.into(),
            proc: Mutex::new(None),
        }
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

    /// terminal_reader 入口（docs/sidecar.md §读通道接线）：
    /// find_tab(instance) → read_tab(hwnd, index)；任一步失败返回 None（回退 Context）
    pub fn read_instance(&self, instance: &str) -> Option<String> {
        let found = self.request(&json!({ "cmd": "find_tab", "name": instance }))?;
        let (hwnd, index) = (found["hwnd"].as_i64()?, found["index"].as_i64()?);
        let resp = self.request(&json!({ "cmd": "read_tab", "hwnd": hwnd, "index": index }))?;
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
    let mut c = Command::new(exe)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let stdin = c.stdin.take()?;
    let stdout = BufReader::new(c.stdout.take()?);
    Some(Proc(c, stdin, stdout))
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
    /// `set OVERSEER_SIDECAR=...\overseer-uia-sidecar.exe && cargo test -- --ignored`
    #[test]
    #[ignore = "需要真实 sidecar exe 与 WT 窗口，手动跑"]
    fn sidecar_read_instance_real() {
        let exe = std::env::var("OVERSEER_SIDECAR").expect("OVERSEER_SIDECAR not set");
        let client = SidecarClient::new(exe);
        // 任取一个已知存在的 tab 名片段（环境相关，打印人工核对）
        let text = client.read_instance("PowerShell");
        println!("read_instance(PowerShell) → {:?}", text.map(|t| t.len()));
    }
}
