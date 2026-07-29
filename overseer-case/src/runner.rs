//! step 执行器（docs/case-runner.md §steps）：batch runner 与 REPL 共用。

use overseer_core::case::CaseObserve;
use overseer_core::context::{Role, ToolCall};
use overseer_core::llm::Llm;
use overseer_core::overseer::{Effect, OverseerBackend};

/// load：storage 已于启动时 replay（占位，保持步骤可见性）
pub fn exec_load() {
    println!("(storage 已于启动时 replay)");
}

/// observe：按请求项分节打印（内容级，全文不截断）；context_diff 基准随行数推进
pub fn exec_observe<L: Llm>(ov: &OverseerBackend<L>, items: &[String], last_len: &mut usize) {
    let obs = overseer_core::case::observe(ov);
    print_observe(&obs, items, last_len);
}

/// timer 周期：非 Closed 实例逐个读——Some → 变化检测入队；None → closed；最后放行。
/// 返回 (扫描数, 判 closed 数)
pub async fn exec_timer_scan<L: Llm>(ov: &mut OverseerBackend<L>, ts: i64) -> (usize, usize) {
    let instances: Vec<String> = ov
        .harness
        .agents
        .iter()
        .filter(|a| a.status != overseer_core::AgentStatus::Closed)
        .map(|a| a.name.clone())
        .collect();
    let total = instances.len();
    let mut closed = 0;
    for inst in instances {
        let content = ov.terminal_reader.as_ref().and_then(|r| r(&inst));
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
pub fn exec_tool_call<L: Llm>(ov: &mut OverseerBackend<L>, name: &str, args: &str, id: &str) {
    let call = ToolCall {
        id: id.to_string(),
        name: name.to_string(),
        arguments: args.to_string(),
    };
    let (result, effects) = ov.execute_tool(&call);
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
                println!("context: {} 行", obs.context.len());
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
            other => println!("(未知 observe 项: {other})"),
        }
    }
    *last_len = obs.context.len();
}
