// 托盘设置面板窗口（docs/config.md）：右键托盘弹出，失焦自动隐藏——菜单行为
use tauri::{Manager, WebviewWindow};

/// 在点击位置上方弹出面板（托盘在屏幕底部，向上展开）
pub fn show_at(app: &tauri::AppHandle, x: f64, y: f64) {
    let Some(w) = app.get_webview_window("menu") else { return };
    let (w_px, h_px) = (380.0, 560.0);
    // 水平居中于点击点，垂直贴着点击点上方；粗防出屏
    let px = (x - w_px / 2.0).max(8.0) as i32;
    let py = (y - h_px - 12.0).max(8.0) as i32;
    let _ = w.set_position(tauri::PhysicalPosition::new(px, py));
    let _ = w.show();
    let _ = w.set_focus();
}

/// 失焦 → 隐藏（菜单语义：点别的地方就关）
pub fn init_menu_window(menu: &WebviewWindow) {
    let m = menu.clone();
    menu.on_window_event(move |event| {
        if let tauri::WindowEvent::Focused(false) = event {
            let _ = m.hide();
        }
    });
}
