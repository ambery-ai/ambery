//! Rust 壳侧非只读 Tauri 运行时动作的唯一出口（docs/effect-reporting.md §运行时动作层）：
//! 语义化动作 = 真实 tauri API 写调用，成功后经同一记录入口写对应 effect（失败不写）。
//! 记录 best-effort、不得阻断主动作（state 未就绪只跳过记录）；只读调用不进本层。
//! 动作词表与 WebView 侧（app/src/tauri_runtime_actions.ts）共享，不共享跨语言代码。

use crate::SharedTauriState;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager};

/// 同一记录入口：record_frontend_effect → harness.log_effect(Frontend, ...)
/// （origin=frontend 不区分 WebView 与 Rust 壳的实现位置）
pub(crate) fn record<R: tauri::Runtime>(app: &AppHandle<R>, kind: &'static str, payload: Value) {
    let mgr = app.state::<SharedTauriState>().inner().clone();
    tauri::async_runtime::spawn(async move {
        let Some(s) = mgr.0.lock().unwrap().clone() else {
            eprintln!("[effect] state 未就绪，跳过记录 {kind}");
            return;
        };
        s.overseer().lock().await.record_frontend_effect(kind, payload);
    });
}

/// show_window：show 成功 → window_visible {window}
pub fn show_window<R: tauri::Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        if w.show().is_ok() {
            record(app, "window_visible", json!({ "window": label }));
        }
    }
}

/// hide_window：hide 成功 → window_hidden {window}
pub fn hide_window<R: tauri::Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        if w.hide().is_ok() {
            record(app, "window_hidden", json!({ "window": label }));
        }
    }
}

/// move_window：set_position 成功 → window_moved {window, x, y}
pub fn move_window<R: tauri::Runtime>(app: &AppHandle<R>, label: &str, x: i32, y: i32) {
    if let Some(w) = app.get_webview_window(label) {
        if w.set_position(tauri::PhysicalPosition::new(x, y)).is_ok() {
            record(app, "window_moved", json!({ "window": label, "x": x, "y": y }));
        }
    }
}

/// close_window：close 成功 → window_closed {window}
#[allow(dead_code)] // 词表对齐保留；当前 Rust 壳只藏不关（toggle/托盘语义）
pub fn close_window<R: tauri::Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        if w.close().is_ok() {
            record(app, "window_closed", json!({ "window": label }));
        }
    }
}

/// emit_event：emit 成功 → event_emit {event}
pub fn emit_event<R: tauri::Runtime>(app: &AppHandle<R>, event: &'static str, payload: Value) {
    if app.emit(event, payload).is_ok() {
        record(app, "event_emit", json!({ "event": event }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use overseer_core::llm::LlmBackend;
    use overseer_core::overseer::OverseerBackend;
    use overseer_core::server::AppState;
    use overseer_core::{Config, Harness};
    use std::sync::Arc;

    fn harness_state(tag: &str) -> (Arc<AppState>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("overseer-actions-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let config = Config::load_or_default(&dir);
        let harness = Harness::load(&dir, &dir, config.effective_compression_limit().unwrap_or(usize::MAX), 0).unwrap();
        let backend = LlmBackend::from_config(&config.llm);
        let ov = OverseerBackend::new(harness, config, backend);
        let mock = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
        (Arc::new(AppState::new(ov, mock)), dir)
    }

    fn mock_app() -> tauri::App<tauri::test::MockRuntime> {
        tauri::test::mock_builder()
            .manage(SharedTauriState::new(crate::TauriState(std::sync::Mutex::new(None))))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap()
    }

    /// 轮询等 effect.jsonl 出现目标子串（记录任务异步落盘；超时即 None）
    fn wait_effect_file(dir: &std::path::Path, needle: &str) -> Option<String> {
        for _ in 0..60 {
            let content = std::fs::read_to_string(dir.join(overseer_core::EFFECT_FILE)).unwrap_or_default();
            if content.contains(needle) {
                return Some(content);
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        None
    }

    #[tokio::test]
    async fn hide_window_records_window_hidden_after_success() {
        let app = mock_app();
        let (state, dir) = harness_state("hide");
        *app.state::<SharedTauriState>().0.lock().unwrap() = Some(state);
        tauri::WebviewWindowBuilder::new(&app, "pet", Default::default())
            .build()
            .unwrap();
        hide_window(app.handle(), "pet");
        let content = wait_effect_file(&dir, "window_hidden").expect("hide 成功后应记录 window_hidden");
        assert!(content.contains("\"window\":\"pet\""), "{content}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_window_records_nothing() {
        let app = mock_app();
        let (state, dir) = harness_state("miss");
        *app.state::<SharedTauriState>().0.lock().unwrap() = Some(state);
        hide_window(app.handle(), "no-such-window");
        emit_event(app.handle(), "pet:shown", json!(()));
        // emit 无监听者也 Ok → event_emit 会记；窗口动作找不到窗口 = 未执行 = 不记
        let content = wait_effect_file(&dir, "event_emit").expect("emit 成功应记 event_emit");
        assert!(!content.contains("window_hidden"), "窗口不存在不得记 effect: {content}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
