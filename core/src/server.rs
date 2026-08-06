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
    /// 外部自动载入的最近错误（docs/config.md §外部文件自动载入）：
    /// 文件被移动/删除或加载失败时保持 live Config，错误在此暴露给反射 Config UI
    config_error: Mutex<Option<String>>,
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
            config_error: Mutex::new(None),
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
    /// 外部自动载入的最近错误（反射 Config UI 显示用）
    pub async fn config_error(&self) -> Option<String> { self.config_error.lock().await.clone() }
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
    json!({ "kaomoji": cfg.kaomoji, "setAutonomyDefaultTtlMs": cfg.set_autonomy_default_ttl_ms, "viewScale": cfg.view_scale, "badgeStyle": cfg.badge_style, "badgeSide": cfg.badge_side, "theme": cfg.theme, "themes": cfg.themes, "uiLanguage": cfg.ui_language, "name": cfg.name })
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
        .route("/effect", post(post_effect))
        .route("/cards", get(get_cards))
        .route("/cards/layout", post(post_card_layout))
        .route("/cards/user_closed", post(post_card_user_closed))
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
    let app = app
        .route("/debug/terminal", post(crate::mock::post_debug_terminal))
        .route("/debug/effect", post(crate::mock::post_debug_effect));
    app.layer(tower_http::cors::CorsLayer::permissive()).with_state(state_for_ws)
}

