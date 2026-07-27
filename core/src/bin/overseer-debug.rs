//! debug 模式入口：overseer-core + DebugAgent + HTTP/WS server。
//! 用法：`cargo run --bin overseer-debug`（storage 目录默认 ../storage，OVERSEER_STORAGE 可覆盖）

use overseer_core::llm::{DebugAgent, LlmBackend, LlmOutput};
use overseer_core::overseer::OverseerBackend;
use overseer_core::queue::{QueueMessage, ToolCall};
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use std::sync::Arc;
use tokio::sync::broadcast;

/// debug CLI 决策源：人/外部脚本当 LLM（mock 零逻辑，决策全来自 stdin）。
/// 每次调用把全量 Queue 以机读帧打到 stdout（@@@ 前缀与 server 日志区分），
/// 再从 stdin 读响应：`c <文本>` | `t <tool名> <json参数>`（可多行）| 空行提交。
/// 外部脚本（如 scripts/debug_brain.py）接管本进程 stdin/stdout 即可驱动。
fn cli_decide(messages: &[QueueMessage]) -> LlmOutput {
    use std::io::BufRead;
    println!("@@@QUEUE_BEGIN");
    for (i, m) in messages.iter().enumerate() {
        println!(
            "{}",
            serde_json::json!({
                "i": i,
                "role": m.role,
                "content": m.content,
                "tool_calls": m.tool_calls,
                "tool_call_id": m.tool_call_id,
            })
        );
    }
    println!("@@@QUEUE_END");
    println!("@@@PROMPT c <文本> | t <tool名> <json参数> | 空行提交");
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut content = None;
    let mut tool_calls = vec![];
    loop {
        let mut line = String::new();
        match lock.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF：按已输入内容提交
            Ok(_) => {}
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(text) = line.strip_prefix("c ") {
            content = Some(text.to_string());
        } else if let Some(rest) = line.strip_prefix("t ") {
            match rest.split_once(' ') {
                Some((name, args)) => tool_calls.push(ToolCall {
                    id: format!("cli-{}", tool_calls.len() + 1),
                    name: name.to_string(),
                    arguments: args.to_string(),
                }),
                None => println!("@@@ERR 格式：t <tool名> <json参数>"),
            }
        } else {
            println!("@@@ERR 无法识别：c <文本> | t <tool名> <json参数> | 空行提交");
        }
    }
    LlmOutput {
        content,
        tool_calls,
        reasoning_content: None,
    }
}

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
    let harness = Harness::load(&storage_dir, &config_dir, config.token_threshold, now_ms())
        .expect("load harness");
    let backend = match LlmBackend::from_config(&config.llm) {
        // debug 分支换 CLI 决策源（OpenAi 变体的内部降级仍是沉默 mock）
        LlmBackend::Debug(_) => LlmBackend::Debug(DebugAgent::new(cli_decide)),
        b => b,
    };
    println!(
        "llm: active=「{}」→ {}",
        config.llm.active,
        match &backend {
            LlmBackend::Debug(_) => "DebugAgent（CLI 决策源）",
            LlmBackend::OpenAi { .. } => "OpenAiClient（失败降级 DebugAgent）",
        }
    );
    let mut overseer = OverseerBackend::new(harness, config, backend);
    // 读通道链（docs/sidecar.md）：sidecar（路径自动发现，env 可覆盖）→ MockTerminals → Context
    let sidecar = overseer_core::paths::sidecar_exe()
        .map(overseer_core::sidecar::SidecarClient::new)
        .map(Arc::new);
    overseer.sidecar_enabled = sidecar.is_some();
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
