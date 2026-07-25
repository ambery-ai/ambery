//! Tauri 壳（docs/tauri-shell.md）：单透明 overlay 窗口 + 内嵌 overseer-core server。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use overseer_core::llm::DebugAgent;
use overseer_core::overseer::Overseer;
use overseer_core::server::{now_ms, router, spawn_timer_task, AppState};
use overseer_core::{Config, Harness};
use std::sync::Arc;
use tauri::Manager;
use tokio::sync::broadcast;

fn main() {
    // 内嵌 overseer-core（spec.md 架构决定 #1：前端始终走 HTTP+WS loopback）
    std::thread::spawn(|| {
        let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
        rt.block_on(run_core());
    });

    tauri::Builder::default()
        .setup(|app| {
            let win = app.get_webview_window("main").expect("main window");
            let raw = win.hwnd().expect("hwnd").0 as *mut core::ffi::c_void;
            // 跨虚拟桌面 pin（mock-a 方案：IVirtualDesktopPinnedApps::PinWindow，
            // 切桌面时ペット跟着走；winvd 0.0.49 要求 Win11 24H2 26100.2605+）
            if let Err(err) = winvd::pin_window(windows::Win32::Foundation::HWND(raw)) {
                eprintln!("winvd pin_window: {err:?}");
            }
            let hwnd = raw as isize;
            // 任务栏 fight-back（mock-a 方案）：每 500ms 安静地重申 TOPMOST，
            // SWP_NOACTIVATE 不抢焦点、SWP_NOMOVE/SWP_NOSIZE 不动几何
            std::thread::spawn(move || loop {
                std::thread::sleep(std::time::Duration::from_millis(500));
                unsafe {
                    SetWindowPos(
                        hwnd,
                        HWND_TOPMOST,
                        0,
                        0,
                        0,
                        0,
                        SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                    );
                }
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

const HWND_TOPMOST: isize = -1;
const SWP_NOACTIVATE: u32 = 0x10;
const SWP_NOMOVE: u32 = 0x2;
const SWP_NOSIZE: u32 = 0x1;

extern "system" {
    fn SetWindowPos(
        h_wnd: isize,
        h_wnd_insert_after: isize,
        x: i32,
        y: i32,
        cx: i32,
        cy: i32,
        u_flags: u32,
    ) -> i32;
}

async fn run_core() {
    let dir = std::env::var("OVERSEER_STORAGE").unwrap_or_else(|_| "storage".into());
    let dir = std::path::Path::new(&dir);
    let config = Config::load_or_default(dir);
    let harness = Harness::load(
        dir,
        config.base_prompt.clone(),
        config.token_threshold,
        now_ms(),
    )
    .expect("load harness");
    let mut overseer = Overseer::new(harness, config, DebugAgent::default());
    // 读通道链（docs/sidecar.md）：sidecar → MockTerminals → Context
    let sidecar = std::env::var("OVERSEER_SIDECAR")
        .ok()
        .map(overseer_core::sidecar::SidecarClient::new)
        .map(Arc::new);
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
    let (tx, _) = broadcast::channel(64);
    let state = Arc::new(AppState::new(overseer, tx, mock));
    spawn_timer_task(state.clone(), 60_000, 2); // 真实 tick：60s（Config 的 5min 间隔由 TimerWheel 控制）
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:47600")
        .await
        .expect("bind 47600");
    axum::serve(listener, app).await.expect("serve core");
}
