//! HTTP server（docs/core-server.md）：
//! - Tauri 模式：仅 `/hook`（前端走 Tauri IPC）
//! - debug 模式：完整 HTTP+WS（浏览器前端）

use axum::{
    extract::ws::Message,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::llm::LlmBackend;
use crate::lifecycle::Lifecycle;
use crate::overseer::{Effect, OverseerBackend};
use crate::context::Role;
use crate::Config;

pub type EffectSender = Box<dyn Fn(Value) + Send + Sync>;

pub struct AppState {
    overseer: Mutex<OverseerBackend<LlmBackend>>,
    pending_notifications: Mutex<usize>,
    mock_terminals: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    send: Mutex<Option<EffectSender>>,
    /// Queue 放行信号（concepts §10c）：生产者入队后唤醒单消费者
    pub queue_notify: tokio::sync::Notify,
}

impl AppState {
    pub fn new(
        overseer: OverseerBackend<LlmBackend>,
        mock_terminals: Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    ) -> Self {
        Self {
            overseer: Mutex::new(overseer),
            pending_notifications: Mutex::new(0),
            mock_terminals,
            send: Mutex::new(None),
            queue_notify: tokio::sync::Notify::new(),
        }
    }

    pub async fn set_sender(&self, send: EffectSender) { *self.send.lock().await = Some(send); }
    pub async fn broadcast_effect_json(&self, msg: Value) {
        if let Some(send) = self.send.lock().await.as_ref() { send(msg); }
    }
    /// 流式 delta 旁路接线（docs/streaming.md）：OverseerBackend.effect_sink →
    /// effect 通道广播（Tauri emit / WS 由 sender 双发）。Weak 防循环引用。
    pub async fn wire_effect_sink(self: &Arc<AppState>) {
        let weak = Arc::downgrade(self);
        let mut ov = self.overseer.lock().await;
        ov.effect_sink = Some(Arc::new(move |e: &Effect| {
            if let Some(s) = weak.upgrade() {
                let msg = effect_json(e);
                tokio::spawn(async move { s.broadcast_effect_json(msg).await; });
            }
        }));
    }
    pub(crate) fn mock_terminals(&self) -> &Arc<std::sync::Mutex<std::collections::HashMap<String, String>>> { &self.mock_terminals }
    pub fn overseer(&self) -> &Mutex<OverseerBackend<LlmBackend>> { &self.overseer }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

pub fn effect_json(e: &Effect) -> Value {
    match e {
        Effect::RenderComponent(spec) => json!({ "kind": "render_component", "spec": spec }),
        Effect::CloseComponent(id) => json!({ "kind": "close_component", "id": id }),
        Effect::SetAutonomy { face, motion, ttl_ms, once } => json!({ "kind": "set_autonomy", "face": face, "motion": motion, "ttlMs": ttl_ms, "once": once }),
        Effect::ConfigChanged { .. } => json!({ "kind": "config" }),
        Effect::AssistantDelta { content, reasoning_content } => json!({ "kind": "assistant_delta", "content": content, "reasoning_content": reasoning_content }),
        Effect::AssistantDone => json!({ "kind": "assistant_done" }),
    }
}

fn config_json(cfg: &Config) -> Value {
    json!({ "kaomoji": cfg.kaomoji, "setAutonomyDefaultTtlMs": cfg.set_autonomy_default_ttl_ms, "viewScale": cfg.view_scale, "badgeStyle": cfg.badge_style, "badgeSide": cfg.badge_side })
}

async fn state_json_value(s: &AppState) -> Value {
    let ov = s.overseer.lock().await;
    let pending = *s.pending_notifications.lock().await;
    json!({ "instances": ov.harness.agents.iter().map(|a| json!({"id":a.hash,"name":a.name,"status":a.status})).collect::<Vec<_>>(), "pendingNotifications": pending })
}

/// 完整 router（debug 模式：浏览器前端需要 HTTP+WS）
pub fn router(state: Arc<AppState>, ws_tx: tokio::sync::broadcast::Sender<String>) -> Router {
    use axum::extract::ws::WebSocketUpgrade;
    let ws_tx_clone = ws_tx.clone();
    let state_for_ws = state.clone();

    let app = Router::new()
        .route("/state", get(get_state))
        .route("/context", get(get_context))
        .route("/queue/user", post(post_user))
        .route("/events", post(post_event))
        .route("/config", get(get_config))
        .route("/config/schema", get(get_config_schema))
        .route("/config", post(post_config))
        .route("/hook", post(post_hook))
        .route("/ws", get(move |ws: WebSocketUpgrade, State(s): State<Arc<AppState>>| {
            let tx = ws_tx_clone.clone();
            async move {
                ws.on_upgrade(move |mut socket| async move {
                    let st = state_json_value(&s).await;
                    if socket.send(Message::Text(json!({"kind":"top_state","state":st}).to_string().into())).await.is_err() { return; }
                    let mut rx = tx.subscribe();
                    while let Ok(msg) = rx.recv().await {
                        if socket.send(Message::Text(msg.into())).await.is_err() { break; }
                    }
                })
            }
        }));
    #[cfg(feature = "mock")]
    let app = app.route("/debug/terminal", post(crate::mock::post_debug_terminal));
    app.layer(tower_http::cors::CorsLayer::permissive()).with_state(state_for_ws)
}

/// Tauri 模式薄 router：仅 `/hook`
pub fn hook_router(state: Arc<AppState>) -> Router {
    Router::new().route("/hook", post(post_hook)).with_state(state)
}

// ── handlers ──

async fn get_state(State(s): State<Arc<AppState>>) -> impl IntoResponse { Json(state_json_value(&s).await) }

async fn get_context(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    Json(json!(ov.harness.context.messages()))
}

#[derive(Deserialize)]
struct UserBody { text: String }

async fn post_user(State(s): State<Arc<AppState>>, Json(body): Json<UserBody>) -> impl IntoResponse {
    *s.pending_notifications.lock().await = 0;
    {
        let mut ov = s.overseer.lock().await;
        if let Err(err) = ov.enqueue(Role::User, body.text, now_ms()) {
            return err_response(err);
        }
    }
    // 生产者只入队，放行由消费者任务串行驱动（concepts §10c）
    s.queue_notify.notify_one();
    Json(json!({ "ok": true })).into_response()
}

#[derive(Deserialize)]
struct EventBody {
    desc: String,
    /// 用户 × 关闭卡片时的 card id（生命周期 closed_by_user 双行事件，docs/components.md）
    card_id: Option<String>,
    /// 结构化状态快照（双载荷，todobox 交互附带，docs/harness.md）
    state: Option<Value>,
}

async fn post_event(State(s): State<Arc<AppState>>, Json(body): Json<EventBody>) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    if body.desc.starts_with("用户关闭了") { *s.pending_notifications.lock().await = s.pending_notifications.lock().await.saturating_sub(1); }
    // 用户 × 关卡：closed_by_user 双行事件（自然语言 + 生命周期行，docs/components.md）
    if let Some(cid) = body.card_id.as_deref() {
        let ts = now_ms();
        if let Some(meta) = ov.cards.remove(cid) {
            let lc = crate::lifecycle::DefaultLifecycle;
            ov.harness.event_buffer.push(lc.user_close_line(&meta));
            let alive = ov.cards.len();
            ov.harness.event_buffer.push(lc.closed_line(&meta, alive, ts));
            return Json(json!({ "ok": true }));
        }
    }
    // 双载荷（docs/harness.md）：带快照走 push_with_state，否则普通描述
    match body.state {
        Some(state) => ov.harness.event_buffer.push_with_state(body.desc, state),
        None => ov.harness.event_buffer.push(body.desc),
    }
    Json(json!({ "ok": true }))
}

async fn get_config(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    Json(config_json(&ov.config))
}

async fn get_config_schema(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    Json(json!({ "version": crate::config::migrate::CURRENT_VERSION, "readOnly": ov.config.read_only, "nodes": crate::config::reflect::config_nodes(&ov.config) }))
}

#[derive(Deserialize)]
struct SetConfigBody { path: String, value: Value }

async fn post_config(State(s): State<Arc<AppState>>, Json(body): Json<SetConfigBody>) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    match ov.apply_config_by_path(&body.path, body.value) {
        Ok(outcome) => {
            let restart = outcome.restart_required.clone();
            drop(ov);
            for e in outcome.effects { s.broadcast_effect_json(effect_json(&e)).await; }
            (StatusCode::OK, Json(json!({ "ok": true, "restartRequired": restart })))
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e }))),
    }
}

