//! 实时 storage → case 导出管线
//! 过滤阶段只控制哪些行进入 case（不改行内容）；最小化阶段对行内容瘦身（不改结构）。

use serde_json::Value;

pub struct ExportOpts {
    pub case_id: String,
    pub notes: String,
    pub instances: Option<Vec<String>>,
    pub before: Option<i64>,
    pub after: Option<i64>,
    pub window_ms: Option<i64>,
    pub keep_last: Option<usize>,
    pub trim_context: bool,
    pub dedup: bool,
    /// 默认不导出 work_agents 节（含项目名，隐私）；显式才保留完整节
    pub keep_agents: bool,
    /// Memory 导出资格（必须与 memory 同用，CLI 层校验成对）
    pub keep_memory: bool,
    /// Memory 文件过滤器：普通记忆按 name 选；保留值 `AGENTS` 带入导航原文；index.md 不可选
    pub memory: Option<Vec<String>>,
    /// Cron 导出资格（必须与 cron_ids 同用，CLI 层校验成对）
    pub keep_cron: bool,
    /// Cron 计划过滤器：选中 id 的 create / fire / delete 行完整保留，不受时间窗逐行裁断
    pub cron_ids: Option<Vec<String>>,
    /// 预览过滤后各文件行数，不生成 case 文件
    pub dry_run: bool,
}

/// 解析时长字符串（30s / 30m / 1h / 1d）→ ms
pub fn parse_duration(s: &str) -> Option<i64> {
    let (num, unit) = s.split_at(s.len().saturating_sub(1));
    let n: i64 = num.parse().ok()?;
    match unit {
        "s" => Some(n * 1_000),
        "m" => Some(n * 60_000),
        "h" => Some(n * 3_600_000),
        "d" => Some(n * 86_400_000),
        _ => None,
    }
}

fn lines_of(path: &std::path::Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(String::from)
        .collect()
}

fn json(line: &str) -> Option<Value> {
    serde_json::from_str(line).ok()
}

/// 时间窗过滤：after ≤ ts ≤ before；window 相对文件内最大 ts 回溯
fn in_time(ts: i64, opts: &ExportOpts, max_ts: i64) -> bool {
    if let Some(a) = opts.after {
        if ts < a {
            return false;
        }
    }
    if let Some(b) = opts.before {
        if ts > b {
            return false;
        }
    }
    if let Some(w) = opts.window_ms {
        if ts < max_ts - w {
            return false;
        }
    }
    true
}

/// work-agents.jsonl 行过滤：--instances 按 name；时间按 last_seen；--keep-last 每 hash 最后 N 行
fn filter_work_agents(lines: Vec<String>, opts: &ExportOpts) -> Vec<String> {
    let max_ts = lines
        .iter()
        .filter_map(|l| json(l)?["last_seen"].as_i64())
        .max()
        .unwrap_or(0);
    let mut kept: Vec<String> = lines
        .into_iter()
        .filter(|l| {
            let Some(v) = json(l) else { return false };
            if let Some(ins) = &opts.instances {
                let name = v["name"].as_str().unwrap_or("");
                if !ins.iter().any(|i| i == name) {
                    return false;
                }
            }
            in_time(v["last_seen"].as_i64().unwrap_or(0), opts, max_ts)
        })
        .collect();
    if let Some(n) = opts.keep_last {
        // 每 hash 只保留最后 N 行（保序）
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        let total_by_hash: std::collections::HashMap<String, usize> = kept
            .iter()
            .filter_map(|l| json(l))
            .fold(std::collections::HashMap::new(), |mut m, v| {
                *m.entry(v["hash"].as_str().unwrap_or("").to_string())
                    .or_insert(0) += 1;
                m
            });
        kept = kept
            .into_iter()
            .filter(|l| {
                let h = json(l)
                    .map(|v| v["hash"].as_str().unwrap_or("").to_string())
                    .unwrap_or_default();
                let seen = counts.entry(h.clone()).or_insert(0);
                *seen += 1;
                *seen > total_by_hash.get(&h).copied().unwrap_or(0).saturating_sub(n)
            })
            .collect();
    }
    kept
}

