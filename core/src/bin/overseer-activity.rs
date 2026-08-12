//! overseer-activity（docs/tools.md §overseer-activity）：storage 活动查看器。
//! 读取 Storage 目录下 JSONL 文件（docs/storage.md），交互查看内部消息流。
//! 目录参数默认取 `storage_dir`（`OVERSEER_STORAGE_DIR` 可覆盖），也支持显式传目录。

use overseer_core::context::ContextMessage;
use overseer_core::queue::QueueInput;
use overseer_core::{
    AgentEntry, ContextLine, EffectRecord, TerminalContentRecord, CONTEXT_FILE, EFFECT_FILE,
    QUEUE_FILE, TERMINAL_CONTENT_FILE, WORK_AGENTS_FILE,
};
use std::path::{Path, PathBuf};

/// 统一行模型：一个 JSONL 文件的一行记录折叠为一条可展示的 ActivityRow
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityRow {
    /// 来源文件名（如 context.jsonl）
    pub file: &'static str,
    /// 行类型 / kind（message 的 role、effect 的 kind 等）
    pub kind: String,
    /// 记录时刻（epoch ms；缺失时 0）
    pub ts: i64,
    /// 单行摘要
    pub summary: String,
}

/// 活动视图：6 个 JSONL 数据源读入后的统一行集
#[derive(Debug, Default)]
pub struct Activity {
    pub rows: Vec<ActivityRow>,
}

impl Activity {
    /// 从 storage 目录读取全部数据源（缺失文件视为空）
    pub fn load(dir: &Path) -> std::io::Result<Self> {
        let mut rows = Vec::new();
        rows.extend(read_context(dir)?);
        rows.extend(read_queue(dir)?);
        rows.extend(read_effect(dir)?);
        rows.extend(read_terminal_content(dir)?);
        rows.extend(read_work_agents(dir)?);
        rows.extend(read_cron(dir)?);
        rows.sort_by_key(|r| r.ts);
        Ok(Self { rows })
    }
}

fn read_lines(dir: &Path, file: &str) -> std::io::Result<Vec<String>> {
    let path = dir.join(file);
    if !path.exists() {
        return Ok(vec![]);
    }
    let text = std::fs::read_to_string(path)?;
    Ok(text
        .lines()
        .map(|l| l.trim().to_string())
        .filter(|l| !l.is_empty())
        .collect())
}

fn truncate(s: &str, n: usize) -> String {
    let mut out: String = s.chars().take(n).collect();
    if s.chars().count() > n {
        out.push('…');
    }
    out.replace('\n', " ")
}

fn read_context(dir: &Path) -> std::io::Result<Vec<ActivityRow>> {
    let mut rows = Vec::new();
    for line in read_lines(dir, CONTEXT_FILE)? {
        let Ok(l) = serde_json::from_str::<ContextLine>(&line) else {
            continue;
        };
        let (kind, ts, summary) = match l {
            ContextLine::Message { msg } => context_message_row(&msg),
            ContextLine::Autonomy { content, ts } => {
                ("autonomy".into(), ts, truncate(&content, 60))
            }
            ContextLine::Head { content, ts } => ("head".into(), ts, truncate(&content, 60)),
            ContextLine::Usage {
                prompt_tokens,
                completion_tokens,
                ts,
            } => (
                "usage".into(),
                ts,
                format!("prompt={prompt_tokens} completion={completion_tokens}"),
            ),
            ContextLine::CompactBoundary {
                summary,
                pre_tokens,
                post_tokens,
                ts,
                ..
            } => (
                "compact_boundary".into(),
                ts,
                format!("{}→{}: {}", pre_tokens, post_tokens, truncate(&summary, 40)),
            ),
            ContextLine::Content { instance, ts, .. } => {
                ("content".into(), ts, format!("[legacy] {instance}"))
            }
            ContextLine::Session { session_id, ts } => ("session".into(), ts, session_id),
        };
        rows.push(ActivityRow {
            file: CONTEXT_FILE,
            kind,
            ts,
            summary,
        });
    }
    Ok(rows)
}

fn context_message_row(msg: &ContextMessage) -> (String, i64, String) {
    let role = format!("{:?}", msg.role).to_lowercase();
    let body = if let Some(calls) = &msg.tool_calls {
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        format!("tool_calls: {}", names.join(", "))
    } else {
        truncate(msg.content.as_deref().unwrap_or(""), 60)
    };
    (format!("message/{role}"), msg.ts, body)
}

fn read_queue(dir: &Path) -> std::io::Result<Vec<ActivityRow>> {
    let mut rows = Vec::new();
    for line in read_lines(dir, QUEUE_FILE)? {
        let Ok(q) = serde_json::from_str::<QueueInput>(&line) else {
            continue;
        };
        rows.push(ActivityRow {
            file: QUEUE_FILE,
            kind: format!("{:?}", q.source).to_lowercase(),
            ts: q.ts,
            summary: truncate(&q.content, 60),
        });
    }
    Ok(rows)
}

fn read_effect(dir: &Path) -> std::io::Result<Vec<ActivityRow>> {
    let mut rows = Vec::new();
    for line in read_lines(dir, EFFECT_FILE)? {
        let Ok(e) = serde_json::from_str::<EffectRecord>(&line) else {
            continue;
        };
        rows.push(ActivityRow {
            file: EFFECT_FILE,
            kind: format!("{}/{}", e.origin.as_str(), e.kind),
            ts: e.ts,
            summary: truncate(&e.payload.to_string(), 60),
        });
    }
    Ok(rows)
}