#[derive(Deserialize)]
struct HookBody {
    event: String,
    session_id: Option<String>, cwd: Option<String>, kind: Option<String>,
    prompt: Option<String>, message: Option<String>,
    instance: Option<String>, project: Option<String>, content: Option<String>,
    last_assistant_message: Option<String>,
}

async fn post_hook(State(s): State<Arc<AppState>>, Json(body): Json<HookBody>) -> impl IntoResponse {
    {
        let mut ov = s.overseer.lock().await;
        let result = if let Some(session_id) = body.session_id.as_deref() {
            ov.handle_real_hook(&body.event, session_id, body.cwd.as_deref().unwrap_or(""), body.kind.as_deref(), body.prompt.as_deref(), body.message.as_deref(), body.last_assistant_message.as_deref(), now_ms()).await
        } else {
            let content = body.content.or(body.last_assistant_message).unwrap_or_default();
            ov.handle_hook(&body.event, body.instance.as_deref().unwrap_or(""), body.project.as_deref().unwrap_or(""), &content, now_ms()).await
        };
        if let Err(err) = result {
            return err_response(err);
        }
    }
    // hook 只当触发信号：入队后唤醒消费者，不等 LLM 轮次（fire-and-forget 友好）
    s.queue_notify.notify_one();
    Json(json!({ "ok": true })).into_response()
}

