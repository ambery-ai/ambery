//! Mock 面（隔离区）——真实 hook 接入完成后**整模块删除**
//!
//! 删除清单（一刀）：
//! 1. 本文件（core/src/mock.rs）
//! 2. Cargo.toml `[features] mock`（及 default 里的 "mock"）
//! 3. server.rs 中 `#[cfg(feature = "mock")]` 的 /debug/terminal 路由注册
//! 4. app/src-tauri/main.rs 与 core/host.rs 的 MapAdapter 兜底分支
//!
//! 约定：mock 相关代码只许进本模块 + 上述挂点，禁止外溢。

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::server::AppState;
use axum::{extract::State, response::IntoResponse, Json};

/// MockTerminals（MapAdapter 共享 map）：instance → 当前终端文本
pub type MockTerminals = Arc<Mutex<HashMap<String, String>>>;

pub fn new_terminals() -> MockTerminals {
    Arc::new(Mutex::new(HashMap::new()))
}

/// POST /debug/terminal {instance, content} → 注入「终端当前显示什么」
pub async fn post_debug_terminal(
    State(s): State<Arc<AppState>>,
    Json(body): Json<Value>,
) -> impl IntoResponse {
    let instance = body["instance"].as_str().unwrap_or("").to_string();
    let content = body["content"].as_str().unwrap_or("").to_string();
    s.mock_terminals()
        .lock()
        .unwrap()
        .insert(instance, content);
    Json(json!({ "ok": true }))
}

/// POST /debug/effect {kind, ...} → 向 effect 下行总线广播任意 effect 消息
/// （前端进 case v2 的注入面：headless 测试确定性驱动 render/close/config 等事件，
///  不经 LLM；消息原样进 sender，WS 订阅方按 kind 分发）
pub async fn post_debug_effect(
    State(s): State<Arc<AppState>>,
    Json(msg): Json<Value>,
) -> impl IntoResponse {
    s.broadcast_effect_json(msg).await;
    Json(json!({ "ok": true }))
}