/// Tauri 模式薄 router：仅 `/hook`
pub fn hook_router(state: Arc<AppState>) -> Router {
    Router::new().route("/hook", post(post_hook)).with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::DebugAgent;

    /// #25 观测链路端到端（HTTP 层）：POST /effect → effect.jsonl 落盘，
    /// 同 id 两条 window_opened 中间无 window_closed = 重复窗口的证据形态被捕获
    #[tokio::test]
    async fn post_effect_records_frontend_action() {
        let dir = std::env::temp_dir().join(format!("overseer-test-server-eff-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let harness = crate::Harness::load(&dir, &dir, 100_000, 0).unwrap();
        let ov = OverseerBackend::new(harness, crate::Config::default(), LlmBackend::Debug(DebugAgent::silent()));
        let state = Arc::new(AppState::new(ov, Default::default()));
        let (ws_tx, _) = tokio::sync::broadcast::channel(4);
        let app = router(state, ws_tx);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let client = reqwest::Client::new();
        let url = format!("http://127.0.0.1:{port}/effect");
        for _ in 0..2 {
            let r = client
                .post(&url)
                .json(&json!({"kind":"window_opened","payload":{"window":"card-demo"}}))
                .send()
                .await
                .unwrap();
            assert!(r.status().is_success());
        }
        let raw = std::fs::read_to_string(dir.join(crate::EFFECT_FILE)).unwrap();
        let opened = raw.matches("\"kind\":\"window_opened\"").count();
        assert_eq!(opened, 2, "两条 window_opened 落盘: {raw}");
        assert!(raw.contains("\"origin\":\"frontend\""));
        assert!(!raw.contains("window_closed"), "无 closed——#25 证据形态成立");
        let _ = std::fs::remove_dir_all(&dir);
    }
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
    // 动作流记录（docs/effect-reporting.md §kind）：前端 push_event = interaction/frontend
    ov.record_frontend_effect(
        "interaction",
        json!({ "desc": body.desc.as_str(), "card_id": body.card_id.as_deref() }),
    );
    if body.desc.starts_with("用户关闭了") { *s.pending_notifications.lock().await = s.pending_notifications.lock().await.saturating_sub(1); }
    // 用户 × 关卡：dismiss（删 .card.json、出注册表、忘记布局）+ closed_by_user 双行事件
    if let Some(cid) = body.card_id.as_deref() {
        let ts = now_ms();
        if let Some(entry) = ov.harness.cards_remove(cid) {
            let lc = crate::lifecycle::DefaultLifecycle::for_lang(crate::i18n::Lang::of(&ov.config.harness_language));
            ov.harness.event_buffer.push(lc.user_close_line(&entry.meta));
            let alive = ov.harness.cards.len();
            ov.harness.event_buffer.push(lc.closed_line(&entry.meta, alive, ts));
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

/// Card 跨重启恢复（readonly 查询，docs/components.md §Card 文件）：
/// 与 Tauri command list_cards 同一 core 逻辑（双运输层共享）
async fn get_cards(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    let cards_dir = ov.harness.cards_dir();
    let mut out = vec![];
    for (id, e) in &ov.harness.cards {
        if let Some(component) = crate::cards::read_component(&cards_dir, id) {
            out.push(json!({
                "component": component,
                "user_closed": e.user_closed,
                "layout": e.layout,
            }));
        }
    }
    Json(json!(out))
}

#[derive(Deserialize)]
struct CardLayoutBody {
    id: String,
    offset: (i64, i64),
}

/// Card 布局回写（docs/components.md §Card 文件）：与 update_card_layout 同一 core 逻辑
async fn post_card_layout(State(s): State<Arc<AppState>>, Json(body): Json<CardLayoutBody>) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    match ov.harness.cards_write_layout(&body.id, body.offset) {
        Ok(()) => {
            ov.record_frontend_effect("card_layout", json!({ "id": body.id.as_str(), "manual": true }));
            Json(json!({ "ok": true }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

#[derive(Deserialize)]
struct CardUserClosedBody {
    id: String,
    user_closed: bool,
}

/// Card 显示选择回写（Cards Shelf 显隐切换）：与 set_card_user_closed 同一 core 逻辑
async fn post_card_user_closed(State(s): State<Arc<AppState>>, Json(body): Json<CardUserClosedBody>) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    match ov.harness.cards_write_user_closed(&body.id, body.user_closed) {
        Ok(()) => {
            ov.record_frontend_effect(
                "card_visibility",
                json!({ "id": body.id.as_str(), "user_closed": body.user_closed }),
            );
            Json(json!({ "ok": true }))
        }
        Err(e) => Json(json!({ "ok": false, "error": e })),
    }
}

async fn get_config_schema(State(s): State<Arc<AppState>>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    let restart = ov.restart_required();
    let load_error = s.config_error.lock().await.clone();
    Json(json!({
        "version": crate::config::migrate::CURRENT_VERSION,
        "readOnly": ov.config.read_only,
        "restartRequired": restart,
        "loadError": load_error,
        "nodes": crate::config::reflect::config_nodes(&ov.config),
    }))
}

/// ConfigOutcome 应用收尾（统一管道热应用，docs/config.md §统一修改入口）：
/// llm_changed → 重建 LlmBackend 注入（热字段立即生效）；effects 广播。
pub async fn finish_config_outcome(s: &Arc<AppState>, outcome: crate::overseer::ConfigOutcome) {
    if outcome.llm_changed {
        let llm_cfg = { s.overseer.lock().await.config.llm.clone() };
        let backend = LlmBackend::from_config(&llm_cfg);
        s.overseer.lock().await.replace_llm(backend);
    }
    for e in outcome.effects {
        s.broadcast_effect_json(effect_json(&e)).await;
    }
}

/// 外部文件自动载入（docs/config.md §外部文件自动载入）：轮询 config.json。
/// - 成功：与一次全文 update 完全相同的管线与热应用；冷字段 pending 按启动快照发散重算
/// - 文件被移动/删除、读取/解析/校验失败：保持 live Config 不变，错误暴露给反射 UI；
///   不自动重建默认文件或写回，后续检测到文件修复或重新出现时自动重试
pub fn spawn_config_watcher(s: Arc<AppState>, dir: std::path::PathBuf) {
    tokio::spawn(async move {
        let file = dir.join(crate::config::CONFIG_FILE);
        let stamp = || {
            std::fs::metadata(&file)
                .ok()
                .and_then(|m| m.modified().ok().map(|t| (t, m.len())))
        };
        let mut last = stamp(); // 启动基线：不因启动本身触发重载
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(2000));
        loop {
            interval.tick().await;
            let cur = stamp();
            if cur == last {
                continue;
            }
            last = cur;
            match crate::config::migrate::preview(&dir) {
                Ok(new_cfg) => {
                    let mut ov = s.overseer.lock().await;
                    if ov.config == new_cfg {
                        // 内容无实际变化（mtime 抖动）；清错误状态即可
                        let had_err = s.config_error.lock().await.take().is_some();
                        drop(ov);
                        if had_err {
                            s.broadcast_effect_json(json!({ "kind": "config" })).await;
                        }
                        continue;
                    }
                    let llm_changed = ov.apply_external_config(new_cfg);
                    *s.config_error.lock().await = None;
                    drop(ov);
                    if llm_changed {
                        let llm_cfg = { s.overseer.lock().await.config.llm.clone() };
                        let backend = LlmBackend::from_config(&llm_cfg);
                        s.overseer.lock().await.replace_llm(backend);
                    }
                    s.broadcast_effect_json(json!({ "kind": "config" })).await;
                }
                Err(e) => {
                    eprintln!("[config] 外部载入失败：{e}");
                    *s.config_error.lock().await = Some(e);
                    s.broadcast_effect_json(json!({ "kind": "config" })).await;
                }
            }
        }
    });
}

#[derive(Deserialize)]
struct SetConfigBody { path: String, value: Value }

async fn post_config(State(s): State<Arc<AppState>>, Json(body): Json<SetConfigBody>) -> impl IntoResponse {
    let mut ov = s.overseer.lock().await;
    match ov.apply_config_by_path(&body.path, body.value) {
        Ok(outcome) => {
            // 动作流记录（docs/effect-reporting.md §kind）：前端设置面板 = config_update/frontend
            ov.record_frontend_effect("config_update", json!({ "path": body.path }));
            let restart = outcome.restart_required.clone();
            drop(ov);
            finish_config_outcome(&s, outcome).await;
            (StatusCode::OK, Json(json!({ "ok": true, "restartRequired": restart })))
        }
        Err(e) => (StatusCode::BAD_REQUEST, Json(json!({ "ok": false, "error": e }))),
    }
}

/// 前端非 readonly @tauri-apps/api 调用上报（docs/effect-reporting.md §通道）
#[derive(Deserialize)]
struct EffectBody {
    kind: String,
    #[serde(default)]
    payload: Value,
}

async fn post_effect(State(s): State<Arc<AppState>>, Json(body): Json<EffectBody>) -> impl IntoResponse {
    let ov = s.overseer.lock().await;
    ov.record_frontend_effect(&body.kind, body.payload);
    Json(json!({ "ok": true }))
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

/// Cron 调度任务（concepts §10g，docs/cron.md §调度实现）：
/// 每 500ms 轮询——① waiters 到点唤醒（共享句柄，不经 overseer 锁：sleep 持
/// Queue 串行点等待时无死锁）；② entries due → message 作 system 输入入 Queue
/// （与 hook 同构，唤醒单消费者）。
pub fn spawn_cron_task(s: Arc<AppState>) {
    tokio::spawn(async move {
        let waiters = { s.overseer.lock().await.harness.cron.waiter_handle() };
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(500));
        loop {
            interval.tick().await;
            let now = now_ms();
            // ① sleep waiters（锁外句柄）
            waiters.fire_due(now);
            // ② 持久化计划到期 → 入 Queue
            let messages = {
                let mut ov = s.overseer.lock().await;
                match ov.harness.cron.due(now) {
                    Ok(m) => m,
                    Err(e) => {
                        eprintln!("[cron] due 失败：{e}");
                        vec![]
                    }
                }
            };
            for message in messages {
                let mut ov = s.overseer.lock().await;
                if let Err(e) = ov.enqueue(crate::context::Role::System, message, now) {
                    eprintln!("[cron] 到期入队失败：{e}");
                    continue;
                }
                drop(ov);
                s.queue_notify.notify_one();
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