fn read_terminal_content(dir: &Path) -> std::io::Result<Vec<ActivityRow>> {
    let mut rows = Vec::new();
    for line in read_lines(dir, TERMINAL_CONTENT_FILE)? {
        let Ok(t) = serde_json::from_str::<TerminalContentRecord>(&line) else {
            continue;
        };
        rows.push(ActivityRow {
            file: TERMINAL_CONTENT_FILE,
            kind: format!("{:?}", t.source).to_lowercase(),
            ts: t.ts,
            summary: format!("{}: {}", t.instance, truncate(&t.raw, 40)),
        });
    }
    Ok(rows)
}

fn read_work_agents(dir: &Path) -> std::io::Result<Vec<ActivityRow>> {
    let mut rows = Vec::new();
    for line in read_lines(dir, WORK_AGENTS_FILE)? {
        let Ok(a) = serde_json::from_str::<AgentEntry>(&line) else {
            continue;
        };
        rows.push(ActivityRow {
            file: WORK_AGENTS_FILE,
            kind: format!("{:?}", a.status).to_lowercase(),
            ts: a.last_seen,
            summary: format!("{} ({})", a.name, a.project),
        });
    }
    Ok(rows)
}

fn read_cron(dir: &Path) -> std::io::Result<Vec<ActivityRow>> {
    let mut rows = Vec::new();
    for line in read_lines(dir, overseer_core::cron::CRON_FILE)? {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        let op = v["op"].as_str().unwrap_or("?").to_string();
        let ts = v["ts"].as_i64().unwrap_or(0);
        let summary = match op.as_str() {
            "create" => format!(
                "create {}: {}",
                v["id"].as_str().unwrap_or("?"),
                truncate(v["message"].as_str().unwrap_or(""), 40)
            ),
            "fire" => format!("fire {}", v["id"].as_str().unwrap_or("?")),
            "delete" => format!("delete {}", v["id"].as_str().unwrap_or("?")),
            _ => line_brief(&line),
        };
        rows.push(ActivityRow {
            file: overseer_core::cron::CRON_FILE,
            kind: op,
            ts,
            summary,
        });
    }
    Ok(rows)
}

fn line_brief(line: &str) -> String {
    truncate(line, 60)
}

fn storage_dir_arg() -> PathBuf {
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        if a == "--dir" {
            if let Some(d) = args.next() {
                return PathBuf::from(d);
            }
        }
    }
    overseer_core::paths::storage_dir()
}

fn main() {
    let dir = storage_dir_arg();
    match Activity::load(&dir) {
        Ok(a) => {
            for r in &a.rows {
                println!("{} [{}] {} {}", r.ts, r.file, r.kind, r.summary);
            }
            eprintln!("({} rows from {})", a.rows.len(), dir.display());
        }
        Err(e) => {
            eprintln!("overseer-activity: {}: {e}", dir.display());
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "overseer-activity-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn loads_all_six_sources_sorted_by_ts() {
        let dir = tmp_dir("six");
        fs::write(
            dir.join(CONTEXT_FILE),
            concat!(
                r#"{"type":"session","session_id":"s1","ts":1}"#,
                "\n",
                r#"{"type":"message","role":"user","content":"你好","ts":5}"#,
                "\n"
            ),
        )
        .unwrap();
        fs::write(
            dir.join(QUEUE_FILE),
            r#"{"role":"user","content":"hi","source":"user_chat","ts":3}"#.to_string() + "\n",
        )
        .unwrap();
        fs::write(
            dir.join(EFFECT_FILE),
            r#"{"type":"effect","origin":"backend","kind":"render_component","payload":{},"ts":4}"#
                .to_string()
                + "\n",
        )
        .unwrap();
        fs::write(
            dir.join(TERMINAL_CONTENT_FILE),
            r#"{"instance":"ft","raw":"原文","source":"hook","ts":2}"#.to_string() + "\n",
        )
        .unwrap();
        fs::write(
            dir.join(WORK_AGENTS_FILE),
            r#"{"hash":"h1","name":"ft","project":"p","status":"idle","first_seen":0,"last_seen":6}"#
                .to_string()
                + "\n",
        )
        .unwrap();
        fs::write(
            dir.join(overseer_core::cron::CRON_FILE),
            r#"{"op":"create","id":"c1","schedule":{"every_ms":1000},"message":"提醒","ts":7}"#
                .to_string()
                + "\n",
        )
        .unwrap();

        let a = Activity::load(&dir).unwrap();
        assert_eq!(a.rows.len(), 7);
        // 按 ts 排序
        let tss: Vec<i64> = a.rows.iter().map(|r| r.ts).collect();
        assert_eq!(tss, vec![1, 2, 3, 4, 5, 6, 7]);
        // 六个文件都出现
        let files: std::collections::HashSet<&str> = a.rows.iter().map(|r| r.file).collect();
        assert!(files.contains(CONTEXT_FILE));
        assert!(files.contains(QUEUE_FILE));
        assert!(files.contains(EFFECT_FILE));
        assert!(files.contains(TERMINAL_CONTENT_FILE));
        assert!(files.contains(WORK_AGENTS_FILE));
        assert!(files.contains(overseer_core::cron::CRON_FILE));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_dir_yields_no_rows() {
        let dir = tmp_dir("empty");
        let a = Activity::load(&dir).unwrap();
        assert!(a.rows.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}
