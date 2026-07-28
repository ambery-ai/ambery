//! overseer-case CLI（docs/case-runner.md）：storage 快照回放与概念观测。

use overseer_core::case::CaseFile;
use overseer_core::server::{now_ms, AppState};
use overseer_core::Config;
use std::io::Write;
use std::sync::Arc;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: overseer-case <case.json> [--step-num N] [--health]");
        std::process::exit(2);
    }
    let case_path = &args[1];
    let health_mode = args.iter().any(|a| a == "--health");
    let max_step = args.iter().position(|a| a == "--step-num")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse::<usize>().ok());

    let case_json = std::fs::read_to_string(case_path).expect("read case file");
    let case: CaseFile = serde_json::from_str(&case_json).expect("parse case");

    if health_mode { health(&case); return; }

    // 临时目录写 storage 快照
    let tmp = std::env::temp_dir().join(format!("overseer-case-{}", case.meta.case_id));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tmp dir");

    write_jsonl(&tmp, "work-agents.jsonl", &case.data.work_agents);
    write_jsonl(&tmp, "context.jsonl", &case.data.context);
    write_jsonl(&tmp, "queue.jsonl", &case.data.queue);

    let mut config = Config::load_or_default(&tmp);
    // 注入 mock_terminals
    let mock = Arc::new(std::sync::Mutex::new(case.data.mock_terminals.clone()));
    config.timer_interval_ms = case.data.config.get("timer_interval_ms")
        .and_then(|v| v.as_i64()).unwrap_or(300_000);
    config.timer_tick_ms = case.data.config.get("timer_tick_ms")
        .and_then(|v| v.as_i64()).unwrap_or(5_000) as u64;

    let harness = overseer_core::Harness::load(&tmp, &tmp, config.token_threshold, now_ms())
        .expect("load harness");
    let backend = overseer_core::llm::LlmBackend::from_config(&config.llm);
    let overseer = overseer_core::overseer::OverseerBackend::new(harness, config, backend);

    let mut mock_for_reader = mock.clone();
    let mut ov = overseer;
    ov.terminal_reader = Some(Arc::new(move |inst: &str| {
        mock_for_reader.lock().unwrap().get(inst).cloned()
    }));
    let state = Arc::new(AppState::new(ov, mock));

    for (i, step) in case.steps.iter().enumerate() {
        if let Some(n) = max_step { if i >= n { break; } }
        println!("── step {}: {} ──", i + 1, step.name());
        match step {
            _ => {}
        }
        if matches!(step.name(), "observe") {
            let obs = overseer_core::case::observe(&state);
            println!("{}", serde_json::to_string_pretty(&obs).unwrap());
        }
    }
}

fn health(case: &CaseFile) {
    let mut ok = true;
    // 1. JSONL 可解析
    for (name, content) in &[("work_agents", &case.data.work_agents), ("context", &case.data.context), ("queue", &case.data.queue)] {
        if content.is_empty() { continue; }
        for (i, line) in content.lines().enumerate() {
            if serde_json::from_str::<serde_json::Value>(line).is_err() {
                eprintln!("FAIL: {name} line {}: invalid JSON", i + 1);
                ok = false;
            }
        }
    }
    // 2. meta 必填字段
    if case.meta.case_id.is_empty() { eprintln!("FAIL: meta.case_id empty"); ok = false; }
    if case.meta.created.is_empty() { eprintln!("FAIL: meta.created empty"); ok = false; }
    // 3. 概念结构完整性：work_agents 每行有必填字段
    for (i, line) in case.data.work_agents.lines().enumerate() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            for f in &["hash", "name", "project", "status"] {
                if v.get(f).is_none() { eprintln!("FAIL: work_agents line {}: missing {}", i + 1, f); ok = false; }
            }
        }
    }
    if ok { println!("PASS"); std::process::exit(0); }
    else { std::process::exit(1); }
}

fn write_jsonl(dir: &std::path::Path, name: &str, content: &str) {
    if content.is_empty() { return; }
    std::fs::write(dir.join(name), content).expect("write jsonl");
}
