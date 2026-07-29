//! overseer-case CLI（docs/case-runner.md）：storage 快照回放、step 执行与概念观测。

use overseer_core::case::CaseFile;
use overseer_core::context::{Role, ToolCall};
use overseer_core::llm::LlmBackend;
use overseer_core::overseer::{Effect, OverseerBackend};
use overseer_core::server::now_ms;
use overseer_core::Config;
use std::sync::Arc;

mod export;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: overseer-case <case.json> [--step-num N] [--health]");
        eprintln!("       overseer-case export [--storage DIR] [--instances a,b] [--window 30m]");
        eprintln!("              [--before TS] [--after TS] [--keep-last N] [--trim-context] [--dedup] [--case-id ID]");
        std::process::exit(2);
    }
    if args[1] == "export" {
        run_export(&args[2..]);
        return;
    }
    let case_path = &args[1];
    let health_mode = args.iter().any(|a| a == "--health");
    let max_step = args
        .iter()
        .position(|a| a == "--step-num")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse::<usize>().ok());

    let case_json = std::fs::read_to_string(case_path).expect("read case file");
    let case: CaseFile = serde_json::from_str(&case_json).expect("parse case");

    if health_mode {
        health(&case);
        return;
    }

    // 临时目录写 storage 快照
    let tmp = std::env::temp_dir().join(format!("overseer-case-{}", case.meta.case_id));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tmp dir");

    write_jsonl(&tmp, "work-agents.jsonl", &case.data.work_agents);
    write_jsonl(&tmp, "context.jsonl", &case.data.context);
    write_jsonl(&tmp, "queue.jsonl", &case.data.queue);

    let mut config = Config::load_or_default(&tmp);
    // case 确定性：强制 DebugAgent 沉默决策源（不碰网络；LLM 行为用 steps 剧本驱动）
    config.llm.active = "debug".into();
    config.timer_interval_ms = case
        .data
        .config
        .get("timer_interval_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(300_000);
    config.timer_tick_ms = case
        .data
        .config
        .get("timer_tick_ms")
        .and_then(|v| v.as_i64())
        .unwrap_or(5_000) as u64;

    let harness = overseer_core::Harness::load(&tmp, &tmp, config.token_threshold, now_ms())
        .expect("load harness");
    let backend = LlmBackend::from_config(&config.llm);
    let mut ov = OverseerBackend::new(harness, config, backend);

    // 读通道 mock（MockTerminals）：指定 instance 返回内容，未指定返回 None（= tab 不复存在）
    let mock = Arc::new(std::sync::Mutex::new(case.data.mock_terminals.clone()));
    let mock_for_reader = mock.clone();
    ov.terminal_reader = Some(Arc::new(move |inst: &str| {
        mock_for_reader.lock().unwrap().get(inst).cloned()
    }));

    // 合成 ts：steps 未带 ts 时按序递增（回放确定性）
    let mut seq_ts: i64 = 1_000;
    let mut pick_ts = |v: Option<&serde_json::Value>| -> i64 {
        seq_ts += 1;
        v.and_then(|v| v.as_i64()).unwrap_or(seq_ts)
    };
    // context_diff 基准：上次 observe 时的 Context 行数
    let mut last_context_len = 0usize;

    for (i, step) in case.steps.iter().enumerate() {
        if let Some(n) = max_step {
            if i >= n {
                break;
            }
        }
        println!("── step {}: {} ──", i + 1, step.name());
        match step {
            overseer_core::case::CaseStep::Observe { observe: items } => {
                let obs = overseer_core::case::observe(&ov);
                print_observe(&obs, items, &mut last_context_len);
            }
            overseer_core::case::CaseStep::Cmd { .. } => {
                println!("(storage 已于启动时 replay)");
            }
            overseer_core::case::CaseStep::Cmd2 { .. } => {
                // timer 周期：非 closed 实例逐个读——Some → 变化检测入队；None → closed
                let instances: Vec<String> = ov
                    .harness
                    .agents
                    .iter()
                    .filter(|a| a.status != overseer_core::AgentStatus::Closed)
                    .map(|a| a.name.clone())
                    .collect();
                let ts = pick_ts(None);
                for inst in instances {
                    let content = ov.terminal_reader.as_ref().and_then(|r| r(&inst));
                    match content {
                        Some(c) => ov
                            .handle_timer_scan(&inst, &c, ts)
                            .await
                            .expect("timer scan"),
                        None => ov.mark_instance_closed(&inst, ts).expect("mark closed"),
                    }
                }
                print_effects(ov.drain_queue(0).await.expect("drain"));
            }
            overseer_core::case::CaseStep::Cmd3 { hook } => {
                let event = hook["event"].as_str().expect("hook.event");
                let name = hook["name"].as_str().unwrap_or("");
                let project = hook["project"].as_str().unwrap_or("");
                let content = hook["content"].as_str().unwrap_or("");
                let ts = pick_ts(hook.get("ts"));
                ov.handle_hook(event, name, project, content, ts)
                    .await
                    .expect("hook");
                print_effects(ov.drain_queue(0).await.expect("drain"));
            }
            overseer_core::case::CaseStep::Cmd4 { .. } => {
                print_effects(ov.drain_queue(0).await.expect("drain"));
            }
            overseer_core::case::CaseStep::Cmd5 { user } => {
                let text = user["text"].as_str().expect("user.text");
                let ts = pick_ts(user.get("ts"));
                ov.enqueue(Role::User, text.to_string(), ts).expect("enqueue");
                print_effects(ov.drain_queue(0).await.expect("drain"));
            }
            overseer_core::case::CaseStep::Cmd6 { tool_call } => {
                let name = tool_call.first().expect("tool_call[0] = name");
                let args = tool_call.get(1).map(String::as_str).unwrap_or("{}");
                let call = ToolCall {
                    id: format!("case-{i}"),
                    name: name.clone(),
                    arguments: args.to_string(),
                };
                let (result, effects) = ov.execute_tool(&call);
                println!("result: {result}");
                print_effects(effects);
            }
        }
    }
}