/// context.jsonl 行过滤：时间按 ts；--trim-context（需配合 --instances）：
/// - content 行：只留保留实例（孤行清理）
/// - message 行：content 提及保留实例名才留
/// - autonomy/session/head/compact_boundary：装配/审计留痕，case replay 不入内存 → 丢弃
///   （默认保留；多 run 切片类 case 不要用 --trim-context）
fn filter_context(lines: Vec<String>, opts: &ExportOpts) -> Vec<String> {
    let max_ts = lines
        .iter()
        .filter_map(|l| json(l)?["ts"].as_i64())
        .max()
        .unwrap_or(0);
    let mut kept: Vec<String> = lines
        .into_iter()
        .filter(|l| {
            let Some(v) = json(l) else { return false };
            if !in_time(v["ts"].as_i64().unwrap_or(0), opts, max_ts) {
                return false;
            }
            if opts.trim_context {
                if let Some(ins) = &opts.instances {
                    match v["type"].as_str() {
                        Some("content") => {
                            let inst = v["instance"].as_str().unwrap_or("");
                            if !ins.iter().any(|i| i == inst) {
                                return false;
                            }
                        }
                        Some("message") => {
                            let body = v["content"].as_str().unwrap_or("");
                            if !ins.iter().any(|i| body.contains(i.as_str())) {
                                return false;
                            }
                        }
                        // 装配/审计留痕：case replay 不入内存（空 Context 起步+归零重同步），
                        // 裁剪时一并丢弃（默认保留，多 run 切片类 case 不用 --trim-context）
                        Some("autonomy" | "session" | "head" | "compact_boundary") => return false,
                        _ => {}
                    }
                }
            }
            true
        })
        .collect();
    // --keep-last：content 行每 instance 只留最后 N 行（保序）
    if let (Some(n), Some(_)) = (opts.keep_last, &opts.instances) {
        let total: std::collections::HashMap<String, usize> = kept
            .iter()
            .filter_map(|l| json(l))
            .filter(|v| v["type"].as_str() == Some("content"))
            .fold(std::collections::HashMap::new(), |mut m, v| {
                *m.entry(v["instance"].as_str().unwrap_or("").to_string())
                    .or_insert(0) += 1;
                m
            });
        let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        kept = kept
            .into_iter()
            .filter(|l| {
                let Some(v) = json(l) else { return true };
                if v["type"].as_str() != Some("content") {
                    return true;
                }
                let inst = v["instance"].as_str().unwrap_or("").to_string();
                let c = seen.entry(inst.clone()).or_insert(0);
                *c += 1;
                *c > total.get(&inst).copied().unwrap_or(0).saturating_sub(n)
            })
            .collect();
    }
    if opts.dedup {
        // 相邻 content 完全相同的行只保留最早一条
        let mut out: Vec<String> = vec![];
        for l in kept {
            let dup = out.last().and_then(|prev| json(prev)).zip(json(&l)).is_some_and(
                |(p, c)| {
                    p["type"] == c["type"]
                        && c["type"].as_str() == Some("content")
                        && p["instance"] == c["instance"]
                        && p["content"] == c["content"]
                },
            );
            if !dup {
                out.push(l);
            }
        }
        kept = out;
    }
    kept
}

