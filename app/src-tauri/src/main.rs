//! Tauri 壳：三独立窗口（pet + chat + menu）+ 内嵌 ambery-core。
//! 前端通信走 Tauri IPC，仅 /hook 保留 HTTP。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use ambery_core::llm::LlmBackend;
use ambery_core::ambery::AmberyBackend;
use ambery_core::context::Role;
use ambery_core::server::{now_ms, hook_router, spawn_queue_consumer, spawn_timer_task, AppState};
use ambery_core::{Config, Harness};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;

mod menu_window;
mod window;
mod tray;
mod tauri_runtime_actions;

/// 面板底部按钮（原托盘菜单动作）。复合入口逐动作转发、逐动作记录
/// （四个动作四条 effect，不能合成一条 toggle）
#[tauri::command]
fn toggle_pet<R: tauri::Runtime>(app: tauri::AppHandle<R>) {
    if let Some(w) = app.get_webview_window("pet") {
        if w.is_visible().unwrap_or(false) {
            tauri_runtime_actions::hide_window(&app, "pet");
            tauri_runtime_actions::hide_window(&app, "chat");
            tauri_runtime_actions::emit_event(&app, "cards:hide", json!(()));
            tauri_runtime_actions::emit_event(&app, "pet:hidden", json!(()));
        } else {
            tauri_runtime_actions::show_window(&app, "pet");
            tauri_runtime_actions::emit_event(&app, "pet:shown", json!(()));
        }
    }
}

#[tauri::command]
fn quit_app(app: tauri::AppHandle) {
    app.exit(0);
}

// ── Tauri IPC commands（替代原 HTTP/WS 路由）──

struct TauriState(pub std::sync::Mutex<Option<Arc<AppState>>>);
type SharedTauriState = Arc<TauriState>;

