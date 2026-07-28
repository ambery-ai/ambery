//! Tauri 壳（docs/multi-window.md）：三独立窗口（pet + chat + menu）+ 内嵌 overseer-core。
//! 前端通信走 Tauri IPC（docs/core-server.md），仅 /hook 保留 HTTP。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::LlmBackend;
use overseer_core::overseer::{Effect, OverseerBackend};
use overseer_core::queue::{QueueMessage, Role};
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use serde_json::{json, Value};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;

mod menu_window;
mod window;
mod tray;

/// 面板底部按钮（原托盘菜单动作）
#[tauri::command]
fn toggle_pet(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("pet") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            if let Some(ch) = app.get_webview_window("chat") { let _ = ch.hide(); }
            let _ = app.emit("cards:hide", ());
        } else {
            let _ = w.show();
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
async fn get_queue(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    Ok(json!(ov.harness.queue.messages()))
}

#[tauri::command]
async fn append_user(state: tauri::State<'_, SharedTauriState>, text: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.overseer().lock().await;
    ov.harness.append_queue(QueueMessage::new(Role::User, text, now_ms()))
        .map_err(|e| e.to_string())?;
    let effects = ov.run_trigger(now_ms(), 0).await.map_err(|e| e.to_string())?;
    drop(ov);
    for e in &effects {
        if matches!(e, Effect::RenderComponent(_)) {}
        let msg = effect_to_json(e);
        s.broadcast_effect_json(msg).await;
    }
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn push_event(state: tauri::State<'_, SharedTauriState>, desc: String) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.overseer().lock().await;
    ov.harness.event_buffer.push(desc);
    Ok(json!({ "ok": true }))
}

#[tauri::command]
async fn get_config(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    eprintln!("[tauri-cmd] get_config called");
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    let cfg = &ov.config;
    Ok(json!({ "kaomoji": cfg.kaomoji, "setAutonomyDefaultTtlMs": cfg.set_autonomy_default_ttl_ms, "viewScale": cfg.view_scale }))
}

#[tauri::command]
async fn get_config_schema(state: tauri::State<'_, SharedTauriState>) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let ov = s.overseer().lock().await;
    Ok(json!({ "version": overseer_core::config::migrate::CURRENT_VERSION, "readOnly": ov.config.read_only, "nodes": overseer_core::config::reflect::config_nodes(&ov.config) }))
}

fn effect_to_json(e: &Effect) -> Value {
    match e {
        Effect::RenderComponent(spec) => json!({ "kind": "render_component", "spec": spec }),
        Effect::SetAutonomy { face, motion, ttl_ms } => json!({ "kind": "set_autonomy", "face": face, "motion": motion, "ttlMs": ttl_ms }),
        Effect::ConfigChanged { .. } => json!({ "kind": "config" }),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            toggle_pet, quit_app,
            get_state, get_queue, append_user, push_event, get_config, get_config_schema
        ])
        .manage(SharedTauriState::new(TauriState(std::sync::Mutex::new(None))))
        .setup(|app| {
            let pet = app.get_webview_window("pet").expect("pet window");
            let chat = app.get_webview_window("chat").expect("chat window");
            let menu = app.get_webview_window("menu").expect("menu window");

            window::init_window(&pet);
            window::init_window(&chat);
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
    let harness = Harness::load(
        &overseer_core::paths::storage_dir(),
        &overseer_core::paths::config_root(),
        config.token_threshold,
        now_ms(),
    ).expect("load harness");
    let backend = LlmBackend::from_config(&config.llm);
    let (timer_tick, timer_batch) = (config.timer_tick_ms, config.timer_batch);
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
        overseer.terminal_reader = Some(Arc::new(move |inst: &str| {
            sidecar.as_ref().and_then(|s| s.read_instance(inst))
                .or_else(|| mock.lock().unwrap().get(inst).cloned())
        }));
    }
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

    // 注入 Tauri managed state
    *state_mgr.0.lock().unwrap() = Some(state.clone());

    // HTTP+WS server（前端 RemoteBridge 暂用，Tauri IPC 后续独立调试）
    let (tx, _) = tokio::sync::broadcast::channel(64);
    {
        let tx = tx.clone();
        state.set_sender(Box::new(move |msg: Value| { let _ = tx.send(msg.to_string()); })).await;
    }
    let app = router(state, tx);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600").await.expect("bind 47600");
    eprintln!("overseer-core listening on http://127.0.0.1:47600");
    axum::serve(listener, app).await.expect("serve core");
}