/// queue.jsonl 行过滤：时间窗；--trim-context（配合 --instances）时
/// 输入条目 content 提及保留实例名才留（hook hint 含实例名，同 message 规则）
fn filter_queue(lines: Vec<String>, opts: &ExportOpts) -> Vec<String> {
    let max_ts = lines
        .iter()
        .filter_map(|l| json(l)?["ts"].as_i64())
        .max()
        .unwrap_or(0);
    lines
        .into_iter()
        .filter(|l| {
            let Some(v) = json(l) else { return false };
            if opts.before.is_some() || opts.after.is_some() || opts.window_ms.is_some() {
                if !in_time(v["ts"].as_i64().unwrap_or(0), opts, max_ts) {
                    return false;
                }
            }
            if opts.trim_context {
                if let Some(ins) = &opts.instances {
                    let body = v["content"].as_str().unwrap_or("");
                    if !ins.iter().any(|i| body.contains(i.as_str())) {
                        return false;
                    }
                }
            }
            true
        })
        .collect()
}

/// Memory 文件选择（--keep-memory + --memory）：`AGENTS` 带导航原文；普通记忆按 name
/// 选 notes/<name>.md；index.md 不可选（CLI 层已拦），沙盒按已选普通记忆重建。
/// 选中名不存在 → 警告并跳过（不阻断其余导出）。
fn select_memory(storage_dir: &std::path::Path, names: &[String]) -> Vec<(String, String)> {
    let mem_root = storage_dir.join(ambery_core::memory::MEMORY_DIR);
    let mut out: Vec<(String, String)> = vec![];
    // AGENTS.md 导航优先（先于 notes 出现，扫读时先见地图）
    if names.iter().any(|n| n == "AGENTS") {
        let p = mem_root.join("AGENTS.md");
        match std::fs::read_to_string(&p) {
            Ok(c) => out.push(("AGENTS.md".to_string(), c)),
            Err(_) => eprintln!("[export] 警告：--memory 含 AGENTS 但 memory/AGENTS.md 不存在，跳过"),
        }
    }
    for name in names.iter().filter(|n| n.as_str() != "AGENTS") {
        if !ambery_core::memory::valid_name(name) || ambery_core::memory::RESERVED.contains(&name.as_str()) {
            eprintln!("[export] 警告：--memory 名 '{name}' 不合法（文件名 grammar/保留名），跳过");
            continue;
        }
        let rel = format!("notes/{name}.md");
        match std::fs::read_to_string(mem_root.join(&rel)) {
            Ok(c) => out.push((rel, c)),
            Err(_) => eprintln!("[export] 警告：普通记忆 '{name}' 不存在（memory/{rel}），跳过"),
        }
    }
    out
}

/// cron.jsonl 行过滤：只按 id 集选择（create / fire / delete 完整生命周期），
/// 不套时间窗/--keep-last（因果链不按时间裁断）
fn filter_cron(lines: Vec<String>, ids: &[String]) -> Vec<String> {
    let kept: Vec<String> = lines
        .into_iter()
        .filter(|l| {
            let Some(v) = json(l) else { return false };
            let id = v["id"].as_str().unwrap_or("");
            ids.iter().any(|i| i == id)
        })
        .collect();
    for id in ids {
        if !kept.iter().any(|l| json(l).is_some_and(|v| v["id"].as_str() == Some(id))) {
            eprintln!("[export] 警告：--cron-ids 的 id '{id}' 在 cron.jsonl 中无任何行，跳过");
        }
    }
    kept
}

