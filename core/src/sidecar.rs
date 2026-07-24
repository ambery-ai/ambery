//! SidecarClient（docs/sidecar.md）：stdio JSON Lines 协议调用 C# UIA sidecar。
//! Mutex 串行化请求（UIA 切 Tab 是全局状态，不可并行）；进程退出自动重启一次。

use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

pub struct SidecarClient {
    exe: PathBuf,
    proc: Mutex<Option<SidecarProc>>,
}

struct SidecarProc {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl SidecarClient {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        Self {
            exe: exe.into(),
            proc: Mutex::new(None),
        }
    }

    fn request(&self, req: &Value) -> Option<Value> {
        let mut guard = self.proc.lock().ok()?;
        for _ in 0..2 {
            if guard.is_none() {
                *guard = SidecarProc::spawn(&self.exe).ok();
            }
            let Some(proc) = guard.as_mut() else { return None };
            match proc.roundtrip(req) {
                Ok(v) => return Some(v),
                Err(_) => *guard = None, // 重启一次再试
            }
        }
        None
    }

    /// terminal_reader 入口（docs/sidecar.md §读通道接线）：
    /// find_tab(instance) → read_tab(hwnd, index)；任一步失败返回 None（回退 Context）
    pub fn read_instance(&self, instance: &str) -> Option<String> {
        let found = self.request(&json!({ "cmd": "find_tab", "name": instance }))?;
        if !found.get("ok")?.as_bool()? {
            return None;
        }
        let hwnd = found.get("hwnd")?.as_i64()?;
        let index = found.get("index")?.as_i64()?;
        let resp = self.request(&json!({ "cmd": "read_tab", "hwnd": hwnd, "index": index }))?;
        if !resp.get("ok")?.as_bool()? {
            return None;
        }
        resp.get("text")?.as_str().map(String::from)
    }
}

impl SidecarProc {
    fn spawn(exe: &Path) -> std::io::Result<SidecarProc> {
        let mut child = Command::new(exe)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take().expect("stdin piped");
        let stdout = BufReader::new(child.stdout.take().expect("stdout piped"));
        Ok(SidecarProc {
            child,
            stdin,
            stdout,
        })
    }

    fn roundtrip(&mut self, req: &Value) -> std::io::Result<Value> {
        if self.child.try_wait()?.is_some() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "sidecar exited",
            ));
        }
        let line_req = serde_json::to_string(req)?;
        writeln!(self.stdin, "{line_req}")?;
        self.stdin.flush()?;
        let mut line = String::new();
        self.stdout.read_line(&mut line)?;
        if line.is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "sidecar closed stdout",
            ));
        }
        Ok(serde_json::from_str(&line)?)
    }
}

impl Drop for SidecarProc {
    fn drop(&mut self) {
        let _ = self.child.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 需要真实 sidecar exe + WT 窗口：
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
