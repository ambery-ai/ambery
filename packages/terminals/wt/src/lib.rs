//! ambery-terminal-wt：Windows Terminal 叶子。
//!
//! 读写面都经独立 C# sidecar 进程（UIA 枚举 + TermControl TextPattern 读取 +
//! COM 虚拟桌面切换）：SidecarClient 是 stdio JSONL 协议客户端，WtAdapter
//! 提供 enumerate/read，SidecarPlatformPrimitives 提供平台原语。
//! 平台边界：UIA sidecar 是 Windows 可选增强——非 Windows 一律 None
//!（不发现、不启动、不使用；Hook 驱动核心体验不依赖它）。

use ambery_terminal_lib::{PlatformPrimitives, ReadOutcome, TabInfo, TabRef, TerminalAdapter};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// UIA sidecar exe 路径发现（顺序即优先级）：
/// `AMBERY_SIDECAR` env > 当前 exe 旁（Tauri externalBin 布局）> exe 旁 sidecar/ >
/// Release publish（self-contained win-x64，打包定案）> Debug（仓库开发）。
#[cfg(windows)]
pub fn sidecar_exe() -> Option<PathBuf> {
    let env_path = std::env::var("AMBERY_SIDECAR").ok().map(PathBuf::from);
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    for p in sidecar_candidates(env_path.as_deref(), &exe_dir, env!("CARGO_MANIFEST_DIR")) {
        if p.exists() {
            return Some(p);
        }
    }
    None
}

/// 候选路径（纯函数，跨平台可测）。env_path=None 时跳过第一级。
pub fn sidecar_candidates(
    env_path: Option<&Path>,
    exe_dir: &Path,
    manifest_dir: &str,
) -> Vec<PathBuf> {
    if !exe_dir.exists() {
        return vec![];
    }
    let exe_name = "ambery-uia-sidecar.exe";
    let mut out = Vec::new();
    if let Some(p) = env_path {
        out.push(p.to_path_buf());
    }
    out.push(exe_dir.join(exe_name));
    out.push(exe_dir.join("sidecar").join(exe_name));
    out.push(
        PathBuf::from(manifest_dir)
            .join("sidecar/bin/Release/net9.0-windows/win-x64/publish")
            .join(exe_name),
    );
    out.push(
        PathBuf::from(manifest_dir)
            .join("sidecar/bin/Debug/net9.0-windows")
            .join(exe_name),
    );
    out
}

/// 非 Windows：无 UIA sidecar
#[cfg(not(windows))]
pub fn sidecar_exe() -> Option<PathBuf> {
    None
}

/// tab 切换限流：全局 5 秒内最多一次
const SWITCH_THROTTLE: Duration = Duration::from_secs(5);

/// SidecarClient：stdio JSON Lines 调 C# UIA sidecar。
/// Mutex 串行化请求（UIA 切 Tab 是全局状态，不可并行）。
/// 常驻简化语义：进程死了即弃，下次请求现拉起（冷启 ~200ms）——无保活预检、无心跳。
///
/// 纯协议客户端（stdio JSONL 往返 + 进程冷启 + tab 切换限流）。
/// 实例语义全在消费方；本客户端只有载体级命令。
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

    /// 公开请求口（启动扫描等消费方逻辑用）
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

    /// list_wt_windows：WT（CASCADIA class）顶层窗口枚举——enumerate 的窗口级原语
    pub fn list_wt_windows(&self) -> Option<Vec<(i64, String)>> {
        let resp = self.request(&json!({ "cmd": "list_wt_windows" }))?;
        resp["windows"]
            .as_array()?
            .iter()
            .map(|w| Some((w["hwnd"].as_i64()?, w["title"].as_str()?.to_string())))
            .collect()
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

/// WtAdapter：Windows Terminal 经 C# sidecar
/// 独立进程访问（UIA 枚举 + TermControl TextPattern 读取）。
/// 纯 L1：不持实例缓存、不做 marker 匹配；tab 位置漂移的自愈与判死
/// 由消费方枚举对账承担。
pub struct WtAdapter {
    sidecar: Arc<SidecarClient>,
}

impl WtAdapter {
    pub fn new(sidecar: Arc<SidecarClient>) -> Self {
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
///（COM IVirtualDesktopManager 在 sidecar 内）
pub struct SidecarPlatformPrimitives {
    sidecar: Arc<SidecarClient>,
}

impl SidecarPlatformPrimitives {
    pub fn new(sidecar: Arc<SidecarClient>) -> Self {
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
    fn sidecar_candidates_cover_env_exe_and_release_layout() {
        let exe_dir = std::env::temp_dir().join("ambery-exe-dir");
        let _ = std::fs::remove_dir_all(&exe_dir);
        std::fs::create_dir_all(&exe_dir).unwrap();
        let manifest = std::env::temp_dir().join("ambery-manifest");
        let env_p = exe_dir.join("env-sidecar.exe");
        std::fs::write(&env_p, "x").unwrap();
        let sibling = exe_dir.join("ambery-uia-sidecar.exe");
        std::fs::write(&sibling, "x").unwrap();
        let rel_release = exe_dir.join("sidecar/ambery-uia-sidecar.exe");
        std::fs::create_dir_all(rel_release.parent().unwrap()).unwrap();
        std::fs::write(&rel_release, "x").unwrap();

        let candidates = sidecar_candidates(Some(&env_p), &exe_dir, manifest.to_str().unwrap());
        let names: Vec<_> = candidates
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(names, vec!["env-sidecar.exe", "ambery-uia-sidecar.exe", "ambery-uia-sidecar.exe", "ambery-uia-sidecar.exe", "ambery-uia-sidecar.exe"], "{candidates:?}");
        // 顺序：env > exe 旁 sibling > exe 旁 sidecar/ > Release publish > Debug
        // join 字符串字面量在 Windows 保留 `/`（display 混用分隔符）——先统一为平台分隔符再匹配
        let sep = std::path::MAIN_SEPARATOR;
        let paths: Vec<_> = candidates
            .iter()
            .map(|p| p.display().to_string().replace('/', &sep.to_string()))
            .collect();
        let norm = |n: &str| n.replace('/', &sep.to_string());
        let pos = |needle: &str| paths.iter().position(|p| p.contains(&norm(needle))).unwrap();
        assert!(pos("env-sidecar") < pos("sidecar/ambery-uia-sidecar"));
        assert!(pos("sidecar/ambery-uia-sidecar") < pos("Release/net9.0-windows/win-x64/publish"));
        assert!(pos("Release/net9.0-windows/win-x64/publish") < pos("Debug/net9.0-windows"));
        let _ = std::fs::remove_dir_all(&exe_dir);
    }

    #[test]
    fn sidecar_candidates_require_existing_exe_dir() {
        let dir = std::env::temp_dir().join("ambery-no-such-dir");
        let candidates = sidecar_candidates(None, &dir, "/nonexistent/manifest");
        assert!(candidates.is_empty());
    }

    /// 需要真实 sidecar exe + WT 窗口（手动跑）：
    /// `set AMBERY_SIDECAR=...\ambery-uia-sidecar.exe && cargo test -- --ignored`
    #[test]
    #[ignore = "需要真实 sidecar exe 与 WT 窗口，手动跑"]
    fn sidecar_read_real() {
        let exe = std::env::var("AMBERY_SIDECAR").expect("AMBERY_SIDECAR not set");
        let adapter = WtAdapter::new(Arc::new(SidecarClient::new(exe)));
        // 枚举全部 WT tab（环境相关，打印人工核对）
        let tabs = adapter.enumerate();
        println!("enumerate → {:?}", tabs.map(|t| t.len()));
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
