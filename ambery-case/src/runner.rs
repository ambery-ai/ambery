//! step 执行器（docs/case-runner.md §steps）。

use ambery_core::case::CaseObserve;
use ambery_core::context::{Role, ToolCall};
use ambery_core::llm::Llm;
use ambery_core::ambery::{Effect, AmberyBackend};

/// load：storage 已于启动时 replay（占位，保持步骤可见性）
pub fn exec_load() {
    println!("(storage 已于启动时 replay)");
}

/// terminal 剧情：设定实例屏幕内容（写 MapAdapter 共享 map，timer_scan/fetch 经它读到）
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

/// terminal_gone 剧情：实例 tab 消亡（MapAdapter 移除内容，locate/read 返回 None）
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
    ov: &AmberyBackend<L>,
    items: &[ambery_core::case::ObserveItem],
    vars: &std::collections::HashMap<String, String>,
) {
    let obs = ambery_core::case::observe(ov);
    print_observe(&obs, items, vars, ov.harness.storage_dir());
}

/// store：设用户变量（value 经对应 parser 求值 → to_string → 存 string）。
/// $tail 绑定规则（case-eval-system.md §变量）：store 求值 = context.jsonl 末行号。
pub fn exec_store<L: Llm>(
    ov: &AmberyBackend<L>,
    map: &std::collections::HashMap<String, ambery_core::case::StoreValue>,
    vars: &mut std::collections::HashMap<String, String>,
) {
    let tail = read_file_lines(&ov.harness.storage_dir().join(ambery_core::CONTEXT_FILE)).len() as i64;
    let env = ambery_core::eval::VarEnv { tail, vars: vars.clone() };
    for (name, sv) in map {
        match ambery_core::eval::eval_store(&env, &sv.ty, &sv.value) {
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
pub async fn exec_timer_scan<L: Llm>(ov: &mut AmberyBackend<L>, _ts: i64) -> (usize, usize) {
    let horizon = ambery_core::server::now_ms()
        + ov.config.timer.interval_ms
        + ov.config.timer.stagger_ms
        + 1;
    let due = ov.due_timer_scans(horizon, 100);
    let total = due.len();
    let mut closed = 0;
    for inst in due {
        let content = ov.read_terminal(&inst);
        let ts = ambery_core::server::now_ms();
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
    ov: &mut AmberyBackend<L>,
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
pub async fn exec_trigger<L: Llm>(ov: &mut AmberyBackend<L>) {
    print_effects(ov.drain_queue(0).await.expect("drain"));
}

/// user：用户消息入队 → 放行
pub async fn exec_user<L: Llm>(ov: &mut AmberyBackend<L>, text: &str, ts: i64) {
    ov.enqueue(Role::User, text.to_string(), ambery_core::queue::QueueSource::UserChat, ts).expect("enqueue");
    print_effects(ov.drain_queue(0).await.expect("drain"));
}

/// tool_call：绕过 LLM 直接执行
pub async fn exec_tool_call<L: Llm>(ov: &mut AmberyBackend<L>, name: &str, args: &str, id: &str) {
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
    items: &[ambery_core::case::ObserveItem],
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
                println!("filtered_content: {} 行（现算）", obs.filtered_content.len());
                for c in &obs.filtered_content {
                    println!("  ({}) {}: {}", c.source, c.instance, c.filtered_content);
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
            "memory" => {
                // Memory index 摘要（name / description / 总数）；不默认展开正文
                println!("memory: Memory index 摘要（{} 条）", obs.memory.len());
                for n in &obs.memory {
                    println!("  {} | {}", n.name, n.description);
                }
            }
            "cron" => {
                // 持久化计划投影（id / schedule / message / next_due）；不含 sleep waiter
                println!("cron: {} 个持久化计划", obs.cron.len());
                for e in &obs.cron {
                    let schedule = match e.schedule {
                        ambery_core::cron::Schedule::At(ts) => format!("at {ts}"),
                        ambery_core::cron::Schedule::EveryMs(ms) => format!("every {ms}ms"),
                    };
                    let next = e
                        .next_due
                        .map(|d| format!("next_due={d}"))
                        .unwrap_or_else(|| "完成态".into());
                    println!("  {} | {} | {} | {}", e.id, schedule, next, e.message);
                }
            }
            "cards" => {
                // Card 注册表投影（id/typ/title/created/user_closed/layout；不展开 component）
                println!("cards: {} 张存活", obs.cards.len());
                for c in &obs.cards {
                    let visibility = if c.user_closed { "user_closed" } else { "visible" };
                    let layout = match (c.layout.offset, c.layout.manual) {
                        (Some((x, y)), true) => format!("manual({x},{y})"),
                        (Some((x, y)), false) => format!("offset({x},{y})"),
                        _ => c
                            .layout
                            .direction
                            .as_deref()
                            .map(|d| format!("auto/{d}"))
                            .unwrap_or_else(|| "auto".into()),
                    };
                    println!("  {} | {}「{}」| {} | {}", c.id, c.typ, c.title, visibility, layout);
                }
            }
            "context" => {
                // 路径类：无 lines → 文件指针+摘要（行首 token 标注，#16）；带 lines → 切片原文
                let path = storage_dir.join(ambery_core::CONTEXT_FILE);
                let tok = match &obs.usage {
                    Some(u) => format!("真值 {} + est 增量 {}", u.prompt_tokens, obs.context_est_delta),
                    None => format!("est 全量 {}", obs.context_est_delta),
                };
                print_path_slice("context", &path, &item.lines, vars, &format!("行 | {tok}"));
            }
            "effects" => {
                // 路径类：无 lines → 文件指针+摘要（条数）；带 lines → 切片原文
                let path = storage_dir.join(ambery_core::EFFECT_FILE);
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
            let env = ambery_core::eval::VarEnv { tail, vars: vars.clone() };
            use ambery_core::eval::Parser;
            let parsed = (ambery_core::eval::RangeParser { env: &env }).parse(expr.as_str());
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
