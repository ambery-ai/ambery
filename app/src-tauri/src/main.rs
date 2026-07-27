//! Tauri 壳（docs/multi-window.md）：三独立窗口（pet + cards + chat）+ 内嵌 overseer-core。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::LlmBackend;
use overseer_core::overseer::OverseerBackend;
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::broadcast;

mod menu_window;
mod window;
mod tray;

/// debug 前端从 devUrl（tauri.conf.json，5199 vite dev server）加载——
/// vite 没跑时 webview 只能拿到过期/空白内容，必须报警而不是静默
#[cfg(debug_assertions)]
fn warn_if_dev_server_down() {
    let down = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], 5199)),
        std::time::Duration::from_millis(300),
    )
    .is_err();
    if down {
        eprintln!("[dev] vite dev server (5199) 没在跑！先起：cd app && npx vite --port 5199 --strictPort");
        // 后台线程弹框：报警不能阻塞启动（模态框卡死过启动链路）
        std::thread::spawn(|| unsafe {
            use windows::Win32::UI::WindowsAndMessaging::*;
            MessageBoxW(
                windows::Win32::Foundation::HWND(std::ptr::null_mut()),
                windows::core::w!("debug 前端从 devUrl 加载，但 vite dev server (5199) 没在跑。\n\n先起：cd app && npx vite --port 5199 --strictPort\n再起本 app。"),
                windows::core::w!("terminal-overseer (debug)"),
                MB_ICONWARNING,
            );
        });
    }
}

/// 面板底部按钮（原托盘菜单动作）
#[tauri::command]
fn toggle_pet(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("pet") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            if let Some(c) = app.get_webview_window("cards") { let _ = c.hide(); }
            if let Some(ch) = app.get_webview_window("chat") { let _ = ch.hide(); }
        } else {
            let _ = w.show();
        }
    }
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

fn main() {
    #[cfg(debug_assertions)]
    warn_if_dev_server_down();
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![toggle_pet, quit_app])
        .setup(|app| {
            let pet = app.get_webview_window("pet").expect("pet window");
            let cards = app.get_webview_window("cards").expect("cards window");
            let chat = app.get_webview_window("chat").expect("chat window");
            let menu = app.get_webview_window("menu").expect("menu window");

            // 所有窗口统一 pin + fight-back（menu 面板除外：它是菜单行为，失焦即隐）
            window::init_window(&pet);
            window::init_window(&cards);
            window::init_window(&chat);
            menu_window::init_menu_window(&menu);

            // 托盘：右键弹设置面板
            tray::init_tray(app.handle(), &pet)?;

            // 复用 Tauri async runtime 启动 overseer-core
            tauri::async_runtime::spawn(run_core());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn run_core() {
    // Config / Storage 分离（concepts §12/§13，core/paths.rs）
    let config = Config::load_or_default(&overseer_core::paths::config_root());
    let harness = Harness::load(
        &overseer_core::paths::storage_dir(),
        &overseer_core::paths::config_root(),
        config.token_threshold,
        now_ms(),
    )
    .expect("load harness");
    let backend = LlmBackend::from_config(&config.llm);
    let mut overseer = OverseerBackend::new(harness, config, backend);
    // 读通道链（docs/sidecar.md）：sidecar → MockTerminals → Context
    // 读通道链（docs/sidecar.md）：sidecar（路径自动发现，env 可覆盖）→ MockTerminals → Context
    let sidecar = overseer_core::paths::sidecar_exe()
        .map(overseer_core::sidecar::SidecarClient::new)
        .map(Arc::new);
        .map(Arc::new);
    overseer.sidecar_enabled = sidecar.is_some();
    let mock = Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::<String, String>::new(),
    ));
    {
        let mock = mock.clone();
        overseer.terminal_reader = Some(Arc::new(move |inst: &str| {
            sidecar
                .as_ref()
                .and_then(|s| s.read_instance(inst))
                .or_else(|| mock.lock().unwrap().get(inst).cloned())
        }));
    }
    let (tx, _) = broadcast::channel(64);
    let state = Arc::new(AppState::new(overseer, tx, mock));
    spawn_timer_task(state.clone(), 60_000, 2); // 真实 tick：60s（Config 的 5min 间隔由 TimerWheel 控制）
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600")
        .await
        .expect("bind 47600");
    axum::serve(listener, app).await.expect("serve core");
}
