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

/// observe：按请求项分节打印（docs/case-runner.md §observe 输出）——
/// 值类直接给当前值；路径类（context/effects）无 lines 给文件指针+摘要，
/// 带 lines 打印文件切片原文（含行号；表达式求值见 docs/case-eval-system.md）
pub fn exec_observe<L: Llm>(
    ov: &OverseerBackend<L>,
    items: &[overseer_core::case::ObserveItem],
    vars: &std::collections::HashMap<String, String>,
) {
    let obs = overseer_core::case::observe(ov);
    print_observe(&obs, items, vars, ov.harness.storage_dir());
}

/// store：设用户变量（value 经对应 parser 求值 → to_string → 存 string）。
/// $tail 绑定规则（case-eval-system.md §变量）：store 求值 = context.jsonl 末行号。
pub fn exec_store<L: Llm>(
    ov: &OverseerBackend<L>,
    map: &std::collections::HashMap<String, overseer_core::case::StoreValue>,
    vars: &mut std::collections::HashMap<String, String>,
) {
    let tail = read_file_lines(&ov.harness.storage_dir().join(overseer_core::CONTEXT_FILE)).len() as i64;
    let env = overseer_core::eval::VarEnv { tail, vars: vars.clone() };
    for (name, sv) in map {
        match overseer_core::eval::eval_store(&env, &sv.ty, &sv.value) {
            Ok(v) => {
                println!("[OK] store ${name} = {v}（{}: {:?}）", sv.ty, sv.value);
                vars.insert(name.clone(), v);
            }
            Err(e) => println!("[FAIL] store ${name}（{}: {:?}）: {e}", sv.ty, sv.value),
        }
    }
}

/// 读文件全部行（缺失 = 空）
fn read_file_lines(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
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
fn print_observe(
    obs: &CaseObserve,
    items: &[overseer_core::case::ObserveItem],
    vars: &std::collections::HashMap<String, String>,
    storage_dir: &std::path::Path,
) {
    for item in items {
        // 路径类之外的 target 带 lines = 非法（health pre-parse 静态拦截；运行期防御打印）
        if item.lines.is_some() && !item.is_path_class() {
            println!("({} 是值类，不支持 lines；路径类：context/effects)", item.target);
            continue;
        }
        match item.target.as_str() {
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
            "filtered_content" => {
                println!("filtered_content: {} 行", obs.content.len());
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
            "usage" => match (&obs.usage, obs.usage_ts) {
                (Some(u), ts) => println!(
                    "usage: prompt_tokens={} completion_tokens={} ts={}",
                    u.prompt_tokens,
                    u.completion_tokens,
                    ts.map(|t| t.to_string()).unwrap_or_else(|| "(无)".into())
                ),
                (None, _) => println!("usage: (无真值)"),
            },
            "answer" => println!("answer: {}", obs.answer.as_deref().unwrap_or("(无)")),
            "context" => {
                // 路径类：无 lines → 文件指针+摘要（行首 token 标注，#16）；带 lines → 切片原文
                let path = storage_dir.join(overseer_core::CONTEXT_FILE);
                let tok = match &obs.usage {
                    Some(u) => format!("真值 {} + est 增量 {}", u.prompt_tokens, obs.context_est_delta),
                    None => format!("est 全量 {}", obs.context_est_delta),
                };
                print_path_slice("context", &path, &item.lines, vars, &format!("行 | {tok}"));
            }
            "effects" => {
                // 路径类：无 lines → 文件指针+摘要（条数）；带 lines → 切片原文
                let path = storage_dir.join(overseer_core::EFFECT_FILE);
                print_path_slice("effects", &path, &item.lines, vars, "条");
            }
            other => println!("(未知 observe 项: {other})"),
        }
    }
}

/// 路径类输出：无 lines 打印 `路径 | N <单位/摘要>`；带 lines 经 RangeParser 求值切片打印原文
fn print_path_slice(
    name: &str,
    path: &std::path::Path,
    lines: &Option<String>,
    vars: &std::collections::HashMap<String, String>,
    summary: &str,
) {
    let file_lines = read_file_lines(path);
    let tail = file_lines.len() as i64;
    match lines {
        None => println!("{name}: {} | {tail} {summary}", path.display()),
        Some(expr) => {
            let env = overseer_core::eval::VarEnv { tail, vars: vars.clone() };
            use overseer_core::eval::Parser;
            let parsed = (overseer_core::eval::RangeParser { env: &env }).parse(expr.as_str());
            match parsed {
                Ok((range, rest)) if rest.is_empty() => match range.resolve(tail) {
                    Some((start, end)) => {
                        println!("{name}: {} | 切片 [{start},{end}]（共 {tail} 行）", path.display());
                        for (i, l) in file_lines[(start - 1) as usize..end as usize].iter().enumerate() {
                            println!("  {}: {}", start + i as i64, l);
                        }
                    }
                    None => println!("{name}: 空切片（{expr:?} @ tail={tail}）"),
                },
                Ok((_, rest)) => println!("{name}: lines 语法错误（多余内容 {rest:?}）: {expr:?}"),
                Err(e) => println!("{name}: lines 解析失败: {e}"),
            }
        }
    }
}
