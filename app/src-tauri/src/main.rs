//! Tauri 壳（docs/multi-window.md）：三独立窗口（pet + cards + chat）+ 内嵌 overseer-core。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::LlmBackend;
use overseer_core::overseer::OverseerBackend;
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use std::sync::Arc;
use tauri::Emitter;
use tauri::Manager;
use tokio::sync::broadcast;

mod menu_window;
mod window;
mod tray;

// tauri dev 自动管 vite，无需额外心跳检测

/// 面板底部按钮（原托盘菜单动作）
#[tauri::command]
fn toggle_pet(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("pet") {
        if w.is_visible().unwrap_or(false) {
            let _ = w.hide();
            if let Some(ch) = app.get_webview_window("chat") { let _ = ch.hide(); }
            // 动态 card 窗口由前端 engine 控，pet toggle 时广播 cards:hide
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

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![toggle_pet, quit_app])
        .setup(|app| {
            let pet = app.get_webview_window("pet").expect("pet window");
            let chat = app.get_webview_window("chat").expect("chat window");
            let menu = app.get_webview_window("menu").expect("menu window");

            // 所有窗口统一 pin + fight-back（menu 面板除外：它是菜单行为，失焦即隐）
            // 动态 card 窗口由前端 WebviewWindow 创建，不在此处初始化
            window::init_window(&pet);
            window::init_window(&chat);
            menu_window::init_menu_window(&menu);

            // 托盘：右键弹设置面板
            tray::init_tray(app.handle(), &pet)?;

            // 复用 Tauri async runtime 启动 overseer-core
            tauri::async_runtime::spawn(run_core());

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

async fn run_core() {
    // Config / Storage 分离（concepts §12/§13，core/paths.rs）
    let config = Config::load_or_default(&overseer_core::paths::config_root());
    let harness = Harness::load(
        &overseer_core::paths::storage_dir(),
        &overseer_core::paths::config_root(),
        config.token_threshold,
        now_ms(),
    )
    .expect("load harness");
    let backend = LlmBackend::from_config(&config.llm);
    let (timer_tick, timer_batch) = (config.timer_tick_ms, config.timer_batch);
    let mut overseer = OverseerBackend::new(harness, config, backend);
    // 读通道链（docs/sidecar.md）：sidecar（路径自动发现，env 可覆盖）→ MockTerminals → Context
    let sidecar = overseer_core::paths::sidecar_exe()
        .map(overseer_core::sidecar::SidecarClient::new)
        .map(Arc::new);
    let sidecar_for_sweep = sidecar.clone();
    let sidecar_for_vd = sidecar.clone();
    overseer.sidecar_enabled = sidecar.is_some();
    let mock = Arc::new(std::sync::Mutex::new(
        std::collections::HashMap::<String, String>::new(),
    ));
    {
        let mock = mock.clone();
        overseer.terminal_reader = Some(Arc::new(move |inst: &str| {
            sidecar
                .as_ref()
                .and_then(|s| s.read_instance(inst))
                .or_else(|| mock.lock().unwrap().get(inst).cloned())
        }));
    }
    // VD 切换器（docs/hook.md §VD 切换能力）：全 VD 窗口标题匹配 → 切到目标桌面（不切回）
    {
        let sc = sidecar_for_vd.clone();
        overseer.vd_switcher = Some(Arc::new(move |inst: &str| {
            let Some(sc) = sc.as_ref() else { return false };
            let Some(resp) = sc.call(&serde_json::json!({ "cmd": "list_windows" })) else {
                return false;
            };
            let win = resp["windows"].as_array().and_then(|ws| {
                ws.iter().find(|w| {
                    w["title"]
                        .as_str()
                        .map(|t| t.contains(inst))
                        .unwrap_or(false)
                })
            });
            let Some(hwnd) = win.and_then(|w| w["hwnd"].as_i64()) else {
                return false;
            };
            sc.call(&serde_json::json!({ "cmd": "switch_to_window_desktop", "hwnd": hwnd }))
                .and_then(|r| r["switched"].as_bool())
                .unwrap_or(false)
        }));
    }
    let (tx, _) = broadcast::channel(64);
    let state = Arc::new(AppState::new(overseer, tx, mock));
    // 启动扫描（docs/hook.md §启动扫描）：全 VD 枚举注册 + N/M/K 对账进 EventBuffer
    if let Some(sc) = sidecar_for_sweep.clone() {
        let mut ov = state.overseer().lock().await;
        if let Err(e) = ov.startup_sweep(&move |req| sc.call(req), now_ms()).await {
            eprintln!("startup sweep: {e}");
        }
    }
    spawn_timer_task(state.clone(), timer_tick, timer_batch); // tick/batch 由 Config 控制（docs/timer.md）
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600")
        .await
        .expect("bind 47600");
    axum::serve(listener, app).await.expect("serve core");
}