/// 实时 storage → 两段式 .case 文本：
/// JSON 头（meta/config/steps）+ __section 分节 JSONL 原文；mock_terminals 不再有节
///（读通道剧情由 steps 的 terminal/terminal_gone 表达，导出默认全 None）
pub fn export(storage_dir: &std::path::Path, opts: &ExportOpts) -> String {
    // work_agents 默认过滤（隐私：含项目结构）；--keep-agents 显式保留（空节保留 marker，
    // 「刻意为空」与「忘了填」可区分，§Case 文件格式）
    let work_agents = if opts.keep_agents {
        filter_work_agents(lines_of(&storage_dir.join("work-agents.jsonl")), opts)
    } else {
        vec![]
    };
    let context = filter_context(lines_of(&storage_dir.join("context.jsonl")), opts);
    let queue = filter_queue(lines_of(&storage_dir.join("queue.jsonl")), opts);
    // cron / memory 默认过滤（隐私）：inclusion bool + 类别过滤双重显式才进 case
    let cron = if opts.keep_cron {
        filter_cron(
            lines_of(&storage_dir.join("cron.jsonl")),
            opts.cron_ids.as_deref().unwrap_or(&[]),
        )
    } else {
        vec![]
    };
    let memory = if opts.keep_memory {
        select_memory(storage_dir, opts.memory.as_deref().unwrap_or(&[]))
    } else {
        vec![]
    };
    if opts.dry_run {
        return format!(
            "work_agents: {} 行, context: {} 行, queue: {} 行, cron: {} 行, memory: {} 文件（dry-run 预览，未生成 case）\n",
            work_agents.len(),
            context.len(),
            queue.len(),
            cron.len(),
            memory.len()
        );
    }
    let head = serde_json::json!({
        "meta": {
            "case_id": opts.case_id,
            "created": chrono_now(),
            "llm_mode": "debug",
            "notes": opts.notes,
        },
        "config": {},
        "steps": [
            { "load": {} },
            { "observe": [{ "target": "agents" }, { "target": "panorama" }] },
        ],
    });
    let mut out = serde_json::to_string_pretty(&head).unwrap();
    out.push('\n');
    let mut section = |name: &str, rows: &[String]| {
        out.push_str(&format!("{{\"__section\":\"=== {name} ===\"}}\n"));
        for r in rows {
            out.push_str(r);
            out.push('\n');
        }
    };
    section("work_agents", &work_agents);
    section("context", &context);
    section("queue", &queue);
    section("cron", &cron);
    // memory 节：Markdown 原文区——{"__file":...} 标记分文件，原文零转义（§一致性剖析：
    // JSONL 与 Markdown 分属各自原始区和边界规则）；空节保留 marker
    out.push_str("{\"__section\":\"=== memory ===\"}\n");
    for (path, content) in &memory {
        for (i, l) in content.lines().enumerate() {
            if l.starts_with("{\"__") {
                eprintln!(
                    "[export] 警告：{path} 第 {} 行以 `{{\"__` 行首开头，与 case 标记语法冲突（case health 会拒）",
                    i + 1
                );
            }
        }
        out.push_str(&format!("{{\"__file\":\"{path}\"}}\n"));
        out.push_str(content.trim_end());
        out.push('\n');
    }
    out
}

