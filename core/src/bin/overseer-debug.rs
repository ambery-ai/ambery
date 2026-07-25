//! debug 模式入口：overseer-core + DebugAgent + HTTP/WS server。
//! 用法：`cargo run --bin overseer-debug`（storage 目录默认 ../storage，OVERSEER_STORAGE 可覆盖）

use overseer_core::llm::DebugAgent;
use overseer_core::overseer::Overseer;
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use std::sync::Arc;
use tokio::sync::broadcast;

#[tokio::main]
async fn main() {
    // Config / Storage 分离（concepts §12/§13，core/paths.rs）
    let config_dir = overseer_core::paths::config_root();
    let storage_dir = overseer_core::paths::storage_dir();
    let mut config = Config::load_or_default(&config_dir);
    // debug 模式可用环境变量缩短 Timer 参数便于观察（真实值由 Config 定义：5min/30s）
    if let Some(n) = std::env::var("OVERSEER_TIMER_INTERVAL_MS").ok().and_then(|v| v.parse().ok()) {
        config.timer_interval_ms = n;
    }
    if let Some(n) = std::env::var("OVERSEER_TIMER_STAGGER_MS").ok().and_then(|v| v.parse().ok()) {
        config.timer_stagger_ms = n;
    }
    let tick_ms: u64 = std::env::var("OVERSEER_TIMER_TICK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(5_000);
    let harness = Harness::load(
        &storage_dir,
        config.base_prompt.clone(),
        config.token_threshold,
        now_ms(),
    )
    .expect("load harness");
    let mut overseer = Overseer::new(harness, config, DebugAgent::default());
    // 读通道链（docs/sidecar.md）：sidecar（OVERSEER_SIDECAR 指定）→ MockTerminals → Context
    let sidecar = std::env::var("OVERSEER_SIDECAR")
        .ok()
        .map(overseer_core::sidecar::SidecarClient::new)
        .map(Arc::new);
    if sidecar.is_some() {
        println!("sidecar enabled: {}", std::env::var("OVERSEER_SIDECAR").unwrap());
    }
    let mock = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<String, String>::new()));
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
    spawn_timer_task(state.clone(), tick_ms, 2);
    let app = router(state);

    let addr = "127.0.0.1:47600";
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("bind 47600");
    println!(
        "overseer-core debug listening on http://{addr}\n  config:  {}\n  storage: {}\n  timer tick: {tick_ms}ms",
        config_dir.display(),
        storage_dir.display()
    );
    axum::serve(listener, app).await.expect("serve");
}
