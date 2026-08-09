//! debug 模式入口：overseer-core + DebugAgent + HTTP/WS server。
//! 用法：`cargo run --bin overseer-debug`（storage/config 目录见 core/paths.rs env 覆盖）
//! 装配骨架与 overseer-case serve 共用 core::host（docs/case-runner.md §壳类比）。

use overseer_core::context::{ContextMessage, ToolCall};
use overseer_core::llm::{DebugAgent, LlmBackend, LlmOutput};

/// debug CLI 决策源：人/外部脚本当 LLM（mock 零逻辑，决策全来自 stdin）。
/// 每次调用把全量 Queue 以机读帧打到 stdout（@@@ 前缀与 server 日志区分），
/// 再从 stdin 读响应：`c <文本>` | `t <tool名> <json参数>`（可多行）| 空行提交。
/// 外部脚本（如 scripts/debug_brain.py）接管本进程 stdin/stdout 即可驱动。
fn cli_decide(messages: &[ContextMessage]) -> LlmOutput {
    use std::io::BufRead;
    println!("@@@QUEUE_BEGIN");
    for (i, m) in messages.iter().enumerate() {
        println!(
            "{}",
            serde_json::json!({
                "i": i,
                "role": m.role,
                "content": m.content,
                "tool_calls": m.tool_calls,
                "tool_call_id": m.tool_call_id,
            })
        );
    }
    println!("@@@QUEUE_END");
    println!("@@@PROMPT c <文本> | t <tool名> <json参数> | 空行提交");
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    let mut content = None;
    let mut tool_calls = vec![];
    loop {
        let mut line = String::new();
        match lock.read_line(&mut line) {
            Ok(0) | Err(_) => break, // EOF：按已输入内容提交
            Ok(_) => {}
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(text) = line.strip_prefix("c ") {
            content = Some(text.to_string());
        } else if let Some(rest) = line.strip_prefix("t ") {
            match rest.split_once(' ') {
                Some((name, args)) => tool_calls.push(ToolCall {
                    id: format!("cli-{}", tool_calls.len() + 1),
                    name: name.to_string(),
                    arguments: args.to_string(),
                }),
                None => println!("@@@ERR 格式：t <tool名> <json参数>"),
            }
        } else {
            println!("@@@ERR 无法识别：c <文本> | t <tool名> <json参数> | 空行提交");
        }
    }
    LlmOutput {
        content,
        tool_calls,
        reasoning_content: None,
        usage: None,
    }
}

#[tokio::main]
async fn main() {
    // debug 分支换 CLI 决策源（OpenAi 变体的内部降级仍是沉默 mock）
    let parts = overseer_core::host::assemble_host(|backend| match backend {
        LlmBackend::Debug(_) => {
            println!("debug 决策源：CLI（@@@ 帧）");
            LlmBackend::Debug(DebugAgent::new(cli_decide))
        }
        b => b,
    });
    let port: u16 = std::env::var("OVERSEER_PORT")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(47600);
    overseer_core::host::serve_host(parts, port).await;
}