/// Timer 后台任务（docs/timer.md）
pub fn spawn_timer_task(s: Arc<AppState>, tick_ms: u64, batch: usize) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(tick_ms));
        loop {
            interval.tick().await;
            let due = { s.overseer.lock().await.due_timer_scans(now_ms(), batch) };
            for inst in due {
                let content = { let ov = s.overseer.lock().await; ov.terminal_reader.as_ref().and_then(|r| r(&inst)) };
                if let Some(content) = content {
                    let result = { s.overseer.lock().await.handle_timer_scan(&inst, &content, now_ms()).await };
                    match result {
                        Ok(()) => s.queue_notify.notify_one(),
                        Err(err) => eprintln!("timer scan {inst}: {err}"),
                    }
                } else {
                    let mut ov = s.overseer.lock().await;
                    if ov.sidecar_enabled { if let Err(err) = ov.mark_instance_closed(&inst, now_ms()) { eprintln!("mark closed {inst}: {err}"); } }
                }
            }
        }
    });
}

/// Queue 单消费者（concepts §10c 串行放行）：唤醒后逐条放行——
/// 放行一条 → Context 写输入 → run_trigger（LLM 一轮）→ 广播副作用 → 放行下一条。
/// 一轮一条地持锁：生产者在轮次之间可继续入队，不等整个积压清空。
pub fn spawn_queue_consumer(s: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            s.queue_notify.notified().await;
            loop {
                let mut ov = s.overseer.lock().await;
                let Some(input) = ov.harness.queue.release() else {
                    drop(ov);
                    break;
                };
                let pending = *s.pending_notifications.lock().await;
                let effects = match ov.release_one(input, pending).await {
                    Ok(e) => e,
                    Err(err) => {
                        eprintln!("queue release: {err}");
                        vec![]
                    }
                };
                drop(ov);
                for e in &effects {
                    if matches!(e, Effect::RenderComponent(_)) { *s.pending_notifications.lock().await += 1; }
                    s.broadcast_effect_json(effect_json(e)).await;
                }
                s.broadcast_effect_json(json!({ "kind": "context_changed" })).await;
                s.broadcast_effect_json(json!({ "kind": "top_state", "state": state_json_value(&s).await })).await;
            }
        }
    });
}

fn err_response(err: std::io::Error) -> axum::response::Response { (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response() }
