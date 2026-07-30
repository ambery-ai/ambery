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

/// 面板底部按钮（原托盘菜单动作）
#[tauri::command]
fn toggle_pet(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("pet") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            if let Some(ch) = app.get_webview_window("chat") { let _ = ch.hide(); }
            let _ = app.emit("cards:hide", ());
            let _ = app.emit("pet:hidden", ());
        } else {
            let _ = w.show();
            let _ = app.emit("pet:shown", ());
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

/// 设置面板改值（对齐 server post_config：apply + 广播 + restartRequired）
#[tauri::command]
async fn set_config(state: tauri::State<'_, SharedTauriState>, path: String, value: Value) -> Result<Value, String> {
    let s = wait_state(&state)?;
    let mut ov = s.overseer().lock().await;
    match ov.apply_config_by_path(&path, value) {
        Ok(outcome) => {
            let restart = outcome.restart_required.clone();
            let effects = outcome.effects;
            drop(ov);
            for e in &effects {
                s.broadcast_effect_json(overseer_core::server::effect_json(e)).await;
            }
            Ok(json!({ "ok": true, "restartRequired": restart }))
        }
        Err(e) => Ok(json!({ "ok": false, "error": e })),
    }
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            toggle_pet, quit_app,
            get_state, get_context, append_user, push_event, get_config, get_config_schema, set_config
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
        config.effective_compression_limit().unwrap_or(usize::MAX),
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
    spawn_queue_consumer(state.clone());

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
        let harness = Harness::load(&dir, &dir, config.token_threshold, 0).unwrap();
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
            .invoke_handler(tauri::generate_handler![get_state, get_config, get_config_schema, set_config])
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
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