fn print_effects(effects: Vec<Effect>) {
    if !effects.is_empty() {
        println!("effects: {effects:?}");
    }
}

/// 内容级 observe 输出（docs/case-runner.md §observe 输出）：按请求项分节打印，全文不截断
fn print_observe(obs: &overseer_core::case::CaseObserve, items: &[String], last_len: &mut usize) {
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

fn health(case: &CaseFile) {
    let mut ok = true;
    // 1. JSONL 可解析
    for (name, content) in &[
        ("work_agents", &case.data.work_agents),
        ("context", &case.data.context),
        ("queue", &case.data.queue),
    ] {
        if content.is_empty() {
            continue;
        }
        for (i, line) in content.lines().enumerate() {
            if serde_json::from_str::<serde_json::Value>(line).is_err() {
                eprintln!("FAIL: {name} line {}: invalid JSON", i + 1);
                ok = false;
            }
        }
    }
    // 2. meta 必填字段
    if case.meta.case_id.is_empty() {
        eprintln!("FAIL: meta.case_id empty");
        ok = false;
    }
    if case.meta.created.is_empty() {
        eprintln!("FAIL: meta.created empty");
        ok = false;
    }
    // 3. 概念结构完整性：work_agents 每行有必填字段
    for (i, line) in case.data.work_agents.lines().enumerate() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            for f in &["hash", "name", "project", "status"] {
                if v.get(f).is_none() {
                    eprintln!("FAIL: work_agents line {}: missing {}", i + 1, f);
                    ok = false;
                }
            }
        }
    }
    if ok {
        println!("PASS");
        std::process::exit(0);
    } else {
        std::process::exit(1);
    }
}

fn write_jsonl(dir: &std::path::Path, name: &str, content: &str) {
    if content.is_empty() {
        return;
    }
    std::fs::write(dir.join(name), content).expect("write jsonl");
}

/// 实时 storage → case 导出（docs/case-runner.md §导出工具）：过滤 → 最小化 → JSON 输出
fn run_export(args: &[String]) {
    let opt_val = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let has = |flag: &str| args.iter().any(|a| a == flag);
    let storage_dir = opt_val("--storage")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(overseer_core::paths::storage_dir);
    let case_id = opt_val("--case-id").unwrap_or_else(|| {
        format!("export-{}", std::process::id())
    });
    let opts = export::ExportOpts {
        case_id,
        notes: "导出自实时 storage，需手修 meta/notes 与 steps".into(),
        instances: opt_val("--instances")
            .map(|s| s.split(',').map(|x| x.trim().to_string()).collect()),
        before: opt_val("--before").and_then(|v| v.parse().ok()),
        after: opt_val("--after").and_then(|v| v.parse().ok()),
        window_ms: opt_val("--window").and_then(|v| export::parse_duration(&v)),
        keep_last: opt_val("--keep-last").and_then(|v| v.parse().ok()),
        trim_context: has("--trim-context"),
        dedup: has("--dedup"),
    };
    println!("{}", export::export(&storage_dir, &opts));
}
