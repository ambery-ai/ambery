// 系统托盘 + 关闭隐藏到托盘。
// 右键 → 设置面板（docs/config.md：原生菜单退役，100% web 渲染）
use tauri::AppHandle;
use tauri::WebviewWindow;
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

use crate::menu_window;

pub fn init_tray(app: &AppHandle, pet: &WebviewWindow) -> tauri::Result<()> {
    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("terminal-overseer  右键 = 设置")
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Right,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                menu_window::show_at(tray.app_handle(), position.x, position.y);
            }
        })
        .build(app)?;

    // 关闭 → 隐藏到托盘
    let pet_clone = pet.clone();
    pet.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = pet_clone.hide();
        }
    });

    Ok(())
}
