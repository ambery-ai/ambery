// 托盘设置面板窗口：右键托盘弹出，失焦自动隐藏——菜单行为
use tauri::{Manager, WebviewWindow};
use std::sync::Mutex;
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::SetForegroundWindow;

/// 显示时刻：失焦隐藏的武装延迟依据（焦点接力失败也不秒杀面板）
static SHOWN_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// 在点击位置上方弹出面板（托盘在屏幕底部，向上展开）。
/// 定位与显示经动作层（window_moved + window_visible 逐动作记录）
pub fn show_at(app: &tauri::AppHandle, x: f64, y: f64) {
    let Some(w) = app.get_webview_window("menu") else { return };
    let (w_px, h_px) = (380.0, 560.0);
    // 水平居中于点击点，垂直贴着点击点上方；粗防出屏
    let px = (x - w_px / 2.0).max(8.0) as i32;
    let py = (y - h_px - 12.0).max(8.0) as i32;
    crate::tauri_runtime_actions::move_window(app, "menu", px, py);
    crate::tauri_runtime_actions::show_window(app, "menu");
    *SHOWN_AT.lock().unwrap() = Some(Instant::now());
    // 直接 Win32 抢前台（Windows 专属）：Tauri set_focus
    // 在后台进程下会被 Windows 焦点保护静默拒绝，而托盘点击上下文授予了前台权
    #[cfg(windows)]
    if let Ok(hwnd) = w.hwnd() {
        unsafe {
            let _ = SetForegroundWindow(windows::Win32::Foundation::HWND(hwnd.0 as *mut _));
        }
    }
    // 非 Windows：走 Tauri 标准聚焦
    #[cfg(not(windows))]
    let _ = w.set_focus();
    // focus 是独立的非只读动作，单独记录
    crate::tauri_runtime_actions::record(app, "window_focused", serde_json::json!({ "window": "menu" }));
}

/// 失焦 → 隐藏（菜单语义：点别的地方就关）；
/// 武装延迟：显示后 600ms 内的失焦事件忽略（焦点接力失败不秒杀）
pub fn init_menu_window(menu: &WebviewWindow) {
    let m = menu.clone();
    menu.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let armed = SHOWN_AT
                .lock()
                .unwrap()
                .is_none_or(|t| t.elapsed() > Duration::from_millis(600));
            if armed {
                crate::tauri_runtime_actions::hide_window(m.app_handle(), "menu");
            }
        }
    });
}
