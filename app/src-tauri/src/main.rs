//! Tauri 壳（docs/tauri-shell.md）：单透明 overlay 窗口 + 内嵌 overseer-core server。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::DebugAgent;
use overseer_core::overseer::Overseer;
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::broadcast;

mod window;
mod tray;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let win = app.get_webview_window("main").expect("main window");

            window::init_window(&win);
            tray::init_tray(app.handle(), &win)?;

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
        config.base_prompt.clone(),
        config.token_threshold,
        now_ms(),
    )
    .expect("load harness");
    let mut overseer = Overseer::new(harness, config, DebugAgent::default());
    // 读通道链（docs/sidecar.md）：sidecar → MockTerminals → Context
    let sidecar = std::env::var("OVERSEER_SIDECAR")
        .ok()
        .map(overseer_core::sidecar::SidecarClient::new)
        .map(Arc::new);
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
    spawn_timer_task(state.clone(), 60_000, 2);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600")
        .await
        .expect("bind 47600");
    axum::serve(listener, app).await.expect("serve core");
}
