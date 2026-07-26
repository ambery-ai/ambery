//! HTTP + WS server（docs/agent-loop.md §协议）：debug 模式下前端与 Harness 的唯一协议。

use axum::{
    extract::ws::{Message, WebSocket, WebSocketUpgrade},
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};
use tower_http::cors::CorsLayer;

use crate::llm::LlmBackend;
use crate::overseer::{Effect, OverseerBackend};
use crate::queue::{QueueMessage, Role};
use crate::Config;

pub struct AppState {
    overseer: Mutex<OverseerBackend<LlmBackend>>,
    /// ペット已通知、用户尚未查看的数量（debug 语义：RenderComponent +1，用户发消息清零，关卡片 -1）
    pending_notifications: Mutex<usize>,
    /// MockTerminals（docs/timer.md §Scanner）：instance → 当前终端文本，模拟读通道
    /// 与 Overseer.terminal_reader 共享同一份
    mock_terminals: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    tx: broadcast::Sender<String>,
}

impl AppState {
    pub fn new(
        overseer: OverseerBackend<LlmBackend>,
        tx: broadcast::Sender<String>,
        mock_terminals: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    ) -> Self {
        Self {
            overseer: Mutex::new(overseer),
            pending_notifications: Mutex::new(0),
            mock_terminals,
            tx,
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/state", get(get_state))
        .route("/queue", get(get_queue))
        .route("/queue/user", post(post_user))
        .route("/events", post(post_event))
        .route("/config", get(get_config))
        .route("/config/schema", get(get_config_schema))
        .route("/config", post(post_config))
        .route("/hook", post(post_hook))
        .route("/debug/terminal", post(post_debug_terminal))
        .route("/ws", get(ws_upgrade))
        .layer(CorsLayer::permissive())
        .with_state(state)
}

fn config_json(cfg: &Config) -> Value {
    json!({
        "kaomoji": cfg.kaomoji,
        "setAutonomyDefaultTtlMs": cfg.set_autonomy_default_ttl_ms,
        "viewScale": cfg.view_scale,
    })
}

async fn state_json(s: &AppState) -> Value {
    let ov = s.overseer.lock().await;
    let pending = *s.pending_notifications.lock().await;
    json!({
        "instances": ov.harness.agents.iter().map(|a| json!({
            "id": a.hash, "name": a.name, "status": a.status
        })).collect::<Vec<_>>(),
        "pendingNotifications": pending
    })
}

/// 副作用 → WS 推送 + queue/top_state 变更广播
async fn broadcast_effects(s: &AppState, effects: Vec<Effect>) {
    for e in effects {
        let msg = match e {
            Effect::RenderComponent(spec) => {
                *s.pending_notifications.lock().await += 1;
                json!({ "kind": "render_component", "spec": spec })
            }
            Effect::SetAutonomy {
                face,
                motion,
                ttl_ms,
            } => json!({ "kind": "set_autonomy", "face": face, "motion": motion, "ttlMs": ttl_ms }),
            Effect::ConfigChanged => {
                let cfg = s.overseer.lock().await.config.clone();
                json!({ "kind": "config", "config": config_json(&cfg) })
            }
        };
        let _ = s.tx.send(msg.to_string());
    }
    let _ = s.tx.send(json!({ "kind": "queue_changed" }).to_string());
    let st = state_json(s).await;
    let _ = s.tx.send(json!({ "kind": "top_state", "state": st }).to_string());
}

async fn get_state(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    Json(state_json(&s).await)
}

async fn get_queue(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    Json(json!(ov.harness.queue.messages()))
}

#[derive(Deserialize)]
struct UserBody {
    text: String,
}

async fn post_user(State(s): State<Arc<AppState>>, Json(body): Json<UserBody>) -> impl IntoResponse {
    *s.pending_notifications.lock().await = 0;
    let mut ov = s.overseer.lock().await;
    if let Err(err) = ov
        .harness
        .append_queue(QueueMessage::new(Role::User, body.text, now_ms()))
    {
        return err_response(err);
    }
    let effects = match ov.run_trigger(now_ms(), 0).await {
        Ok(e) => e,
        Err(err) => return err_response(err),
    };
    drop(ov);
    broadcast_effects(&s, effects).await;
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct EventBody {
    desc: String,
}

async fn post_event(
    State(s): State<Arc<AppState>>,
    Json(body): Json<EventBody>,
) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    // debug 语义：关闭卡片视为已读一个通知
    if body.desc.starts_with("用户关闭了") {
        let mut p = s.pending_notifications.lock().await;
        *p = p.saturating_sub(1);
    }
    ov.harness.event_buffer.push(body.desc);
    Json(json!({ "ok": true })).into_response()
}

async fn get_config(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    Json(config_json(&ov.config))
}

/// 声明式 UI 反射（docs/config.md）：schema 节点 + 当前值，CLI/托盘面板的唯一数据源
async fn get_config_schema(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    Json(json!({
        "version": crate::config::migrate::CURRENT_VERSION,
        "readOnly": ov.config.read_only,
        "nodes": crate::config::reflect::config_nodes(&ov.config),
    }))
}

#[derive(Deserialize)]
struct SetConfigBody {
    path: String,
    value: Value,
}

/// 统一修改管道：验证 → 热应用 → persist → 广播；restart_required 如实上报
async fn post_config(
    State(s): State<Arc<AppState>>,
    Json(body): Json<SetConfigBody>,
) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    match ov.apply_config_by_path(&body.path, body.value) {
        Ok(outcome) => {
            if outcome.llm_changed {
                let backend = LlmBackend::from_config(&ov.config.llm);
                ov.replace_llm(backend);
            }
            let restart = outcome.restart_required.clone();
            drop(ov);
            broadcast_effects(&s, outcome.effects).await;
            (StatusCode::OK, Json(json!({ "ok": true, "restartRequired": restart })))
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({ "ok": false, "error": e })),
        ),
    }
}