/// ISO 8601 当前时刻（不引 chrono：手算 UTC）
fn chrono_now() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) as i64;
    let (mut y, mut rest) = (1970i64, secs);
    let days = rest / 86_400;
    rest %= 86_400;
    let (h, mi, s) = (rest / 3600, rest % 3600 / 60, rest % 60);
    // 简化历年推算（1970-2399 够用）
    let mut d = days;
    loop {
        let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
        let year_days = if leap { 366 } else { 365 };
        if d < year_days {
            break;
        }
        d -= year_days;
        y += 1;
    }
    let leap = y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let months = [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut m = 0usize;
    for (i, &md) in months.iter().enumerate() {
        if d < md {
            m = i + 1;
            break;
        }
        d -= md;
    }
    format!("{y:04}-{m:02}-{:02}T{h:02}:{mi:02}:{s:02}Z", d + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_units() {
        assert_eq!(parse_duration("30s"), Some(30_000));
        assert_eq!(parse_duration("30m"), Some(1_800_000));
        assert_eq!(parse_duration("1h"), Some(3_600_000));
        assert_eq!(parse_duration("2d"), Some(172_800_000));
        assert_eq!(parse_duration("xyz"), None);
    }

    #[test]
    fn dedup_adjacent_content() {
        let opts = ExportOpts {
            case_id: "t".into(),
            notes: String::new(),
            instances: None,
            before: None,
            after: None,
            window_ms: None,
            keep_last: None,
            trim_context: false,
            dedup: true,
            keep_agents: true,
            keep_memory: false,
            memory: None,
            keep_cron: false,
            cron_ids: None,
            dry_run: false,
        };
        let lines = vec![
            r#"{"type":"content","instance":"a","content":"同","ts":1}"#.to_string(),
            r#"{"type":"content","instance":"a","content":"同","ts":2}"#.to_string(),
            r#"{"type":"content","instance":"a","content":"异","ts":3}"#.to_string(),
        ];
        let out = filter_context(lines, &opts);
        assert_eq!(out.len(), 2); // 相邻重复只留最早
        assert!(out[0].contains("\"ts\":1"));
        assert!(out[1].contains("\"ts\":3"));
    }

    #[test]
    fn keep_agents_gated() {
        // 默认不导出 work_agents（隐私）；--keep-agents 显式保留完整节
        let dir = std::env::temp_dir().join(format!("ambery-export-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("work-agents.jsonl"),
            "{\"hash\":\"h1\",\"name\":\"ft\",\"project\":\"secret-proj\",\"status\":\"processing\",\"last_seen\":1}\n",
        )
        .unwrap();
        let mk = || ExportOpts {
            case_id: "t".into(),
            notes: String::new(),
            instances: None,
            before: None,
            after: None,
            window_ms: None,
            keep_last: None,
            trim_context: false,
            dedup: false,
            keep_agents: false,
            keep_memory: false,
            memory: None,
            keep_cron: false,
            cron_ids: None,
            dry_run: false,
        };
        let out = export(&dir, &mk());
        assert!(!out.contains("secret-proj"), "默认不含 work_agents 行");
        assert!(out.contains("=== work_agents ==="), "空节保留 marker（刻意为空可区分）");
        let kept = export(&dir, &ExportOpts { keep_agents: true, ..mk() });
        assert!(kept.contains("secret-proj"), "--keep-agents 显式保留");
        // --dry-run：预览行数，不生成 case
        let preview = export(&dir, &ExportOpts { dry_run: true, keep_agents: true, ..mk() });
        assert!(preview.contains("work_agents: 1 行"), "{preview}");
        assert!(!preview.contains("__section"), "dry-run 不生成 case 文件");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 造带 memory/cron 的 storage 目录
    fn storage_with_memory_cron(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("ambery-export-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let notes = dir.join("memory/notes");
        std::fs::create_dir_all(&notes).unwrap();
        std::fs::write(dir.join("memory/AGENTS.md"), "# Memory Workspace\n导航").unwrap();
        std::fs::write(
            notes.join("work-preferences.md"),
            "---\ndescription: 用户的工作偏好\n---\n\n- 不擅自提交\n",
        )
        .unwrap();
        std::fs::write(notes.join("other-note.md"), "---\ndescription: 另一条\n---\n\n正文\n").unwrap();
        std::fs::write(dir.join("memory/index.md"), "# Memory Index\n（派生文件）").unwrap();
        std::fs::write(
            dir.join("cron.jsonl"),
            concat!(
                "{\"op\":\"create\",\"id\":\"a1\",\"schedule\":{\"every_ms\":60000},\"message\":\"日报\",\"next_due\":61000,\"ts\":1000}\n",
                "{\"op\":\"fire\",\"id\":\"a1\",\"next_due\":121000,\"ts\":61000}\n",
                "{\"op\":\"create\",\"id\":\"b2\",\"schedule\":{\"at\":99999},\"message\":\"一次性\",\"next_due\":99999,\"ts\":2000}\n",
                "{\"op\":\"delete\",\"id\":\"b2\",\"ts\":3000}\n",
            ),
        )
        .unwrap();
        dir
    }

    #[test]
    fn memory_gated_and_selected() {
        let dir = storage_with_memory_cron("mem");
        let mk = || ExportOpts {
            case_id: "t".into(),
            notes: String::new(),
            instances: None,
            before: None,
            after: None,
            window_ms: None,
            keep_last: None,
            trim_context: false,
            dedup: false,
            keep_agents: false,
            keep_memory: false,
            memory: None,
            keep_cron: false,
            cron_ids: None,
            dry_run: false,
        };
        // 默认：memory 节空（marker 保留），任何 memory 文件内容不进 case
        let out = export(&dir, &mk());
        assert!(out.contains("=== memory ==="));
        assert!(!out.contains("不擅自提交"), "默认不导出 memory 原文");
        assert!(!out.contains("AGENTS.md\""), "默认不带 AGENTS 标记");
        // 显式选择：普通记忆按 name；AGENTS 带原文；index.md 永不导出
        let out = export(
            &dir,
            &ExportOpts {
                keep_memory: true,
                memory: Some(vec!["work-preferences".into(), "AGENTS".into()]),
                ..mk()
            },
        );
        assert!(out.contains("{\"__file\":\"AGENTS.md\"}\n# Memory Workspace\n导航"));
        assert!(out.contains("{\"__file\":\"notes/work-preferences.md\"}\n---\ndescription: 用户的工作偏好\n---\n\n- 不擅自提交"));
        assert!(!out.contains("other-note"), "未选中的 note 不进 case");
        assert!(!out.contains("Memory Index"), "派生 index.md 不导出");
        // 选中的名不存在：警告跳过，其余照常
        let out = export(
            &dir,
            &ExportOpts {
                keep_memory: true,
                memory: Some(vec!["no-such".into(), "other-note".into()]),
                ..mk()
            },
        );
        assert!(out.contains("notes/other-note.md"));
        assert!(!out.contains("no-such"), "不存在的名跳过");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_gated_and_full_lifecycle_kept() {
        let dir = storage_with_memory_cron("cron");
        let mk = || ExportOpts {
            case_id: "t".into(),
            notes: String::new(),
            instances: None,
            before: None,
            after: None,
            window_ms: None,
            keep_last: None,
            trim_context: false,
            dedup: false,
            keep_agents: false,
            keep_memory: false,
            memory: None,
            keep_cron: false,
            cron_ids: None,
            dry_run: false,
        };
        // 默认：cron 节空
        let out = export(&dir, &mk());
        assert!(out.contains("=== cron ==="));
        assert!(!out.contains("\"id\":\"a1\""), "默认不导出 cron 行");
        // 选中 id：create/fire/delete 完整生命周期保留；时间窗不适用（--after 2500 也裁不断 a1）
        let out = export(
            &dir,
            &ExportOpts {
                keep_cron: true,
                cron_ids: Some(vec!["a1".into()]),
                after: Some(2500), // 对 context/queue 生效，对 cron 无效
                ..mk()
            },
        );
        let cron_rows: Vec<&str> = out
            .lines()
            .filter(|l| l.contains("\"id\":\"a1\""))
            .collect();
        assert_eq!(cron_rows.len(), 2, "create+fire 全保留：{cron_rows:?}");
        assert!(!out.contains("\"id\":\"b2\""), "未选中 id 不进 case");
        // 选中 b2：delete 行也在（完整生命周期含消亡）
        let out = export(
            &dir,
            &ExportOpts {
                keep_cron: true,
                cron_ids: Some(vec!["b2".into()]),
                ..mk()
            },
        );
        assert!(out.contains("\"op\":\"delete\",\"id\":\"b2\""));
        // dry-run 含 cron/memory 计数
        let preview = export(
            &dir,
            &ExportOpts {
                dry_run: true,
                keep_cron: true,
                cron_ids: Some(vec!["a1".into()]),
                keep_memory: true,
                memory: Some(vec!["work-preferences".into()]),
                ..mk()
            },
        );
        assert!(preview.contains("cron: 2 行"), "{preview}");
        assert!(preview.contains("memory: 1 文件"), "{preview}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
