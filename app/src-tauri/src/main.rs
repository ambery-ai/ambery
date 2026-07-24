//! Tauri 壳（docs/tauri-shell.md）：单透明 overlay 窗口 + 内嵌 overseer-core server。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::DebugAgent;
use overseer_core::overseer::Overseer;
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use std::sync::Arc;
use tokio::sync::broadcast;

fn main() {
    // 内嵌 overseer-core（spec.md 架构决定 #1：前端始终走 HTTP+WS loopback）
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(run_core());
    });

    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn run_core() {
    let dir = std::env::var("OVERSEER_STORAGE").unwrap_or_else(|_| "storage".into());
    let dir = std::path::Path::new(&dir);
    let config = Config::load_or_default(dir);
    let harness = Harness::load(
        dir,
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
    spawn_timer_task(state.clone(), 60_000, 2); // 真实 tick：60s（Config 的 5min 间隔由 TimerWheel 控制）
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600")
        .await
        .expect("bind 47600");
    axum::serve(listener, app).await.expect("serve core");
}