/// mock hook（docs/agent-loop.md §Mock Hook 契约）
#[derive(Deserialize)]
struct HookBody {
    event: String,
    instance: String,
    project: Option<String>,
    content: Option<String>,
    /// 模拟真实 Stop hook 自带字段（concepts §9b）；content 缺省时作为回退
    last_assistant_message: Option<String>,
}

async fn post_hook(State(s): State<Arc<AppState>>, Json(body): Json<HookBody>) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    // content 模拟「Overseer 读 Terminal Content」；读不到时回退 hook 自带的 last_assistant_message
    let content = body
        .content
        .or(body.last_assistant_message)
        .unwrap_or_default();
    let pending = *s.pending_notifications.lock().await;
    let effects = match ov
        .handle_hook(
            &body.event,
            &body.instance,
            body.project.as_deref().unwrap_or(""),
            &content,
            now_ms(),
            pending,
        )
        .await
    {
        Ok(e) => e,
        Err(err) => return err_response(err),
    };
    drop(ov);
    broadcast_effects(&s, effects).await;
    Json(json!({ "ok": true })).into_response()
}

/// debug：注入「终端当前显示什么」（MockTerminals，docs/timer.md §Scanner）
#[derive(Deserialize)]
struct DebugTerminalBody {
    instance: String,
    content: String,
}

async fn post_debug_terminal(
    State(s): State<Arc<AppState>>,
    Json(body): Json<DebugTerminalBody>,
) -> impl IntoResponse {
    s.mock_terminals
        .lock()
        .unwrap()
        .insert(body.instance, body.content);
    Json(json!({ "ok": true })).into_response()
}

/// Timer 后台任务（docs/timer.md）：tick → due → scan → Substantive 才触发
pub fn spawn_timer_task(s: Arc<AppState>, tick_ms: u64, batch: usize) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(tick_ms));
        loop {
            interval.tick().await;
            let due = {
                s.overseer
                    .lock()
                    .await
                    .due_timer_scans(now_ms(), batch)
            };
            for inst in due {
                // 读通道统一走 Overseer.terminal_reader（docs/sidecar.md：sidecar → MockTerminals → 跳过）
                let content = {
                    let ov = s.overseer.lock().await;
                    ov.terminal_reader.as_ref().and_then(|r| r(&inst))
                };
                if let Some(content) = content {
                    let pending = *s.pending_notifications.lock().await;
                    let result = {
                        s.overseer
                            .lock()
                            .await
                            .handle_timer_scan(&inst, &content, now_ms(), pending)
                            .await
                    };
                    match result {
                        Ok(effects) => broadcast_effects(&s, effects).await,
                        Err(err) => eprintln!("timer scan {inst}: {err}"),
                    }
                } else {
                    // tab 消亡检测（docs/storage.md closed 终态）：
                    // 仅 sidecar 在链时 None 才是消亡证据；纯 MockTerminals 的 None 只是未注入
                    let mut ov = s.overseer.lock().await;
                    if ov.sidecar_enabled {
                        if let Err(err) = ov.mark_instance_closed(&inst, now_ms()) {
                            eprintln!("mark closed {inst}: {err}");
                        }
                    }
                }
            }
        }
    });
}

async fn ws_upgrade(ws: WebSocketUpgrade, State(s): State<Arc<AppState>>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| ws_loop(socket, s))
}

async fn ws_loop(mut socket: WebSocket, s: Arc<AppState>) {
    // 连接即推当前 top_state
    let st = state_json(&s).await;
    if socket
        .send(Message::Text(
            json!({ "kind": "top_state", "state": st }).to_string().into(),
        ))
        .await
        .is_err()
    {
        return;
    }
    let mut rx = s.tx.subscribe();
    while let Ok(msg) = rx.recv().await {
        if socket.send(Message::Text(msg.into())).await.is_err() {
            break;
        }
    }
}

fn err_response(err: std::io::Error) -> axum::response::Response {
    (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response()
}
