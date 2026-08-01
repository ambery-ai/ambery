//! step 执行器（docs/case-runner.md §steps）：batch runner 与 REPL 共用。

use overseer_core::case::CaseObserve;
use overseer_core::context::{Role, ToolCall};
use overseer_core::llm::Llm;
use overseer_core::overseer::{Effect, OverseerBackend};

/// load：storage 已于启动时 replay（占位，保持步骤可见性）
pub fn exec_load() {
    println!("(storage 已于启动时 replay)");
}

/// terminal 剧情：设定实例屏幕内容（timer_scan 时 terminal_reader 返回它）
pub fn exec_terminal(
    terminals: &std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    instance: &str,
    content: &str,
) {
    terminals
        .lock()
        .unwrap()
        .insert(instance.to_string(), content.to_string());
    println!("[OK] terminal {instance} ← {} 字", content.chars().count());
}

/// terminal_gone 剧情：实例 tab 消亡（terminal_reader 返回 None）
pub fn exec_terminal_gone(
    terminals: &std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    instance: &str,
) {
    terminals.lock().unwrap().remove(instance);
    println!("[OK] terminal_gone {instance}");
}

/// observe：按请求项分节打印（内容级，全文不截断）；context_diff 基准随行数推进
pub fn exec_observe<L: Llm>(ov: &OverseerBackend<L>, items: &[String], last_len: &mut usize) {
    let obs = overseer_core::case::observe(ov);
    print_observe(&obs, items, last_len);
}

/// timer 周期：生产路径 TimerWheel 调度（docs/timer.md）——due_timer_scans 取到期实例，
/// 读 → Some 走 handle_timer_scan（变化检测入队）；None → closed；最后放行。
/// horizon = 模拟一个 interval+stagger 已流逝（case 不等墙钟）。
/// 返回 (到期扫描数, 判 closed 数)
pub async fn exec_timer_scan<L: Llm>(ov: &mut OverseerBackend<L>, _ts: i64) -> (usize, usize) {
    let horizon = overseer_core::server::now_ms()
        + ov.config.timer.interval_ms
        + ov.config.timer.stagger_ms
        + 1;
    let due = ov.due_timer_scans(horizon, 100);
    let total = due.len();
    let mut closed = 0;
    for inst in due {
        let content = ov.terminal_reader.as_ref().and_then(|r| r(&inst));
        let ts = overseer_core::server::now_ms();
        match content {
            Some(c) => ov
                .handle_timer_scan(&inst, &c, ts)
                .await
                .expect("timer scan"),
            None => {
                ov.mark_instance_closed(&inst, ts).expect("mark closed");
                closed += 1;
            }
        }
    }
    print_effects(ov.drain_queue(0).await.expect("drain"));
    (total, closed)
}

/// hook：mock hook 事件注入（按事件分层）→ 放行
pub async fn exec_hook<L: Llm>(
    ov: &mut OverseerBackend<L>,
    event: &str,
    name: &str,
    project: &str,
    content: &str,
    ts: i64,
) {
    ov.handle_hook(event, name, project, content, ts)
        .await
        .expect("hook");
    print_effects(ov.drain_queue(0).await.expect("drain"));
}

/// trigger：放行全部待放行输入
pub async fn exec_trigger<L: Llm>(ov: &mut OverseerBackend<L>) {
    print_effects(ov.drain_queue(0).await.expect("drain"));
}

/// user：用户消息入队 → 放行
pub async fn exec_user<L: Llm>(ov: &mut OverseerBackend<L>, text: &str, ts: i64) {
    ov.enqueue(Role::User, text.to_string(), ts).expect("enqueue");
    print_effects(ov.drain_queue(0).await.expect("drain"));
}

/// tool_call：绕过 LLM 直接执行
pub async fn exec_tool_call<L: Llm>(ov: &mut OverseerBackend<L>, name: &str, args: &str, id: &str) {
    let call = ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: args.to_string(),
    };
    let (result, effects) = ov.execute_tool(&call).await;
    println!("result: {result}");
    print_effects(effects);
}

fn print_effects(effects: Vec<Effect>) {
    if !effects.is_empty() {
        println!("effects: {effects:?}");
    }
}

/// 内容级 observe 输出（docs/case-runner.md §observe 输出）
fn print_observe(obs: &CaseObserve, items: &[String], last_len: &mut usize) {
    let msg_line = |m: &overseer_core::case::MessageSnapshot| {
        format!(
            "  [{}] {}{}",
            m.role,
            m.content.as_deref().unwrap_or(""),
            if m.tool_calls > 0 { format!(" (+{} tool_calls)", m.tool_calls) } else { String::new() }
        )
    };
    for item in items {
        match item.as_str() {
            "agents" => {
                println!("agents:");
                for a in &obs.agents {
                    println!("  {} ({}) [{:?}] last_seen={}", a.name, a.hash, a.status, a.last_seen);
                }
            }
            "panorama" => match &obs.panorama {
                Some(p) => println!("panorama:\n  {}", p.replace('\n', "\n  ")),
                None => println!("panorama: (无存活实例)"),
            },
            "context" => {
                // 行首 token 摘要（#16）：真值锚点 + est 增量分开标注；无真值 = est 全量
                let tok = match &obs.usage {
                    Some(u) => format!("真值 {} + est 增量 {}", u.prompt_tokens, obs.context_est_delta),
                    None => format!("est 全量 {}", obs.context_est_delta),
                };
                println!("context: {} 行 | {}", obs.context.len(), tok);
                for m in &obs.context {
                    println!("{}", msg_line(m));
                }
            }
            "context_diff" => {
                let start = (*last_len).min(obs.context.len());
                println!("context_diff: +{} 行", obs.context.len() - start);
                for m in &obs.context[start..] {
                    println!("{}", msg_line(m));
                }
            }
            "content" => {
                println!("content: {} 行", obs.content.len());
                for c in &obs.content {
                    println!("  ({}) {}: {}", c.source, c.instance, c.content);
                }
            }
            "queue" => {
                println!("queue: {} 待放行", obs.queue.len());
                for q in &obs.queue {
                    println!("  [{:?}] {}", q.role, q.content);
                }
            }
            "event_buffer" => {
                println!("event_buffer: {} 条", obs.event_buffer.len());
                for e in &obs.event_buffer {
                    println!("  - {e}");
                }
            }
            "usage" => match &obs.usage {
                Some(u) => println!(
                    "usage: prompt_tokens={} completion_tokens={}",
                    u.prompt_tokens, u.completion_tokens
                ),
                None => println!("usage: (无真值)"),
            },
            "answer" => println!("answer: {}", obs.answer.as_deref().unwrap_or("(无)")),
            other => println!("(未知 observe 项: {other})"),
        }
    }
    *last_len = obs.context.len();
}
