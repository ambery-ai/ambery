//! 实时 storage → case 导出管线（docs/case-runner.md §导出工具）
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

/// context.jsonl 行过滤：时间按 ts；--trim-context 删除与保留 instances 无关的 content 行
///（message/head/autonomy/session/compact_boundary 是对话本体与装配留痕，不按 instance 过滤）
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
                if let (Some(ins), Some(inst)) = (&opts.instances, v["instance"].as_str()) {
                    if v["type"].as_str() == Some("content") && !ins.iter().any(|i| i == inst) {
                        return false;
                    }
                }
            }
            true
        })
        .collect();
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

/// queue.jsonl 行过滤：常全量保留（仅时间窗，docs/case-runner.md §过滤器参数）
fn filter_queue(lines: Vec<String>, opts: &ExportOpts) -> Vec<String> {
    if opts.before.is_none() && opts.after.is_none() && opts.window_ms.is_none() {
        return lines;
    }
    let max_ts = lines
        .iter()
        .filter_map(|l| json(l)?["ts"].as_i64())
        .max()
        .unwrap_or(0);
    lines
        .into_iter()
        .filter(|l| {
            json(l)
                .map(|v| in_time(v["ts"].as_i64().unwrap_or(0), opts, max_ts))
                .unwrap_or(false)
        })
        .collect()
}

/// 实时 storage → case JSON 字符串（mock_terminals 留空待手填，docs/case-runner.md §管线）
pub fn export(storage_dir: &std::path::Path, opts: &ExportOpts) -> String {
    let work_agents = filter_work_agents(lines_of(&storage_dir.join("work-agents.jsonl")), opts);
    let context = filter_context(lines_of(&storage_dir.join("context.jsonl")), opts);
    let queue = filter_queue(lines_of(&storage_dir.join("queue.jsonl")), opts);
    let case = serde_json::json!({
        "meta": {
            "case_id": opts.case_id,
            "created": chrono_now(),
            "notes": opts.notes,
        },
        "data": {
            "work_agents": work_agents.join("\n"),
            "context": context.join("\n"),
            "queue": queue.join("\n"),
            "mock_terminals": {},
            "config": {},
        },
        "steps": [
            { "load": {} },
            { "observe": ["agents", "panorama"] },
        ],
    });
    serde_json::to_string_pretty(&case).unwrap()
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
}
