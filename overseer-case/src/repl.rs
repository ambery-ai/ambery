//! 交互调试 REPL（docs/case-runner.md §REPL）：stdin 读 step 指令 → 执行 → 即时观测。
//! 支持在 case steps 之外手动驱动，方便观察即时状态和探索性调试。

use overseer_core::llm::Llm;
use overseer_core::overseer::OverseerBackend;
use std::io::Write;

use crate::runner;

pub async fn run<L: Llm>(ov: &mut OverseerBackend<L>) {
    let stdin = std::io::stdin();
    let mut last_context_len = 0usize;
    let mut seq_ts: i64 = 1_000;
    println!("overseer-case REPL（load/observe/timer_scan/hook/trigger/user/tool_call/quit）");
    loop {
        print!("> ");
        let _ = std::io::stdout().flush();
        let mut line = String::new();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            break; // EOF
        }
        seq_ts += 1;
        let ts = seq_ts;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, ' ');
        let cmd = parts.next().unwrap_or("");
        let rest = parts.next().unwrap_or("").trim();
        match cmd {
            "quit" | "exit" => break,
            "load" => {
                runner::exec_load();
                let agents = ov.harness.agents.len();
                let ctx = ov.harness.context.messages().len();
                println!("[OK] {agents} agents, {ctx} context rows");
            }
            "observe" => {
                let items: Vec<String> = if rest.is_empty() {
                    vec!["agents".into(), "panorama".into()]
                } else {
                    rest.split_whitespace().map(String::from).collect()
                };
                runner::exec_observe(ov, &items, &mut last_context_len);
            }
            "timer_scan" => {
                let (total, closed) = runner::exec_timer_scan(ov, ts).await;
                println!("[OK] timer scan: {total} scanned, {closed} closed");
            }
            "hook" => {
                // hook <event> <name> <project> [content...]
                let mut p = rest.splitn(4, ' ');
                let (event, name, project, content) = (
                    p.next().unwrap_or(""),
                    p.next().unwrap_or(""),
                    p.next().unwrap_or(""),
                    p.next().unwrap_or(""),
                );
                if event.is_empty() || name.is_empty() {
                    eprintln!("usage: hook <event> <name> <project> [content...]");
                    continue;
                }
                runner::exec_hook(ov, event, name, project, content, ts).await;
                println!("[OK] hook {event}");
            }
            "trigger" => {
                runner::exec_trigger(ov).await;
                println!("[OK] trigger");
            }
            "user" => {
                if rest.is_empty() {
                    eprintln!("usage: user <text...>");
                    continue;
                }
                runner::exec_user(ov, rest, ts).await;
                println!("[OK] user");
            }
            "tool_call" => {
                // tool_call <name> [args_json]
                let mut p = rest.splitn(2, ' ');
                let name = p.next().unwrap_or("");
                let args = p.next().unwrap_or("{}");
                if name.is_empty() {
                    eprintln!("usage: tool_call <name> [args_json]");
                    continue;
                }
                runner::exec_tool_call(ov, name, args, "repl");
            }
            other => eprintln!("未知指令: {other}（load/observe/timer_scan/hook/trigger/user/tool_call/quit）"),
        }
    }
}
