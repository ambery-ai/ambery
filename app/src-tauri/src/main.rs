//! Tauri 壳（docs/multi-window.md）：三独立窗口（pet + cards + chat）+ 内嵌 overseer-core。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::LlmBackend;
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
            let pet = app.get_webview_window("pet").expect("pet window");
            let cards = app.get_webview_window("cards").expect("cards window");
            let chat = app.get_webview_window("chat").expect("chat window");

            // 所有窗口统一 pin + fight-back
            window::init_window(&pet);
            window::init_window(&cards);
            window::init_window(&chat);

            // 托盘：控制 pet 窗口（连带隐藏 cards/chat）
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
    let mut overseer = Overseer::new(harness, config, backend);
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
    spawn_timer_task(state.clone(), 60_000, 2); // 真实 tick：60s（Config 的 5min 间隔由 TimerWheel 控制）
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600")
        .await
        .expect("bind 47600");
    axum::serve(listener, app).await.expect("serve core");
}
