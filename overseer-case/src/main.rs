//! overseer-case CLI（docs/case-runner.md）：两段式 .case 回放、step 执行与概念观测。

use overseer_core::case::CaseFile;
use overseer_core::llm::LlmBackend;
use overseer_core::overseer::OverseerBackend;
use overseer_core::server::now_ms;
use overseer_core::{Config, LlmProvider};
use std::sync::Arc;

mod export;
mod runner;

type SharedTerminals = Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>;

fn opt_val(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .and_then(|i| args.get(i + 1))
        .cloned()
}

/// debug 决策源参数（docs/debug-agent.md §HTTP brain / docs/case-runner.md §用例）：
/// `--brain-addr <url>` = HTTP brain（注入 provider，走 OpenAiClient 通用路径）；
/// `--silent` = 沉默 DebugAgent。两者互斥；debug 模式必须显式给一个
fn parse_decision(args: &[String]) -> (Option<String>, bool) {
    let brain = opt_val(args, "--brain-addr");
    let silent = args.iter().any(|a| a == "--silent");
    if brain.is_some() && silent {
        eprintln!("USAGE: --brain-addr 与 --silent 互斥");
        std::process::exit(2);
    }
    (brain, silent)
}

fn apply_decision(config: &mut Config, brain_addr: Option<&str>, silent: bool) {
    if let Some(addr) = brain_addr {
        config.llm.providers.insert(
            "brain".into(),
            LlmProvider {
                base_url: addr.trim_end_matches('/').into(),
                model: "brain".into(),
                api_key_env: None,
                temperature: None,
                context_window: None,
                compression_reserve: None,
            },
        );
        config.llm.active = "brain".into();
        eprintln!("[case] debug 决策源：HTTP brain @ {addr}（OpenAiClient 通用路径）");
    } else if silent {
        config.llm.active = "debug".into();
        eprintln!("[case] debug 决策源：沉默");
    }
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: overseer-case <case.case> [--step-num N] [--health] [--brain-addr <url> | --silent]");
        eprintln!("       overseer-case serve [--brain-addr <url> | --silent]   # 完整 router 宿主（docs/case-runner.md §壳类比）");
        eprintln!("       overseer-case export [--storage DIR] [--instances a,b] [--window 30m]");
        eprintln!("              [--before TS] [--after TS] [--keep-last N] [--keep-agents] [--trim-context] [--dedup] [--dry-run] [--case-id ID]");
        eprintln!("              [--keep-memory --memory name-a,AGENTS] [--keep-cron --cron-ids id-a,id-b]");
        std::process::exit(2);
    }
    if args[1] == "export" {
        run_export(&args[2..]);
        return;
    }
    // serve：完整 router 宿主（浏览器调试 RemoteBridge / 前端进 case 的内嵌 core）
    if args[1] == "serve" {
        let (brain, silent) = parse_decision(&args[2..]);
        let needs_decision =
            Config::load_or_default(&overseer_core::paths::config_root()).llm.active == "debug";
        if needs_decision && brain.is_none() && !silent {
            eprintln!("USAGE: llm active=debug 的 serve 必须显式 --brain-addr <url> 或 --silent（docs/debug-agent.md）");
            std::process::exit(2);
        }
        let parts = overseer_core::host::assemble_host(
            |c| apply_decision(c, brain.as_deref(), silent),
            |b| b,
        );
        let port: u16 = std::env::var("OVERSEER_PORT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(47600);
        overseer_core::host::serve_host(parts, port).await;
        return;
    }
    let case_path = &args[1];
    let health_mode = args.iter().any(|a| a == "--health");
    let max_step = args
        .iter()
        .position(|a| a == "--step-num")
        .and_then(|i| args.get(i + 1))
        .and_then(|n| n.parse::<usize>().ok());
    let (brain_addr, silent) = parse_decision(&args[2..]);

    let text = std::fs::read_to_string(case_path).expect("read case file");
    let case = match overseer_core::case::parse(&text) {
        Ok(c) => c,
        Err(e) => {
            // 缺 llm_mode / 非法值 / no_case_visible 等不合法：health 模式 FAIL 退出 1，否则报错退出
            eprintln!("FAIL: 解析 case（两段式 .case）: {e}");
            std::process::exit(if health_mode { 1 } else { 2 });
        }
    };

    // debug 决策源规则（docs/case-runner.md §用例）：debug 模式必须显式 --brain-addr
    // 或 --silent；real 模式禁止携带（brain 是 debug 专用决策源）。health 静态检查豁免
    if !health_mode {
        match case.meta.llm_mode {
            overseer_core::case::LlmMode::Debug if brain_addr.is_none() && !silent => {
                eprintln!("USAGE: debug 模式必须显式给 --brain-addr <url> 或 --silent（docs/case-runner.md §用例）");
                std::process::exit(2);
            }
            overseer_core::case::LlmMode::Real if brain_addr.is_some() || silent => {
                eprintln!("USAGE: real LLM 模式不接受 --brain-addr/--silent");
                std::process::exit(2);
            }
            _ => {}
        }
    }

    if health_mode {
        health(&text, &case);
        return;
    }

    let (mut ov, terminals) = setup(&case, brain_addr.as_deref(), silent);

    // 合成 ts：steps 未带 ts 时按序递增（回放确定性）
    let mut seq_ts: i64 = 1_000;
    let mut pick_ts = |v: Option<&serde_json::Value>| -> i64 {
        seq_ts += 1;
        v.and_then(|v| v.as_i64()).unwrap_or(seq_ts)
    };
    // 用户变量（store step 设置，跨 step 复用；docs/case-eval-system.md）
    let mut vars: std::collections::HashMap<String, String> = std::collections::HashMap::new();

    for (i, step) in case.steps.iter().enumerate() {
        if let Some(n) = max_step {
            if i >= n {
                break;
            }
        }
        println!("── step {}: {} ──", i + 1, step.name());
        match step {
            overseer_core::case::CaseStep::Observe { observe: items } => {
                runner::exec_observe(&ov, items, &vars);
            }
            overseer_core::case::CaseStep::Store { store } => {
                runner::exec_store(&ov, store, &mut vars);
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
                runner::exec_tool_call(&mut ov, name, args, &format!("case-{i}")).await;
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
fn setup(
    case: &CaseFile,
    brain_addr: Option<&str>,
    silent: bool,
) -> (OverseerBackend<LlmBackend>, SharedTerminals) {
    let tmp = std::env::temp_dir().join(format!("overseer-case-{}", case.meta.case_id));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create tmp dir");

    write_jsonl(&tmp, "work-agents.jsonl", &case.work_agents);
    write_jsonl(&tmp, "context.jsonl", &case.context);
    write_jsonl(&tmp, "queue.jsonl", &case.queue);
    write_jsonl(&tmp, "cron.jsonl", &case.cron);
    // memory 节：Markdown 原文按相对路径落盘；index.md 不进 case——
    // Harness bootstrap 按已选普通记忆在沙盒重建（docs/case-runner.md §数据节）
    for f in &case.memory {
        let path = tmp.join(overseer_core::memory::MEMORY_DIR).join(&f.path);
        std::fs::create_dir_all(path.parent().expect("memory file parent")).expect("memory dir");
        std::fs::write(&path, &f.content).expect("write memory file");
    }

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
    // LLM 模式（docs/case-runner.md §LLM 模式）：meta.llm_mode 两种平级——
    // debug 强制沉默（确定性）；real 合并生产 providers（子集校验）+ env key 现成
    match case.meta.llm_mode {
        overseer_core::case::LlmMode::Debug => {
            if brain_addr.is_none() && !silent {
                // health 烟测路径（不经 LLM 调用）：沉默即可
                config.llm.active = "debug".into();
            }
            apply_decision(&mut config, brain_addr, silent);
        }
        overseer_core::case::LlmMode::Real => {
            let prod = Config::load_or_default(&overseer_core::paths::config_root());
            let declared = config.llm.active.clone();
            // case 不携带 providers（no_case_visible 已在 parse 拒绝）→ 全量取生产
            config.llm.providers = prod.llm.providers;
            config.llm.active = if declared.is_empty() || declared == "debug" {
                prod.llm.active // 未声明 → 生产 active
            } else if config.llm.providers.contains_key(&declared) {
                declared
            } else {
                eprintln!(
                    "[case] FAIL: 声明的 provider '{declared}' 不在生产 providers 里（只能选生产已配置的，不能引入新 provider）"
                );
                std::process::exit(1);
            };
            eprintln!("[case] real LLM 模式: active={}（网络调用，非确定性）", config.llm.active);
        }
    }

    let harness = overseer_core::Harness::load(&tmp, &tmp, config.effective_compression_limit().unwrap_or(usize::MAX), now_ms())
        .expect("load harness");
    let backend = LlmBackend::from_config(&config.llm);
    let mut ov = OverseerBackend::new(harness, config, backend);

    // 读通道：MapAdapter（空 map 起步 = tab 不复存在），terminal/terminal_gone step 写剧情
    let terminals: SharedTerminals = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    ov.terminal = Some(Arc::new(overseer_core::terminal::MapAdapter::new(terminals.clone())));
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
    // JSONL 节逐行可解析；memory 节是 Markdown 原文区（结构已由 parse 校验），不适用
    for (name, content) in &[
        ("work_agents", &case.work_agents),
        ("context", &case.context),
        ("queue", &case.queue),
        ("cron", &case.cron),
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
    // 4. replay 烟测：Harness::load 不 panic + observe 可执行（health 不经 LLM，沉默装配）
    let (ov, _t) = setup(case, None, false);
    let _ = overseer_core::case::observe(&ov);
    // 5. pre-parse 预检（docs/case-eval-system.md §checkhealth，静态不执行）：
    //    表达式 try_parse / 变量引用有效 / store 类型合法 / 类型可落 / observe target 合法
    for f in overseer_core::case::pre_parse_check(case) {
        eprintln!("FAIL: pre-parse: {f}");
        ok = false;
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
    // inclusion bool 与类别过滤器必须成对（docs/case-runner.md §过滤器参数）：单独指定 = USAGE
    let keep_memory = has("--keep-memory");
    let memory = opt_val("--memory")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>());
    let keep_cron = has("--keep-cron");
    let cron_ids = opt_val("--cron-ids")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect::<Vec<_>>());
    if keep_memory != memory.is_some() {
        eprintln!("USAGE: --keep-memory 必须与 --memory <name,...> 同用（双重显式选择，单独指定报错）");
        std::process::exit(2);
    }
    if keep_cron != cron_ids.is_some() {
        eprintln!("USAGE: --keep-cron 必须与 --cron-ids <id,...> 同用（双重显式选择，单独指定报错）");
        std::process::exit(2);
    }
    if memory.as_ref().is_some_and(|ns| ns.iter().any(|n| n == "index")) {
        eprintln!("USAGE: index.md 不可选、不导出——沙盒按已选普通记忆重建");
        std::process::exit(2);
    }
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
        keep_agents: has("--keep-agents"),
        keep_memory,
        memory,
        keep_cron,
        cron_ids,
        dry_run: has("--dry-run"),
    };
    print!("{}", export::export(&storage_dir, &opts));
}