fn wait_state(ts: &TauriState) -> Result<Arc<AppState>, String> {
    for _ in 0..50 {
        if let Some(s) = ts.0.lock().unwrap().clone() { return Ok(s); }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    ts.0.lock().unwrap().clone().ok_or("not ready".into())
}

#[tauri::command]
async fn get_state(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    Ok(s.state_json().await)
}

#[tauri::command]
async fn get_context(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.ambery().lock().await;
    Ok(json!(ov.harness.context.messages()))
}

#[tauri::command]
async fn append_user(state: tauri::State<'_, SharedTauriState>, text: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    {
        let mut ov = s.ambery().lock().await;
        ov.enqueue(Role::User, text, ambery_core::queue::QueueSource::UserChat, now_ms()).map_err(|e| e.to_string())?;
    }
    // 生产者只入队，放行由消费者任务驱动
    s.queue_notify.notify_one();
    Ok(json!({ "ok": true }))
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn push_event(
    state: tauri::State<'_, SharedTauriState>,
    action: String,
    card_id: Option<String>,
    card_type: Option<String>,
    title: Option<String>,
    text: Option<String>,
    target: Option<String>,
    checked: Option<bool>,
    state_snapshot: Option<Value>,
) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.ambery().lock().await;
    // 结构化事实 → 文本（lifecycle 语义单源）
    let ev = json!({
        "action": action,
        "card_type": card_type,
        "title": title,
        "text": text,
        "target": target,
        "checked": checked,
    });
    let desc = ambery_core::lifecycle::user_action_desc(
        ambery_core::i18n::Lang::of(&ov.config.harness_language),
        &ev,
    );
    // 动作流记录：前端 push_event = interaction/frontend
    ov.record_frontend_effect("interaction", json!({ "desc": desc.as_str(), "card_id": card_id.as_deref() }));
    // 用户 × 关卡：dismiss（删 .card.json、出注册表、忘记布局）+ closed_by_user 双行事件
    if action == "dismiss" {
        if let Some(cid) = card_id.as_deref() {
            let ts = now_ms();
            if let Some(entry) = ov.harness.cards_remove(cid) {
                let lc = ambery_core::lifecycle::DefaultLifecycle::for_lang(
                    ambery_core::i18n::Lang::of(&ov.config.harness_language),
                );
                use ambery_core::lifecycle::Lifecycle;
                ov.harness.event_buffer.push(lc.user_close_line(&entry.meta));
                let alive = ov.harness.cards.len();
                ov.harness.event_buffer.push(lc.closed_line(&entry.meta, alive, ts));
                return Ok(json!({ "ok": true }));
            }
        }
    }
    match state_snapshot {
        Some(st) => ov.harness.event_buffer.push_with_state(desc, st),
        None => ov.harness.event_buffer.push(desc),
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    eprintln!("[tauri-cmd] get_config called");
    let s = wait_state(&state)?;
    let ov = s.ambery().lock().await;
    let cfg = &ov.config;
    Ok(json!({ "kaomoji": cfg.kaomoji, "setAutonomyDefaultTtlMs": cfg.set_autonomy_default_ttl_ms, "viewScale": cfg.view_scale, "badgeStyle": cfg.badge_style, "badgeSide": cfg.badge_side, "theme": cfg.theme, "themes": cfg.themes, "uiLanguage": cfg.ui_language, "name": cfg.name }))
}

/// 主题导出：写 `<config_root>/themes/<name>.theme.json`
#[tauri::command]
async fn export_theme(state: tauri::State<'_, SharedTauriState>, name: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.ambery().lock().await;
    let root = ov.harness.config_dir().to_path_buf();
    match ambery_core::config::theme::export_theme(&root, &ov.config, &name) {
        Ok(path) => {
            // 写文件副作用，端点记录
            ov.record_frontend_effect("theme_export", json!({ "name": name.as_str() }));
            Ok(json!({ "ok": true, "path": path.display().to_string() }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// 主题导入：版本检查 → 兼容变换 → 校验 →
/// 统一修改管道写入 themes.<name>（原子拒绝 + 广播 config_changed，全部窗口即切）
#[tauri::command]
async fn import_theme(state: tauri::State<'_, SharedTauriState>, file: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let root = {
        let ov = s.ambery().lock().await;
        ov.harness.config_dir().to_path_buf()
    };
    let (name, value) = match ambery_core::config::theme::import_theme(&root, &file) {
        Ok(r) => r,
        Err(e) => return Ok(json!({ "ok": false, "error": e })),
    };
    let path = format!("themes.{name}");
    let mut ov = s.ambery().lock().await;
    match ov.apply_config_by_path(&path, value) {
        Ok(outcome) => {
            ov.record_frontend_effect("config_update", json!({ "path": path.as_str() }));
            drop(ov);
            ambery_core::server::finish_config_outcome(&s, outcome).await;
            Ok(json!({ "ok": true, "name": name }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

#[tauri::command]
async fn get_config_schema(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.ambery().lock().await;
    let restart = ov.restart_required();
    let load_error = s.config_error().await;
    // llm 初始化失败（active 指向损坏 provider）随 schema 拉取暴露——
    // 启动时事件通道未就绪 push 即丢，schema 是前端首启已用的拉取面（race-free）
    let llm_err = ambery_core::llm::LlmBackend::from_config(&ov.config.llm)
        .err()
        .map(|e| format!("LLM 配置损坏：{e}"));
    Ok(json!({
        "version": ambery_core::config::migrate::CURRENT_VERSION,
        "readOnly": ov.config.read_only,
        "restartRequired": restart,
        "loadError": load_error,
        "llmError": llm_err,
        "nodes": ambery_core::config::reflect::config_nodes(&ov.config),
    }))
}

/// 设置面板改值（对齐 server post_config：apply + 广播 + restartRequired + llm 重建）
#[tauri::command]
async fn set_config(state: tauri::State<'_, SharedTauriState>, path: String, value: Value) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.ambery().lock().await;
    match ov.apply_config_by_path(&path, value) {
        Ok(outcome) => {
            // 动作流记录：前端设置面板 = config_update/frontend
            ov.record_frontend_effect("config_update", json!({ "path": path.as_str() }));
            let restart = outcome.restart_required.clone();
            drop(ov);
            ambery_core::server::finish_config_outcome(&s, outcome).await;
            Ok(json!({ "ok": true, "restartRequired": restart }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// LLM 连通测试：按当前 active provider 构建并调用一次，返回成功或具体失败原因
/// （env 未设 / 401 / 超时 / 网络 / provider 缺失）。
#[tauri::command]
async fn test_llm(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let cfg = {
        let ov = s.ambery().lock().await;
        ov.config.llm.clone()
    };
    match ambery_core::llm::test_llm(&cfg).await {
        Ok(reply) => Ok(json!({ "ok": true, "reply": reply })),
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// provider key 存在性状态（应用级 env 文件 → 进程环境，本地即时）
#[tauri::command]
async fn get_api_key_status(
    state: tauri::State<'_, SharedTauriState>,
    provider: String,
) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let cfg = {
        let ov = s.ambery().lock().await;
        ov.config.llm.clone()
    };
    let (set, source) = ambery_core::llm::api_key_status(&provider, &cfg);
    Ok(json!({ "ok": true, "set": set, "source": source }))
}

/// 写/清 provider key（形态乙）：Some upsert 进应用级 env 文件 + api_key_env 归一；
/// null 从 env 文件清除。写失败返回错误（前端内联报错，绝不静默）。
#[tauri::command]
async fn set_api_key(
    state: tauri::State<'_, SharedTauriState>,
    provider: String,
    key: Option<String>,
) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.ambery().lock().await;
    let provider_name = provider.clone();
    match ambery_core::llm::set_api_key(&provider, key.as_deref(), &mut ov.config.llm) {
        Ok(()) => {
            ov.record_frontend_effect("config_update", json!({ "path": format!("llm.providers.{provider_name}.api_key_env") }));
            let cfg_dir = ambery_core::paths::config_root();
            let _ = ov.config.save(&cfg_dir);
            // key 变化后重建 LlmBackend 换入——启动时构建的旧 backend 看不到新 key
            let new_llm = ambery_core::llm::LlmBackend::from_config(&ov.config.llm).unwrap_or_else(|err| {
                eprintln!("[llm] {err}——按无 LLM 态运行");
                LlmBackend::unavailable(err)
            });
            ov.replace_llm(new_llm);
            Ok(json!({ "ok": true }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// Card 跨重启恢复（readonly 查询）：
/// pet 启动 pull 全部存活卡片（component + _meta）；可见性过滤在前端（pull-on-ready，
/// 规避 push-at-startup 的 webview 未就绪时序漏洞）
#[tauri::command]
async fn list_cards(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.ambery().lock().await;
    let cards_dir = ov.harness.cards_dir();
    let mut out = vec![];
    for (id, e) in &ov.harness.cards {
        if let Some(component) = ambery_core::cards::read_component(&cards_dir, id) {
            out.push(json!({
                "component": component,
                "user_closed": e.user_closed,
                "layout": e.layout,
            }));
        }
    }
    Ok(json!(out))
}

/// Card 布局回写：拖拽结束把相对 pet 偏移落
/// .card.json（_meta.layout.offset/manual）。invoke 写动作，端点记录 card_layout
/// effect（core 接收端记录）
#[tauri::command]
async fn update_card_layout(state: tauri::State<'_, SharedTauriState>, id: String, offset: (i64, i64)) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.ambery().lock().await;
    match ov.harness.cards_write_layout(&id, offset) {
        Ok(()) => {
            ov.record_frontend_effect("card_layout", json!({ "id": id.as_str(), "manual": true }));
            Ok(json!({ "ok": true }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// Card 显示选择回写（Cards Shelf 显隐切换）：
/// 只改 _meta.user_closed（窗口动作由 pet 经 shelf:visibility 事件执行）；
/// invoke 写动作，端点记录 card_visibility effect
#[tauri::command]
async fn set_card_user_closed(state: tauri::State<'_, SharedTauriState>, id: String, user_closed: bool) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.ambery().lock().await;
    match ov.harness.cards_write_user_closed(&id, user_closed) {
        Ok(()) => {
            ov.record_frontend_effect(
                "card_visibility",
                json!({ "id": id.as_str(), "user_closed": user_closed }),
            );
            Ok(json!({ "ok": true }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// 前端非 readonly @tauri-apps/api 调用上报
#[tauri::command]
async fn record_effect(state: tauri::State<'_, SharedTauriState>, kind: String, payload: Option<Value>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.ambery().lock().await;
    ov.record_frontend_effect(&kind, payload.unwrap_or(Value::Null));
    Ok(json!({ "ok": true }))
}

/// card 窗口 id 合法性（与 core 同一约束）：
/// A-Z a-z 0-9 _ - . /，路径段不得为空或 `..`（嵌套子目录合法；core 接受壳不得拒绝）
fn valid_card_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        && !id.split('/').any(|seg| seg.is_empty() || seg == "..")
}

/// Card 窗口权威注册表（#25 断根）。
/// Tauri 注册表的移除走事件循环（`destroy()` 经 dispatcher 分发、event loop 处理时才出表），
/// 决策若读其瞬时视图仍有「将死窗口」窗口期（#25 根因 B 的残留形态）。
/// 本表是 create / reuse / close 决策的唯一依据：`Closing` 吸收 destroy 的生效窗口期——
/// close 等物理移除（或兜底超时）后才出表；ensure 见 `Closing` 等其出表再重建。
#[derive(Default)]
struct CardWindowRegistry(std::sync::Mutex<std::collections::HashMap<String, CardWinState>>);

#[derive(Clone, Copy, PartialEq, Eq)]
enum CardWinState {
    Alive,
    Closing,
}

/// 等 Tauri 注册表物理移除（destroy 经事件循环生效；MockRuntime 无事件循环时兜底超时）
async fn wait_window_gone<R: tauri::Runtime>(app: &tauri::AppHandle<R>, label: &str) {
    for _ in 0..50 {
        if app.get_webview_window(label).is_none() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    eprintln!("[card-win] wait_window_gone 超时（事件循环未处理 destroy？）: {label}");
}

/// 窗口决策上提（#25 断根）：
/// 前端不再 getByLabel 自查自决存在性——Rust 权威注册表同步决策 create / reuse。
/// create → window_opened 记录 + 500ms 后推 card:spec（等页面 JS listener 就绪）；
/// reuse → 立即重推 card:spec（原地更新），记录 event_emit。
#[tauri::command]
async fn ensure_card_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    registry: tauri::State<'_, CardWindowRegistry>,
    topmost: tauri::State<'_, window::TopmostRegistry>,
    state: tauri::State<'_, SharedTauriState>,
    id: String,
    spec: Value,
) -> Result<Value, String> {
    if !valid_card_id(&id) {
        return Ok(json!({ "result": "error", "error": format!("非法 card id: {id}") }));
    }
    let label = format!("card-{id}");
    let s = wait_state(&state)?;
    // 决策环：Closing → 等其出表（close 侧物理移除后才出表，出表即可安全重建）
    for _ in 0..100 {
        let st = registry.0.lock().unwrap().get(&label).copied();
        match st {
            None => break,
            Some(CardWinState::Alive) => {
                if let Some(w) = app.get_webview_window(&label) {
                    let _ = w.emit("card:spec", spec);
                    let ov = s.ambery().lock().await;
                    ov.record_frontend_effect("event_emit", json!({ "event": "card:spec", "target": label.as_str() }));
                    return Ok(json!({ "result": "reused" }));
                }
                // 表与 Tauri 注册表不一致（窗口意外消亡）：自愈清表转重建
                registry.0.lock().unwrap().remove(&label);
                break;
            }
            Some(CardWinState::Closing) => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        }
    }
    // 出表后 Tauri 注册表仍有残留（destroy 未被事件循环处理的极端）：先收尸再建
    if app.get_webview_window(&label).is_some() {
        if let Some(w) = app.get_webview_window(&label) {
            let _ = w.destroy();
        }
        wait_window_gone(&app, &label).await;
        topmost.stop(&label);
    }
    let card_mode = s.ambery().lock().await.config.ui.topmost.card;
    let win = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html#card".into()))
        .title(&label)
        .inner_size(520.0, 440.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(card_mode != ambery_core::config::TopmostMode::Off)
        .focused(false)
        .shadow(false)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    registry.0.lock().unwrap().insert(label.clone(), CardWinState::Alive);
    // 置顶模式统一出口：aggressive 档补 pin + 轮询线程
    window::apply_topmost(&win, card_mode, &topmost);
    {
        let ov = s.ambery().lock().await;
        ov.record_frontend_effect("window_opened", json!({ "window": label.as_str() }));
    }
    // 页面 JS listener 注册在 load 之后；沿用 500ms 经验延迟推 spec（窗已毁则 emit 静默失败）
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = win.emit("card:spec", spec);
    });
    Ok(json!({ "result": "opened" }))
}

/// 统一关闭：destroy 不经 onCloseRequested
/// （preventDefault 会留将死窗口，#25 根因 B 的机制）。标 Closing → destroy → 等物理
/// 移除 → 出表；ensure 侧见 Closing 等待，窗口期被本表完全吸收。
/// agent close / shelf dismiss / 用户 × 三条路径收口到本命令。
#[tauri::command]
async fn close_card_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    registry: tauri::State<'_, CardWindowRegistry>,
    topmost: tauri::State<'_, window::TopmostRegistry>,
    state: tauri::State<'_, SharedTauriState>,
    id: String,
) -> Result<Value, String> {
    let label = format!("card-{id}");
    {
        let mut m = registry.0.lock().unwrap();
        if !m.contains_key(&label) {
            return Ok(json!({ "result": "absent" }));
        }
        m.insert(label.clone(), CardWinState::Closing);
    }
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.destroy();
    }
    wait_window_gone(&app, &label).await;
    registry.0.lock().unwrap().remove(&label);
    topmost.stop(&label);
    let s = wait_state(&state)?;
    let ov = s.ambery().lock().await;
    ov.record_frontend_effect("window_closed", json!({ "window": label.as_str() }));
    Ok(json!({ "result": "closed" }))
}

/// 置顶模式统一应用：常驻三窗按各自档位 + 全部活卡窗按 card 档。
/// 启动初始化与 config 热更（effect kind=config）共用本出口。
fn apply_topmost_all(handle: &tauri::AppHandle, cfg: &ambery_core::config::TopmostConfig) {
    let registry = handle.state::<window::TopmostRegistry>();
    for (label, mode) in [("pet", cfg.pet), ("chat", cfg.chat), ("shelf", cfg.shelf)] {
        if let Some(w) = handle.get_webview_window(label) {
            window::apply_topmost(&w, mode, &registry);
        }
    }
    for (label, w) in handle.webview_windows() {
        if label.starts_with("card-") {
            window::apply_topmost(&w, cfg.card, &registry);
        }
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            toggle_pet, quit_app,
            get_state, get_context, append_user, push_event, get_config, get_config_schema, set_config,
            test_llm, get_api_key_status, set_api_key,
            record_effect, list_cards, update_card_layout, set_card_user_closed,
            ensure_card_window, close_card_window, export_theme, import_theme
        ])
        .manage(SharedTauriState::new(TauriState(std::sync::Mutex::new(None))))
        .manage(CardWindowRegistry::default())
        .manage(window::TopmostRegistry::default())
        .setup(|app| {
            let pet = app.get_webview_window("pet").expect("pet window");
            let chat = app.get_webview_window("chat").expect("chat window");
            let menu = app.get_webview_window("menu").expect("menu window");
            let shelf = app.get_webview_window("shelf").expect("shelf window");

            // 置顶模式初始应用：chat/shelf 默认 topmost——不再起轮询线程（WindowNotFound 噪音源根除）
            let topmost_cfg = Config::load_or_default(&ambery_core::paths::config_root()).ui.topmost;
            apply_topmost_all(app.handle(), &topmost_cfg);
            menu_window::init_menu_window(&menu);
            tray::init_tray(app.handle(), &pet)?;

            let handle = app.handle().clone();
            let state_mgr = app.state::<SharedTauriState>().inner().clone();

            tauri::async_runtime::spawn(run_core(handle, state_mgr));

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn run_core(handle: tauri::AppHandle, state_mgr: SharedTauriState) {
    let config = Config::load_or_default(&ambery_core::paths::config_root());
    let harness = Harness::load_with_lang(
        &ambery_core::paths::storage_dir(),
        &ambery_core::paths::config_root(),
        config.effective_compression_limit().unwrap_or(usize::MAX),
        now_ms(),
        ambery_core::i18n::Lang::of(&config.harness_language),
    ).expect("load harness");
        let backend = LlmBackend::from_config(&config.llm)
            .unwrap_or_else(|err| LlmBackend::unavailable(err));
    let (timer_tick, timer_batch) = (config.timer.tick_ms, config.timer.batch);
    let mut ambery = AmberyBackend::new(harness, config, backend);

    // Terminal Adapter 装配：adapter_wt 开关门控（冷字段，
    // 装配期生效）——false = wt sidecar 完全不接入（无定位/读取/原语/启动扫描），
    // Hook 驱动核心体验仍可用
    let sidecar = if ambery.config.terminal.adapter_wt {
        ambery_terminal_wt::sidecar_exe()
            .map(ambery_terminal_wt::SidecarClient::new)
            .map(Arc::new)
    } else {
        None
    };
    let sidecar_for_sweep = sidecar.clone();

    {
        use ambery_core::terminal::{Composite, TerminalAdapter};
        use ambery_terminal_wt::{SidecarPlatformPrimitives, WtAdapter};
        use ambery_terminal_zellij::{ProcessZellijRunner, ZellijAdapter};
        let mut adapters: Vec<Arc<dyn TerminalAdapter>> = vec![];
        if let Some(sc) = &sidecar {
            adapters.push(Arc::new(WtAdapter::new(sc.clone())));
        }
        if ambery.config.terminal.adapter_zellij {
            adapters.push(Arc::new(ZellijAdapter::new(Arc::new(ProcessZellijRunner))));
        }
        if !adapters.is_empty() {
            ambery.terminal = Some(Arc::new(Composite::new(adapters)));
        }
        if let Some(sc) = &sidecar {
            ambery.primitives = Some(Arc::new(SidecarPlatformPrimitives::new(sc.clone())));
        }
    }

    let state = Arc::new(AppState::new(ambery));

    // 启动扫描
    if let Some(sc) = sidecar_for_sweep.clone() {
        let mut ov = state.ambery().lock().await;
        if let Err(e) = ov.startup_sweep(&move |req| sc.call(req), now_ms()).await {
            eprintln!("startup sweep: {e}");
        }
    }

    spawn_timer_task(state.clone(), timer_tick, timer_batch);
    spawn_queue_consumer(state.clone());
    // 外部文件自动载入
    ambery_core::server::spawn_config_watcher(state.clone(), ambery_core::paths::config_root());
    // Cron 调度任务
    ambery_core::server::spawn_cron_task(state.clone());

    // 注入 Tauri managed state
    *state_mgr.0.lock().unwrap() = Some(state.clone());

    // effects 推送：WS（浏览器 debug 兼容期）+ Tauri 原生事件（#9.5 emit 链路接通）
    let (tx, _) = tokio::sync::broadcast::channel(64);
    {
        let tx = tx.clone();
        let handle = handle.clone();
        state
            .set_sender(Box::new(move |msg: Value| {
                // 置顶模式热应用——config 变更已先落盘（统一管道先 persist 后广播），重读即最新
                if msg.get("kind").and_then(Value::as_str) == Some("config") {
                    let cfg = Config::load_or_default(&ambery_core::paths::config_root());
                    apply_topmost_all(&handle, &cfg.ui.topmost);
                }
                let _ = tx.send(msg.to_string());
                let _ = handle.emit("effect", msg);
            }))
            .await;
    }
    // 流式 delta 旁路
    state.wire_effect_sink().await;
    // Tauri 模式：前端走 IPC（TauriBridge），HTTP 仅留 /hook（外部 hook 脚本，进程外不可走 command）
    let app = hook_router(state);
    let port = match hook_port_value(std::env::var("AMBERY_PORT").ok().as_deref()) {
        Ok(port) => port,
        Err(err) => {
            eprintln!("[ambery-core] {err}");
            std::process::exit(1);
        }
    };
    let listener = match bind_hook_listener(port).await {
        Ok(listener) => listener,
        Err(err) => {
            eprintln!("[ambery-core] {err}");
            std::process::exit(1);
        }
    };
    eprintln!("ambery-core listening on http://127.0.0.1:{port}");
    axum::serve(listener, app).await.expect("serve core");
}

/// Tauri 模式 hook 端口：默认 47600（hook 脚本投递契约），AMBERY_PORT 显式覆盖。
/// 不做随机回退——换端口必须伴随 hook 配置同步，否则外部投递静默失效。
const DEFAULT_HOOK_PORT: u16 = 47600;

fn hook_port_value(env: Option<&str>) -> Result<u16, String> {
    let Some(raw) = env else { return Ok(DEFAULT_HOOK_PORT) };
    let port: u16 = raw.parse().map_err(|_| {
        format!("AMBERY_PORT 值无效：{raw:?}（需要 1..=65535 的端口号）")
    })?;
    if port == 0 {
        return Err("AMBERY_PORT 值无效：0（hook 契约需要固定端口，不能用随机端口）".into());
    }
    Ok(port)
}

async fn bind_hook_listener(port: u16) -> Result<tokio::net::TcpListener, String> {
    let addr = format!("127.0.0.1:{port}");
    tokio::net::TcpListener::bind(&addr).await.map_err(|err| {
        if err.kind() == std::io::ErrorKind::AddrInUse {
            format!(
                "端口 {port} 已被占用（{err}）。hook 脚本依赖固定端口，不能静默换端口；\
                 请关闭占用进程，或设置 AMBERY_PORT 换端口并同步更新 hook 脚本配置。"
            )
        } else {
            format!("绑定 {addr} 失败：{err}")
        }
    })
}

// ── #9.5 二分测试：invoke 是否到达 handler + State 是否提取成功（tauri::test mock runtime）──
#[cfg(test)]
mod ipc_tests {
    use super::*;

    fn build_harness_state(tag: &str) -> Arc<AppState> {
        let dir = std::env::temp_dir().join(format!("ambery-ipc-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::load_or_default(&dir);
        let harness = Harness::load(&dir, &dir, config.effective_compression_limit().unwrap_or(usize::MAX), 0).unwrap();
    // init 失败不阻断启动：记日志 + 按无 LLM 态运行（保循环可用，让用户能修配置）
    let backend = LlmBackend::from_config(&config.llm).unwrap_or_else(|err| {
        eprintln!("[llm] {err}——按无 LLM 态运行");
        LlmBackend::debug(ambery_core::llm::DebugAgent::default())
    });
        let ov = AmberyBackend::new(harness, config, backend);
        Arc::new(AppState::new(ov))
    }

    fn ipc(cmd: &str) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            // Tauri mock runtime origin 平台差异（tauri::test 文档同款）：
            // Windows/Android 用 http://tauri.localhost，其余用 tauri://localhost
            url: if cfg!(any(windows, target_os = "android")) {
                "http://tauri.localhost"
            } else {
                "tauri://localhost"
            }
            .parse()
            .unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        }
    }

    fn mock_app_with_commands() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .manage(SharedTauriState::new(TauriState(std::sync::Mutex::new(None))))
            .manage(CardWindowRegistry::default())
            .invoke_handler(tauri::generate_handler![get_state, get_config, get_config_schema, set_config, test_llm, get_api_key_status, set_api_key, toggle_pet, list_cards, update_card_layout, set_card_user_closed, ensure_card_window, close_card_window, export_theme, import_theme])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    #[tokio::test]
    async fn list_cards_returns_alive_cards_with_meta() {
        let app = mock_app_with_commands();
        let state = build_harness_state("list-cards");
        // 落两张卡：一张正常、一张用户已隐藏（user_closed）
        {
            let mut ov = state.ambery().lock().await;
            ov.harness
                .cards_upsert(&json!({"id":"todo-1","type":"todobox","title":"清单","items":[{"text":"a","done":false}]}), 1000)
                .unwrap();
            ov.harness
                .cards_upsert(&json!({"id":"note-2","type":"text_card","title":"便签","text":"x"}), 1001)
                .unwrap();
            ov.harness.cards_write_user_closed("note-2", true).unwrap();
            ov.harness.cards_write_layout("todo-1", (30, 40)).unwrap();
        }
        *app.state::<SharedTauriState>().0.lock().unwrap() = Some(state);
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let resp = tauri::test::get_ipc_response(&window, ipc("list_cards"));
        let body = resp.unwrap().deserialize::<Value>().unwrap();
        let arr = body.as_array().expect("list_cards 返回数组");
        assert_eq!(arr.len(), 2);
        let todo = arr.iter().find(|c| c["component"]["id"] == "todo-1").unwrap();
        assert_eq!(todo["component"]["type"], "todobox");
        assert_eq!(todo["user_closed"], json!(false));
        assert_eq!(todo["layout"]["offset"], json!([30, 40]));
        assert_eq!(todo["layout"]["manual"], json!(true));
        let note = arr.iter().find(|c| c["component"]["id"] == "note-2").unwrap();
        assert_eq!(note["user_closed"], json!(true));
        let dir = std::env::temp_dir().join("ambery-ipc-test-list-cards");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn update_card_layout_writes_file_and_effect() {
        let app = mock_app_with_commands();
        let state = build_harness_state("layout");
        {
            let mut ov = state.ambery().lock().await;
            ov.harness
                .cards_upsert(&json!({"id":"todo-1","type":"todobox","title":"清单","items":[{"text":"a","done":false}]}), 1000)
                .unwrap();
        }
        *app.state::<SharedTauriState>().0.lock().unwrap() = Some(state);
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let mut req = ipc("update_card_layout");
        req.body = tauri::ipc::InvokeBody::Json(json!({ "id": "todo-1", "offset": [30, 40] }));
        let resp = tauri::test::get_ipc_response(&window, req);
        let body = resp.unwrap().deserialize::<Value>().unwrap();
        assert_eq!(body["ok"], json!(true), "{body}");
        // 文件 _meta.layout 已回写
        let dir = std::env::temp_dir().join("ambery-ipc-test-layout");
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("memory/cards/todo-1.card.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(raw["_meta"]["layout"]["offset"], json!([30, 40]));
        assert_eq!(raw["_meta"]["layout"]["manual"], json!(true));
        // 端点记录 card_layout effect（invoke 写动作 core 接收端记录）
        let effects = std::fs::read_to_string(dir.join("effect.jsonl")).unwrap();
        assert!(effects.contains("\"kind\":\"card_layout\""), "{effects}");
        // 未知 id：ok=false 不落文件
        let mut req = ipc("update_card_layout");
        req.body = tauri::ipc::InvokeBody::Json(json!({ "id": "no-such", "offset": [1, 2] }));
        let resp = tauri::test::get_ipc_response(&window, req);
        assert_eq!(resp.unwrap().deserialize::<Value>().unwrap()["ok"], json!(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn set_card_user_closed_writes_file_and_effect() {
        let app = mock_app_with_commands();
        let state = build_harness_state("visibility");
        {
            let mut ov = state.ambery().lock().await;
            ov.harness
                .cards_upsert(&json!({"id":"todo-1","type":"todobox","title":"清单","items":[{"text":"a","done":false}]}), 1000)
                .unwrap();
        }
        *app.state::<SharedTauriState>().0.lock().unwrap() = Some(state);
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let mut req = ipc("set_card_user_closed");
        req.body = tauri::ipc::InvokeBody::Json(json!({ "id": "todo-1", "userClosed": true }));
        let resp = tauri::test::get_ipc_response(&window, req);
        assert_eq!(resp.unwrap().deserialize::<Value>().unwrap()["ok"], json!(true));
        let dir = std::env::temp_dir().join("ambery-ipc-test-visibility");
        let raw: Value = serde_json::from_str(
            &std::fs::read_to_string(dir.join("memory/cards/todo-1.card.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(raw["_meta"]["user_closed"], json!(true), "显示选择落文件");
        // component 不被显示选择回写触碰
        assert_eq!(raw["component"]["title"], "清单");
        let effects = std::fs::read_to_string(dir.join("effect.jsonl")).unwrap();
        assert!(effects.contains("\"kind\":\"card_visibility\""), "{effects}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn toggle_pet_records_each_runtime_action_not_one_toggle() {
        let app = mock_app_with_commands();
        let mgr = app.state::<SharedTauriState>();
        *mgr.0.lock().unwrap() = Some(build_harness_state("toggle"));
        tauri::WebviewWindowBuilder::new(&app, "pet", Default::default())
            .build()
            .unwrap();
        let was_visible = app.get_webview_window("pet").unwrap().is_visible().unwrap_or(false);
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let resp = tauri::test::get_ipc_response(&window, ipc("toggle_pet"));
        assert!(resp.is_ok(), "toggle_pet invoke 失败: {resp:?}");
        // 复合入口逐动作记录：不能合成一条 toggle
        let dir = std::env::temp_dir().join("ambery-ipc-test-toggle");
        let mut content = String::new();
        for _ in 0..60 {
            content = std::fs::read_to_string(dir.join(ambery_core::EFFECT_FILE)).unwrap_or_default();
            let hits = ["window_hidden", "window_visible", "event_emit"]
                .iter()
                .filter(|k| content.contains(*k))
                .count();
            if hits >= 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(!content.contains("\"kind\":\"toggle\""), "不得出现笼统 toggle effect: {content}");
        if was_visible {
            assert!(content.contains("window_hidden"), "hide 分支: {content}");
            assert!(content.contains("pet:hidden"), "hide 分支: {content}");
        } else {
            assert!(content.contains("window_visible"), "show 分支: {content}");
            assert!(content.contains("pet:shown"), "show 分支: {content}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn ensure_and_close_card_window_authoritative() {
        let app = mock_app_with_commands();
        *app.state::<SharedTauriState>().0.lock().unwrap() = Some(build_harness_state("ensure"));
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let spec = json!({"id":"todo-1","type":"text_card","title":"t","text":"x"});
        let call = |cmd: &str, body: Value| {
            let mut req = ipc(cmd);
            req.body = tauri::ipc::InvokeBody::Json(body);
            tauri::test::get_ipc_response(&window, req)
                .unwrap()
                .deserialize::<Value>()
                .unwrap()
        };
        // 缺席 → create：opened + 窗口在注册表 + window_opened effect
        let r = call("ensure_card_window", json!({ "id": "todo-1", "spec": spec }));
        assert_eq!(r["result"], json!("opened"), "{r}");
        assert!(app.get_webview_window("card-todo-1").is_some());
        // 存在 → reuse：不新建第二个（label 唯一），result=reused
        let r = call("ensure_card_window", json!({ "id": "todo-1", "spec": spec }));
        assert_eq!(r["result"], json!("reused"), "{r}");
        // 统一关闭：权威注册表决策（MockRuntime 无事件循环，物理移除不可观测，
        // close 兜底超时后仍出表；生产由 wait_window_gone 等事件循环真移除）
        let r = call("close_card_window", json!({ "id": "todo-1" }));
        assert_eq!(r["result"], json!("closed"), "{r}");
        // 关已关的窗：absent 幂等（决策层已出表）
        let r = call("close_card_window", json!({ "id": "todo-1" }));
        assert_eq!(r["result"], json!("absent"), "{r}");
        // 非法 id 拒绝
        let r = call("ensure_card_window", json!({ "id": "bad id!", "spec": spec }));
        assert_eq!(r["result"], json!("error"), "{r}");
        // 嵌套 id（可含 / 子目录）core 接受壳也接受
        let r = call("ensure_card_window", json!({ "id": "proj/nested-1", "spec": spec }));
        assert_eq!(r["result"], json!("opened"), "{r}");
        assert!(app.get_webview_window("card-proj/nested-1").is_some());
        // 路径逃逸与空段拒绝
        let r = call("ensure_card_window", json!({ "id": "../escape", "spec": spec }));
        assert_eq!(r["result"], json!("error"), "{r}");
        let r = call("ensure_card_window", json!({ "id": "a//b", "spec": spec }));
        assert_eq!(r["result"], json!("error"), "{r}");
        // effect 流含 window_opened / window_closed
        let dir = std::env::temp_dir().join("ambery-ipc-test-ensure");
        let mut content = String::new();
        for _ in 0..40 {
            content = std::fs::read_to_string(dir.join(ambery_core::EFFECT_FILE)).unwrap_or_default();
            if content.contains("window_opened") && content.contains("window_closed") { break; }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(content.contains("\"kind\":\"window_opened\""), "{content}");
        assert!(content.contains("\"kind\":\"window_closed\""), "{content}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn invoke_reaches_handler_and_extracts_state() {
        let app = mock_app_with_commands();
        // 注入 AppState（模拟 run_core 完成，wait_state 立即返回）
        let state = build_harness_state("extract");
        state.set_pending_notifications(7).await;
        let mgr = app.state::<SharedTauriState>();
        *mgr.0.lock().unwrap() = Some(state);
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();

        // 新 command：get_config（含 eprintln 探针，test 输出可见 [tauri-cmd]）
        let resp = tauri::test::get_ipc_response(&window, ipc("get_config"));
        assert!(resp.is_ok(), "get_config invoke 失败: {resp:?}");
        // 新 command：get_state——必须回读真实计数，不再硬编码 0
        let body = tauri::test::get_ipc_response(&window, ipc("get_state"))
            .unwrap()
            .deserialize::<Value>()
            .unwrap();
        assert_eq!(body["pendingNotifications"], json!(7), "get_state 应回读真实计数: {body}");
        // 设置面板链路：get_config_schema + set_config（apply_config_by_path 全管道）
        let resp = tauri::test::get_ipc_response(&window, ipc("get_config_schema"));
        assert!(resp.is_ok(), "get_config_schema invoke 失败: {resp:?}");
        let mut req = ipc("set_config");
        req.body = tauri::ipc::InvokeBody::Json(json!({ "path": "view_scale", "value": 0.6 }));
        let resp = tauri::test::get_ipc_response(&window, req);
        assert!(resp.is_ok(), "set_config invoke 失败: {resp:?}");
        let body = resp.unwrap().deserialize::<Value>().unwrap();
        assert_eq!(body["ok"], json!(true), "set_config apply 失败: {body}");
    }

    #[test]
    fn invoke_before_state_injection_fails_not_hangs() {
        // race 场景：state 未注入时 wait_state 5s 超时返回 Err（而非 panic/挂死）
        let app = mock_app_with_commands();
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();
        let resp = tauri::test::get_ipc_response(&window, ipc("get_config"));
        // handler 到达但返回 "not ready" 错误（证明 invoke 链路通，只是 state 未就绪）
        let err = resp.expect_err("state 未注入时应返回错误");
        assert!(err.to_string().contains("not ready"), "意外错误: {err}");
    }

    #[test]
    fn hook_port_env_parsing_is_explicit() {
        assert_eq!(hook_port_value(None), Ok(DEFAULT_HOOK_PORT));
        assert_eq!(hook_port_value(Some("47601")), Ok(47601));
        assert!(hook_port_value(Some("0")).is_err(), "0 不是合法监听端口");
        assert!(hook_port_value(Some("not-a-port")).is_err());
    }

    #[tokio::test]
    async fn hook_listener_bind_conflict_returns_readable_error() {
        // 先占一个真实 loopback 端口，再对同端口请求绑定——错误必须可读而非 panic
        let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = occupied.local_addr().unwrap().port();
        let err = bind_hook_listener(port).await.expect_err("同端口应报错");
        assert!(err.contains("已被占用"), "错误不可读: {err}");
        assert!(err.contains(&port.to_string()), "错误未带端口: {err}");
        assert!(err.contains("AMBERY_PORT"), "错误未给换端口指引: {err}");
    }
}
