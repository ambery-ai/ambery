//! Tauri 壳（docs/multi-window.md）：三独立窗口（pet + chat + menu）+ 内嵌 overseer-core。
//! 前端通信走 Tauri IPC（docs/core-server.md），仅 /hook 保留 HTTP。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::LlmBackend;
use overseer_core::overseer::OverseerBackend;
use overseer_core::context::Role;
use overseer_core::server::{now_ms, hook_router, spawn_queue_consumer, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;

mod menu_window;
mod window;
mod tray;
mod tauri_runtime_actions;

/// 面板底部按钮（原托盘菜单动作）。复合入口逐动作转发、逐动作记录
/// （docs/effect-reporting.md：四个动作四条 effect，不能合成一条 toggle）
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
    let ov = s.overseer().lock().await;
    Ok(json!({
        "instances": ov.harness.agents.iter().map(|a| json!({"id":a.hash,"name":a.name,"status":a.status})).collect::<Vec<_>>(),
        "pendingNotifications": 0
    }))
}

#[tauri::command]
async fn get_context(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    Ok(json!(ov.harness.context.messages()))
}

#[tauri::command]
async fn append_user(state: tauri::State<'_, SharedTauriState>, text: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    {
        let mut ov = s.overseer().lock().await;
        ov.enqueue(Role::User, text, now_ms()).map_err(|e| e.to_string())?;
    }
    // 生产者只入队，放行由消费者任务驱动（concepts §10c）
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
    let mut ov = s.overseer().lock().await;
    // 结构化事实 → 文本（lifecycle 语义单源，docs/i18n.md §Harness 内部语言）
    let ev = json!({
        "action": action,
        "card_type": card_type,
        "title": title,
        "text": text,
        "target": target,
        "checked": checked,
    });
    let desc = overseer_core::lifecycle::user_action_desc(
        overseer_core::i18n::Lang::of(&ov.config.harness_language),
        &ev,
    );
    // 动作流记录（docs/effect-reporting.md §kind）：前端 push_event = interaction/frontend
    ov.record_frontend_effect("interaction", json!({ "desc": desc.as_str(), "card_id": card_id.as_deref() }));
    // 用户 × 关卡：dismiss（删 .card.json、出注册表、忘记布局）+ closed_by_user 双行事件
    if action == "dismiss" {
        if let Some(cid) = card_id.as_deref() {
            let ts = now_ms();
            if let Some(entry) = ov.harness.cards_remove(cid) {
                let lc = overseer_core::lifecycle::DefaultLifecycle::for_lang(
                    overseer_core::i18n::Lang::of(&ov.config.harness_language),
                );
                use overseer_core::lifecycle::Lifecycle;
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
    let ov = s.overseer().lock().await;
    let cfg = &ov.config;
    Ok(json!({ "kaomoji": cfg.kaomoji, "setAutonomyDefaultTtlMs": cfg.set_autonomy_default_ttl_ms, "viewScale": cfg.view_scale, "badgeStyle": cfg.badge_style, "badgeSide": cfg.badge_side, "theme": cfg.theme, "themes": cfg.themes, "uiLanguage": cfg.ui_language, "name": cfg.name }))
}

/// 主题导出（docs/theme.md §导出、分享与兼容）：写 `<config_root>/themes/<name>.theme.json`
#[tauri::command]
async fn export_theme(state: tauri::State<'_, SharedTauriState>, name: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    let root = ov.harness.config_dir().to_path_buf();
    match overseer_core::config::theme::export_theme(&root, &ov.config, &name) {
        Ok(path) => {
            // 写文件副作用，端点记录（docs/effect-reporting.md §通道）
            ov.record_frontend_effect("theme_export", json!({ "name": name.as_str() }));
            Ok(json!({ "ok": true, "path": path.display().to_string() }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// 主题导入（docs/theme.md §导出、分享与兼容）：版本检查 → 兼容变换 → 校验 →
/// 统一修改管道写入 themes.<name>（原子拒绝 + 广播 config_changed，全部窗口即切）
#[tauri::command]
async fn import_theme(state: tauri::State<'_, SharedTauriState>, file: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let root = {
        let ov = s.overseer().lock().await;
        ov.harness.config_dir().to_path_buf()
    };
    let (name, value) = match overseer_core::config::theme::import_theme(&root, &file) {
        Ok(r) => r,
        Err(e) => return Ok(json!({ "ok": false, "error": e })),
    };
    let path = format!("themes.{name}");
    let mut ov = s.overseer().lock().await;
    match ov.apply_config_by_path(&path, value) {
        Ok(outcome) => {
            ov.record_frontend_effect("config_update", json!({ "path": path.as_str() }));
            drop(ov);
            overseer_core::server::finish_config_outcome(&s, outcome).await;
            Ok(json!({ "ok": true, "name": name }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

#[tauri::command]
async fn get_config_schema(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    let restart = ov.restart_required();
    let load_error = s.config_error().await;
    Ok(json!({
        "version": overseer_core::config::migrate::CURRENT_VERSION,
        "readOnly": ov.config.read_only,
        "restartRequired": restart,
        "loadError": load_error,
        "nodes": overseer_core::config::reflect::config_nodes(&ov.config),
    }))
}

/// 设置面板改值（对齐 server post_config：apply + 广播 + restartRequired + llm 重建）
#[tauri::command]
async fn set_config(state: tauri::State<'_, SharedTauriState>, path: String, value: Value) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.overseer().lock().await;
    match ov.apply_config_by_path(&path, value) {
        Ok(outcome) => {
            // 动作流记录（docs/effect-reporting.md §kind）：前端设置面板 = config_update/frontend
            ov.record_frontend_effect("config_update", json!({ "path": path.as_str() }));
            let restart = outcome.restart_required.clone();
            drop(ov);
            overseer_core::server::finish_config_outcome(&s, outcome).await;
            Ok(json!({ "ok": true, "restartRequired": restart }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// Card 跨重启恢复（readonly 查询，docs/components.md §Card 文件）：
/// pet 启动 pull 全部存活卡片（component + _meta）；可见性过滤在前端（pull-on-ready，
/// 规避 push-at-startup 的 webview 未就绪时序漏洞）
#[tauri::command]
async fn list_cards(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    let cards_dir = ov.harness.cards_dir();
    let mut out = vec![];
    for (id, e) in &ov.harness.cards {
        if let Some(component) = overseer_core::cards::read_component(&cards_dir, id) {
            out.push(json!({
                "component": component,
                "user_closed": e.user_closed,
                "layout": e.layout,
            }));
        }
    }
    Ok(json!(out))
}

/// Card 布局回写（docs/components.md §Card 文件）：拖拽结束把相对 pet 偏移落
/// .card.json（_meta.layout.offset/manual）。invoke 写动作，端点记录 card_layout
/// effect（docs/effect-reporting.md §通道：core 接收端记录）
#[tauri::command]
async fn update_card_layout(state: tauri::State<'_, SharedTauriState>, id: String, offset: (i64, i64)) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.overseer().lock().await;
    match ov.harness.cards_write_layout(&id, offset) {
        Ok(()) => {
            ov.record_frontend_effect("card_layout", json!({ "id": id.as_str(), "manual": true }));
            Ok(json!({ "ok": true }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

/// Card 显示选择回写（Cards Shelf 显隐切换，docs/components.md §Card 文件）：
/// 只改 _meta.user_closed（窗口动作由 pet 经 shelf:visibility 事件执行）；
/// invoke 写动作，端点记录 card_visibility effect（docs/effect-reporting.md §通道）
#[tauri::command]
async fn set_card_user_closed(state: tauri::State<'_, SharedTauriState>, id: String, user_closed: bool) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.overseer().lock().await;
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

/// 前端非 readonly @tauri-apps/api 调用上报（docs/effect-reporting.md §通道）
#[tauri::command]
async fn record_effect(state: tauri::State<'_, SharedTauriState>, kind: String, payload: Option<Value>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    ov.record_frontend_effect(&kind, payload.unwrap_or(Value::Null));
    Ok(json!({ "ok": true }))
}

/// card 窗口 id 合法性（与 core 同一约束，docs/components.md §Card 文件）：
/// A-Z a-z 0-9 _ - . /，路径段不得为空或 `..`（嵌套子目录合法；core 接受壳不得拒绝）
fn valid_card_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/'))
        && !id.split('/').any(|seg| seg.is_empty() || seg == "..")
}

/// Card 窗口权威注册表（docs/case-runner.md §窗口决策上提，#25 断根）。
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

/// 窗口决策上提（docs/case-runner.md §窗口决策上提，#25 断根）：
/// 前端不再 getByLabel 自查自决存在性——Rust 权威注册表同步决策 create / reuse。
/// create → window_opened 记录 + 500ms 后推 card:spec（等页面 JS listener 就绪）；
/// reuse → 立即重推 card:spec（原地更新），记录 event_emit。
#[tauri::command]
async fn ensure_card_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    registry: tauri::State<'_, CardWindowRegistry>,
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
                    let ov = s.overseer().lock().await;
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
    }
    let win = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App("index.html#card".into()))
        .title(&label)
        .inner_size(520.0, 440.0)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .focused(false)
        .shadow(false)
        .skip_taskbar(true)
        .visible(false)
        .build()
        .map_err(|e| e.to_string())?;
    registry.0.lock().unwrap().insert(label.clone(), CardWinState::Alive);
    {
        let ov = s.overseer().lock().await;
        ov.record_frontend_effect("window_opened", json!({ "window": label.as_str() }));
    }
    // 页面 JS listener 注册在 load 之后；沿用 500ms 经验延迟推 spec（窗已毁则 emit 静默失败）
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        let _ = win.emit("card:spec", spec);
    });
    Ok(json!({ "result": "opened" }))
}

/// 统一关闭（docs/multi-window.md §窗口创建与生命周期）：destroy 不经 onCloseRequested
/// （preventDefault 会留将死窗口，#25 根因 B 的机制）。标 Closing → destroy → 等物理
/// 移除 → 出表；ensure 侧见 Closing 等待，窗口期被本表完全吸收。
/// agent close / shelf dismiss / 用户 × 三条路径收口到本命令。
#[tauri::command]
async fn close_card_window<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    registry: tauri::State<'_, CardWindowRegistry>,
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
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    ov.record_frontend_effect("window_closed", json!({ "window": label.as_str() }));
    Ok(json!({ "result": "closed" }))
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            toggle_pet, quit_app,
            get_state, get_context, append_user, push_event, get_config, get_config_schema, set_config,
            record_effect, list_cards, update_card_layout, set_card_user_closed,
            ensure_card_window, close_card_window, export_theme, import_theme
        ])
        .manage(SharedTauriState::new(TauriState(std::sync::Mutex::new(None))))
        .setup(|app| {
            let pet = app.get_webview_window("pet").expect("pet window");
            let chat = app.get_webview_window("chat").expect("chat window");
            let menu = app.get_webview_window("menu").expect("menu window");
            let shelf = app.get_webview_window("shelf").expect("shelf window");

            window::init_window(&pet);
            window::init_window(&chat);
            window::init_window(&shelf);
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
    let config = Config::load_or_default(&overseer_core::paths::config_root());
    let harness = Harness::load_with_lang(
        &overseer_core::paths::storage_dir(),
        &overseer_core::paths::config_root(),
        config.effective_compression_limit().unwrap_or(usize::MAX),
        now_ms(),
        overseer_core::i18n::Lang::of(&config.harness_language),
    ).expect("load harness");
    let backend = LlmBackend::from_config(&config.llm);
    let (timer_tick, timer_batch) = (config.timer.tick_ms, config.timer.batch);
    let mut overseer = OverseerBackend::new(harness, config, backend);

    let sidecar = overseer_core::paths::sidecar_exe()
        .map(overseer_core::sidecar::SidecarClient::new)
        .map(Arc::new);
    let sidecar_for_sweep = sidecar.clone();
    let sidecar_for_vd = sidecar.clone();
    overseer.sidecar_enabled = sidecar.is_some();

    let mock = Arc::new(std::sync::Mutex::new(std::collections::HashMap::<String, String>::new()));
    {
        let mock = mock.clone();
        let sidecar_for_read = sidecar.clone();
        overseer.terminal_reader = Some(Arc::new(move |inst: &str| {
            sidecar_for_read.as_ref().and_then(|s| s.read_instance(inst))
                .or_else(|| mock.lock().unwrap().get(inst).cloned())
        }));
    }
    // tab 定位服务（docs/hook.md §定位缓存）：session_start 探测 / 读路径回写 / 结束清缓存
    let sidecar_for_locate = sidecar.clone();
    overseer.tab_locator = Some(Arc::new(move |inst: &str| {
        sidecar_for_locate.as_ref().and_then(|sc| sc.call(&json!({ "cmd": "find_tab", "name": inst })))
            .and_then(|r| Some(overseer_core::TabRef { hwnd: r["hwnd"].as_i64()?, index: r["index"].as_i64()? }))
    }));
    let sidecar_for_forget = sidecar.clone();
    overseer.tab_forgetter = Some(Arc::new(move |inst: &str| {
        if let Some(sc) = sidecar_for_forget.as_ref() {
            sc.evict(inst);
        }
    }));
    {
        let sc = sidecar_for_vd.clone();
        overseer.vd_switcher = Some(Arc::new(move |inst: &str| {
            let Some(sc) = sc.as_ref() else { return false };
            let Some(resp) = sc.call(&json!({ "cmd": "list_windows" })) else { return false };
            let win = resp["windows"].as_array().and_then(|ws| ws.iter().find(|w| w["title"].as_str().map(|t| t.contains(inst)).unwrap_or(false)));
            let Some(hwnd) = win.and_then(|w| w["hwnd"].as_i64()) else { return false };
            sc.call(&json!({ "cmd": "switch_to_window_desktop", "hwnd": hwnd }))
                .and_then(|r| r["switched"].as_bool()).unwrap_or(false)
        }));
    }

    let state = Arc::new(AppState::new(overseer, mock));

    // 启动扫描
    if let Some(sc) = sidecar_for_sweep.clone() {
        let mut ov = state.overseer().lock().await;
        if let Err(e) = ov.startup_sweep(&move |req| sc.call(req), now_ms()).await {
            eprintln!("startup sweep: {e}");
        }
    }

    spawn_timer_task(state.clone(), timer_tick, timer_batch);
    spawn_queue_consumer(state.clone());
    // 外部文件自动载入（docs/config.md §外部文件自动载入）
    overseer_core::server::spawn_config_watcher(state.clone(), overseer_core::paths::config_root());
    // Cron 调度任务（concepts §10g，docs/cron.md）
    overseer_core::server::spawn_cron_task(state.clone());

    // 注入 Tauri managed state
    *state_mgr.0.lock().unwrap() = Some(state.clone());

    // effects 推送：WS（浏览器 debug 兼容期）+ Tauri 原生事件（#9.5 emit 链路接通）
    let (tx, _) = tokio::sync::broadcast::channel(64);
    {
        let tx = tx.clone();
        let handle = handle.clone();
        state
            .set_sender(Box::new(move |msg: Value| {
                let _ = tx.send(msg.to_string());
                let _ = handle.emit("effect", msg);
            }))
            .await;
    }
    // 流式 delta 旁路（docs/streaming.md）
    state.wire_effect_sink().await;
    // Tauri 模式：前端走 IPC（TauriBridge），HTTP 仅留 /hook（外部 hook 脚本，进程外不可走 command）
    let app = hook_router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600").await.expect("bind 47600");
    eprintln!("overseer-core listening on http://127.0.0.1:47600");
    axum::serve(listener, app).await.expect("serve core");
}

// ── #9.5 二分测试：invoke 是否到达 handler + State 是否提取成功（tauri::test mock runtime）──
#[cfg(test)]
mod ipc_tests {
    use super::*;

    fn build_harness_state(tag: &str) -> Arc<AppState> {
        let dir = std::env::temp_dir().join(format!("overseer-ipc-test-{tag}"));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::load_or_default(&dir);
        let harness = Harness::load(&dir, &dir, config.effective_compression_limit().unwrap_or(usize::MAX), 0).unwrap();
        let backend = LlmBackend::from_config(&config.llm);
        let ov = OverseerBackend::new(harness, config, backend);
        let mock = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        Arc::new(AppState::new(ov, mock))
    }

    fn ipc(cmd: &str) -> tauri::webview::InvokeRequest {
        tauri::webview::InvokeRequest {
            cmd: cmd.into(),
            callback: tauri::ipc::CallbackFn(0),
            error: tauri::ipc::CallbackFn(1),
            url: "http://tauri.localhost".parse().unwrap(),
            body: tauri::ipc::InvokeBody::default(),
            headers: Default::default(),
            invoke_key: tauri::test::INVOKE_KEY.to_string(),
        }
    }

    fn mock_app_with_commands() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .manage(SharedTauriState::new(TauriState(std::sync::Mutex::new(None))))
            .manage(CardWindowRegistry::default())
            .invoke_handler(tauri::generate_handler![get_state, get_config, get_config_schema, set_config, toggle_pet, list_cards, update_card_layout, set_card_user_closed, ensure_card_window, close_card_window, export_theme, import_theme])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    #[tokio::test]
    async fn list_cards_returns_alive_cards_with_meta() {
        let app = mock_app_with_commands();
        let state = build_harness_state("list-cards");
        // 落两张卡：一张正常、一张用户已隐藏（user_closed）
        {
            let mut ov = state.overseer().lock().await;
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
        let dir = std::env::temp_dir().join("overseer-ipc-test-list-cards");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn update_card_layout_writes_file_and_effect() {
        let app = mock_app_with_commands();
        let state = build_harness_state("layout");
        {
            let mut ov = state.overseer().lock().await;
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
        let dir = std::env::temp_dir().join("overseer-ipc-test-layout");
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
            let mut ov = state.overseer().lock().await;
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
        let dir = std::env::temp_dir().join("overseer-ipc-test-visibility");
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
        // 复合入口逐动作记录：不能合成一条 toggle（docs/effect-reporting.md §一动作一记录）
        let dir = std::env::temp_dir().join("overseer-ipc-test-toggle");
        let mut content = String::new();
        for _ in 0..60 {
            content = std::fs::read_to_string(dir.join(overseer_core::EFFECT_FILE)).unwrap_or_default();
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
        // 嵌套 id（docs/components.md §Card 文件：可含 / 子目录）core 接受壳也接受
        let r = call("ensure_card_window", json!({ "id": "proj/nested-1", "spec": spec }));
        assert_eq!(r["result"], json!("opened"), "{r}");
        assert!(app.get_webview_window("card-proj/nested-1").is_some());
        // 路径逃逸与空段拒绝
        let r = call("ensure_card_window", json!({ "id": "../escape", "spec": spec }));
        assert_eq!(r["result"], json!("error"), "{r}");
        let r = call("ensure_card_window", json!({ "id": "a//b", "spec": spec }));
        assert_eq!(r["result"], json!("error"), "{r}");
        // effect 流含 window_opened / window_closed
        let dir = std::env::temp_dir().join("overseer-ipc-test-ensure");
        let mut content = String::new();
        for _ in 0..40 {
            content = std::fs::read_to_string(dir.join(overseer_core::EFFECT_FILE)).unwrap_or_default();
            if content.contains("window_opened") && content.contains("window_closed") { break; }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        assert!(content.contains("\"kind\":\"window_opened\""), "{content}");
        assert!(content.contains("\"kind\":\"window_closed\""), "{content}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invoke_reaches_handler_and_extracts_state() {
        let app = mock_app_with_commands();
        // 注入 AppState（模拟 run_core 完成，wait_state 立即返回）
        let mgr = app.state::<SharedTauriState>();
        *mgr.0.lock().unwrap() = Some(build_harness_state("extract"));
        let window = tauri::WebviewWindowBuilder::new(&app, "main", Default::default())
            .build()
            .unwrap();

        // 新 command：get_config（含 eprintln 探针，test 输出可见 [tauri-cmd]）
        let resp = tauri::test::get_ipc_response(&window, ipc("get_config"));
        assert!(resp.is_ok(), "get_config invoke 失败: {resp:?}");
        // 新 command：get_state
        let resp = tauri::test::get_ipc_response(&window, ipc("get_state"));
        assert!(resp.is_ok(), "get_state invoke 失败: {resp:?}");
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
}
