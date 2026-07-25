// 系统托盘 + 关闭隐藏到托盘
use tauri::AppHandle;
use tauri::Manager;
use tauri::WebviewWindow;
use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::TrayIconBuilder;

pub fn init_tray(app: &AppHandle, pet: &WebviewWindow) -> tauri::Result<()> {
    let toggle = MenuItemBuilder::with_id("toggle", "显示/隐藏").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "退出").build(app)?;
    let menu = MenuBuilder::new(app).item(&toggle).item(&quit).build()?;

    let _tray = TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("terminal-overseer")
        .menu(&menu)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "toggle" => {
                if let Some(w) = app.get_webview_window("pet") {
                    if w.is_visible().unwrap_or(false) {
                        let _ = w.hide();
                        // 连带隐藏 cards/chat
                        if let Some(c) = app.get_webview_window("cards") { let _ = c.hide(); }
                        if let Some(ch) = app.get_webview_window("chat") { let _ = ch.hide(); }
                    } else {
                        let _ = w.show();
                    }
                }
            }
            "quit" => app.exit(0),
            _ => {}
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
