// 窗口置顶模式（T14）：三档应用 + 轮询线程生命周期管理
// aggressive = 跨虚拟桌面 pin + 500ms 轮询重申 TOPMOST（任务栏/他窗抢顶时 fight-back）
// topmost    = 仅 alwaysOnTop 窗口属性
// off        = 普通窗口（既不 pin 也不置顶）
// pin/轮询为 Windows 专属（winvd/windows 依赖仅 cfg(windows) 目标拉入）；
// 非 Windows 只应用 alwaysOnTop（Tauri 跨平台处理）。
use tauri::WebviewWindow;

use ambery_core::config::TopmostMode;

/// 轮询线程停止令牌注册表：窗口 label → 停止标志。
/// 热切换档位或窗口销毁时停掉旧线程——线程不泄漏、不再幽灵重申。
#[derive(Default)]
pub struct TopmostRegistry(
    std::sync::Mutex<
        std::collections::HashMap<String, std::sync::Arc<std::sync::atomic::AtomicBool>>,
    >,
);

impl TopmostRegistry {
    /// 停掉指定窗口的轮询线程（若有）；线程在下一个 tick（≤500ms）退出
    pub fn stop(&self, label: &str) {
        if let Some(flag) = self.0.lock().unwrap().remove(label) {
            flag.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

/// 按模式应用窗口置顶行为：先停旧轮询（幂等），再按档落定属性 / pin / 轮询。
pub fn apply_topmost(window: &WebviewWindow, mode: TopmostMode, registry: &TopmostRegistry) {
    let label = window.label().to_string();
    registry.stop(&label);
    let _ = window.set_always_on_top(mode != TopmostMode::Off);
    #[cfg(windows)]
    match mode {
        TopmostMode::Aggressive => start_aggressive(window, label, registry),
        // 非 aggressive 不持有 pin；尽力 unpin（未 pin 过/句柄异常时静默）
        TopmostMode::Topmost | TopmostMode::Off => unpin(window),
    }
}

/// aggressive 档全集：跨虚拟桌面 pin + 500ms 轮询重申 TOPMOST（可停止）
#[cfg(windows)]
fn start_aggressive(window: &WebviewWindow, label: String, registry: &TopmostRegistry) {
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::thread;
    use std::time::Duration;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    };

    let raw = window.hwnd().expect("hwnd").0 as *mut core::ffi::c_void;
    if let Err(err) = winvd::pin_window(HWND(raw)) {
        eprintln!("winvd pin_window: {err:?}");
    }
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    registry.0.lock().unwrap().insert(label, stop.clone());
    let hwnd_val = raw as isize;
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(500));
        if stop.load(Ordering::Relaxed) {
            break;
        }
        unsafe {
            let _ = SetWindowPos(
                HWND(hwnd_val as *mut _),
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            );
        }
    });
}

#[cfg(windows)]
fn unpin(window: &WebviewWindow) {
    use windows::Win32::Foundation::HWND;
    let raw = window.hwnd().expect("hwnd").0 as *mut core::ffi::c_void;
    let _ = winvd::unpin_window(HWND(raw));
}
