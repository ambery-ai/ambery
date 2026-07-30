//! overseer-case CLI（docs/case-runner.md）：两段式 .case 回放、step 执行与概念观测。

use overseer_core::case::CaseFile;
use overseer_core::llm::LlmBackend;
use overseer_core::overseer::OverseerBackend;
use overseer_core::server::now_ms;
use overseer_core::Config;
use std::sync::Arc;

mod export;
mod runner;

type SharedTerminals = Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>;

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: overseer-case <case.case> [--step-num N] [--health]");
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

    let text = std::fs::read_to_string(case_path).expect("read case file");
    let case = overseer_core::case::parse(&text).expect("parse case（两段式 .case）");

    if health_mode {
        health(&text, &case);
        return;
    }

    let (mut ov, terminals) = setup(&case);

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
                runner::exec_observe(&ov, items, &mut last_context_len);
            }
            overseer_core::case::CaseStep::Cmd { .. } => runner::exec_load(),
            overseer_core::case::CaseStep::Cmd2 { .. } => {
                runner::exec_timer_scan(&mut ov, pick_ts(None)).await;
            }
            overseer_core::case::CaseStep::Cmd3 { hook } => {
                let event = hook["event"].as_str().expect("hook.event");
                let name = hook["name"].as_str().unwrap_or("");
                let project = hook["project"].as_str().unwrap_or("");
                let content = hook["content"].as_str().unwrap_or("");
                let ts = pick_ts(hook.get("ts"));
                runner::exec_hook(&mut ov, event, name, project, content, ts).await;
            }
            overseer_core::case::CaseStep::Cmd4 { .. } => {
                runner::exec_trigger(&mut ov).await;
            }
            overseer_core::case::CaseStep::Cmd5 { user } => {
                let text = user["text"].as_str().expect("user.text");
                let ts = pick_ts(user.get("ts"));
                runner::exec_user(&mut ov, text, ts).await;
            }
            overseer_core::case::CaseStep::Cmd6 { tool_call } => {
                let name = tool_call.first().expect("tool_call[0] = name");
                let args = tool_call.get(1).map(String::as_str).unwrap_or("{}");
                runner::exec_tool_call(&mut ov, name, args, &format!("case-{i}"));
            }
            overseer_core::case::CaseStep::Terminal { terminal } => {
                let instance = terminal["instance"].as_str().expect("terminal.instance");
                let content = terminal["content"].as_str().unwrap_or("");
                runner::exec_terminal(&terminals, instance, content);
            }
            overseer_core::case::CaseStep::TerminalGone { terminal_gone } => {
                let instance = terminal_gone["instance"].as_str().expect("terminal_gone.instance");
                runner::exec_terminal_gone(&terminals, instance);
            }
        }
    }
}

/// case → 可执行 OverseerBackend（tmp storage 快照 + debug/real LLM + 可变读通道）
fn setup(case: &CaseFile) -> (OverseerBackend<LlmBackend>, SharedTerminals) {
    let tmp = std::env::temp_dir().join(format!("overseer-case-{}", case.meta.case_id));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tmp dir");

    write_jsonl(&tmp, "work-agents.jsonl", &case.work_agents);
    write_jsonl(&tmp, "context.jsonl", &case.context);
    write_jsonl(&tmp, "queue.jsonl", &case.queue);

    let mut config = Config::load_or_default(&tmp);
    // 头部 config 泛化（统一管道 set_by_path，timer 字段自然兼容）
    if let Some(obj) = case.config.as_object() {
        let mut cv = serde_json::to_value(&config).expect("config to value");
        for (k, v) in obj {
            overseer_core::config::reflect::set_by_path(&mut cv, k, v.clone())
                .unwrap_or_else(|e| eprintln!("[case] config {k} 写入失败: {e}"));
        }
        config = serde_json::from_value(cv).expect("config from value");
    }
    // LLM 模式（docs/case-runner.md §LLM 模式）：debug 强制沉默（确定性）；
    // llm.active 非 debug = real——合并生产 providers，env key 现成
    let real_llm = config.llm.active != "debug";
    if real_llm {
        let prod = Config::load_or_default(&overseer_core::paths::config_root());
        for (k, p) in prod.llm.providers {
            config.llm.providers.entry(k).or_insert(p);
        }
        eprintln!("[case] real LLM 模式: active={}（网络调用，非确定性）", config.llm.active);
    } else {
        config.llm.active = "debug".into();
    }

    let harness = overseer_core::Harness::load(&tmp, &tmp, config.effective_token_threshold(), now_ms())
        .expect("load harness");
    let backend = LlmBackend::from_config(&config.llm);
    let mut ov = OverseerBackend::new(harness, config, backend);

    // 读通道：空 map 起步（默认全 None = tab 不复存在），terminal/terminal_gone step 写剧情
    let terminals: SharedTerminals = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let reader = terminals.clone();
    ov.terminal_reader = Some(Arc::new(move |inst: &str| {
        reader.lock().unwrap().get(inst).cloned()
    }));
    (ov, terminals)
}

/// 两段式合法性校验（docs/case-runner.md §health 检查项）
fn health(text: &str, case: &CaseFile) {
    let mut ok = true;
    // 1. 数据区每行（含 marker 行）是合法 JSONL；marker 行形态校验
    for line in text.lines() {
        if line.starts_with(overseer_core::case::SECTION_MARKER) {
            if serde_json::from_str::<serde_json::Value>(line).is_err() {
                eprintln!("FAIL: marker 行非法 JSON: {line}");
                ok = false;
            }
        }
    }
    for (name, content) in &[
        ("work_agents", &case.work_agents),
        ("context", &case.context),
        ("queue", &case.queue),
    ] {
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
    // 3. 概念结构完整：work_agents 每行有必填字段；context 行 type/ts（message 须 role）；queue 行 role/ts
    for (i, line) in case.work_agents.lines().enumerate() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            for f in &["hash", "name", "project", "status"] {
                if v.get(f).is_none() {
                    eprintln!("FAIL: work_agents line {}: missing {}", i + 1, f);
                    ok = false;
                }
            }
        }
    }
    for (i, line) in case.context.lines().enumerate() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            for f in &["type", "ts"] {
                if v.get(f).is_none() {
                    eprintln!("FAIL: context line {}: missing {}", i + 1, f);
                    ok = false;
                }
            }
            if v["type"].as_str() == Some("message") && v.get("role").is_none() {
                eprintln!("FAIL: context line {}: message missing role", i + 1);
                ok = false;
            }
        }
    }
    for (i, line) in case.queue.lines().enumerate() {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
            for f in &["role", "ts"] {
                if v.get(f).is_none() {
                    eprintln!("FAIL: queue line {}: missing {}", i + 1, f);
                    ok = false;
                }
            }
        }
    }
    // 4. replay 烟测：Harness::load 不 panic + observe 可执行
    let (ov, _t) = setup(case);
    let _ = overseer_core::case::observe(&ov);
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

/// 实时 storage → .case 导出（docs/case-runner.md §导出工具）：过滤 → 最小化 → 两段式输出
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
    print!("{}", export::export(&storage_dir, &opts));
}
