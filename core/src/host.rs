//! 共享宿主装配：debug 宿主的统一骨架——
//! Config/Harness/LLM/Terminal Adapter 装配 → AppState → 完整 router。
//! ambery-case serve / frontend 共用；Tauri 壳自带窗口管理分歧，不复用本骨架。

use crate::llm::LlmBackend;
use crate::ambery::AmberyBackend;
use crate::server::{
    now_ms, router, spawn_config_watcher, spawn_cron_task, spawn_queue_consumer,
    spawn_timer_task, AppState,
};
use crate::terminal::{
    Composite, SidecarPlatformPrimitives, TerminalAdapter, WtAdapter, ZellijAdapter,
};
use crate::{Config, Harness};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::broadcast;

/// 装配产物：AppState + 后台任务参数 + config 目录（watcher 用）
pub struct HostParts {
    pub state: Arc<AppState>,
    pub config_dir: PathBuf,
    pub tick_ms: u64,
    pub timer_batch: usize,
}

/// 装配宿主：
/// Config（AMBERY_CONFIG_DIR 可覆盖，`adjust_config` 给调用方一次注入机会，
/// 如 serve 的 brain provider）→ Harness（AMBERY_STORAGE_DIR）→
/// LLM（`wrap_backend` 给调用方一次换入决策源的机会；serve/frontend 传恒等）→
/// Terminal Adapter（WtAdapter 受 adapter_wt 门控 + MapAdapter 兜底 → Composite；
/// primitives 经 sidecar 交付）。
pub fn assemble_host(
    adjust_config: impl FnOnce(&mut Config),
    wrap_backend: impl FnOnce(LlmBackend) -> LlmBackend,
) -> HostParts {
    let config_dir = crate::paths::config_root();
    let storage_dir = crate::paths::storage_dir();
    let mut config = Config::load_or_default(&config_dir);
    adjust_config(&mut config);
    // debug 宿主可用环境变量缩短 Timer 参数便于观察（真实值由 Config 定义）
    if let Some(n) = std::env::var("AMBERY_TIMER_INTERVAL_MS").ok().and_then(|v| v.parse().ok()) {
        config.timer.interval_ms = n;
    }
    if let Some(n) = std::env::var("AMBERY_TIMER_STAGGER_MS").ok().and_then(|v| v.parse().ok()) {
        config.timer.stagger_ms = n;
    }
    let tick_ms: u64 = std::env::var("AMBERY_TIMER_TICK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(config.timer.tick_ms as u64);
    let timer_batch = config.timer.batch;
    let harness = Harness::load_with_lang(
        &storage_dir,
        &config_dir,
        config.effective_compression_limit().unwrap_or(usize::MAX),
        now_ms(),
        crate::i18n::Lang::of(&config.harness_language),
    )
    .expect("load harness");
    println!("llm: active=「{}」", config.llm.active);
    // init 失败不阻断启动：记日志 + 按无 LLM 态运行（保循环可用，让用户能修配置）
    let backend = wrap_backend(LlmBackend::from_config(&config.llm).unwrap_or_else(|err| {
        eprintln!("[llm] {err}——按无 LLM 态运行");
        LlmBackend::debug(crate::llm::DebugAgent::default())
    }));
    let mut ambery = AmberyBackend::new(harness, config, backend);

    // Terminal Adapter 装配：adapter_wt 开关门控（冷字段）
    let sidecar = if ambery.config.terminal.adapter_wt {
        crate::paths::sidecar_exe()
            .map(crate::sidecar::SidecarClient::new)
            .map(Arc::new)
    } else {
        None
    };
    ambery.sidecar_enabled = sidecar.is_some();
    if let Some(p) = crate::paths::sidecar_exe() {
        if ambery.sidecar_enabled {
            println!("sidecar enabled: {}", p.display());
        }
    }
    let mut adapters: Vec<Arc<dyn TerminalAdapter>> = vec![];
    if let Some(sc) = &sidecar {
        adapters.push(Arc::new(WtAdapter::new(sc.clone())));
    }
    if ambery.config.terminal.adapter_zellij {
        adapters.push(Arc::new(ZellijAdapter::new(Arc::new(
            crate::terminal::ProcessZellijRunner,
        ))));
    }
    // 无可用读通道时保持 None（Hook 驱动核心体验不依赖 Terminal Adapter）
    if !adapters.is_empty() {
        ambery.terminal = Some(Arc::new(Composite::new(adapters)));
    }
    if let Some(sc) = &sidecar {
        ambery.primitives = Some(Arc::new(SidecarPlatformPrimitives::new(sc.clone())));
    }

    HostParts {
        state: Arc::new(AppState::new(ambery)),
        config_dir,
        tick_ms,
        timer_batch,
    }
}

/// 完整 router 服役：
/// 广播/effect sink/后台任务（timer/queue/config watcher/cron）+ axum serve。
/// 返回 Err 时由调用方决定退出方式（库不直接 exit）。
pub async fn serve_host(parts: HostParts, port: u16) -> Result<(), String> {
    let (tx, _) = broadcast::channel(64);
    let state = parts.state;
    let tx_for_ws = tx.clone();
    state
        .set_sender(Box::new(move |msg: Value| {
            let _ = tx.send(msg.to_string());
        }))
        .await;
    state.wire_effect_sink().await;
    spawn_timer_task(state.clone(), parts.tick_ms, parts.timer_batch);
    spawn_queue_consumer(state.clone());
    // 外部文件自动载入
    spawn_config_watcher(state.clone(), parts.config_dir.clone());
    // Cron 调度任务
    spawn_cron_task(state.clone());
    let app = router(state, tx_for_ws);

    let addr = format!("127.0.0.1:{port}");
    let listener = tokio::net::TcpListener::bind(&addr).await.map_err(|e| {
        if e.kind() == std::io::ErrorKind::AddrInUse {
            format!("端口 {port} 已被占用（{e}）。serve 依赖固定端口，不能静默换端口；请关闭占用进程，或设置 AMBERY_PORT 换端口并同步更新 hook 脚本配置。")
        } else {
            format!("bind {addr}: {e}")
        }
    })?;
    println!("ambery-core debug listening on http://{addr}");
    axum::serve(listener, app).await.map_err(|e| format!("serve 失败：{e}"))
}
