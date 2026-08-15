//! ambery-activity：storage 活动查看器。
//! 读取 Storage 目录下 JSONL 文件，交互查看内部消息流。
//! 目录参数默认取 `storage_dir`（`AMBERY_STORAGE_DIR` 可覆盖），也支持显式传目录。

use ambery_core::context::ContextMessage;
use ambery_core::queue::QueueInput;
use ambery_core::{
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
            // 用 serde 序列化名（snake_case），Debug 名会丢下划线（MockHook → mockhook）
            kind: serde_name(&q.source),
            ts: q.ts,
            summary: truncate(&q.content, 60),
        });
    }
    Ok(rows)
}

/// 枚举的 serde 序列化名（snake_case），非 Debug 名（multi-word Debug 无下划线分隔）
fn serde_name<T: serde::Serialize>(v: &T) -> String {
    serde_json::to_string(v)
        .map(|s| s.trim_matches('"').to_string())
        .unwrap_or_default()
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
            // serde 序列化名（RecordSource::FetchTerminal 是 snake_case fetch_terminal）
            kind: serde_name(&t.source),
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
    for line in read_lines(dir, ambery_core::cron::CRON_FILE)? {
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
            file: ambery_core::cron::CRON_FILE,
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

/// CLI 选项（--dir 覆盖目录，--follow tail 新增）
struct Options {
    dir: PathBuf,
    follow: bool,
    /// --dump：非交互，纯文本打印全部行（脚本/管道用；默认是 TUI）
    dump: bool,
}

fn parse_args() -> Options {
    let mut dir = ambery_core::paths::storage_dir();
    let mut follow = false;
    let mut dump = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--dir" => {
                if let Some(d) = args.next() {
                    dir = PathBuf::from(d);
                }
            }
            "--follow" | "-f" => follow = true,
            "--dump" | "-d" => dump = true,
            _ => {}
        }
    }
    Options { dir, follow, dump }
}

fn main() {
    let opt = parse_args();
    match Activity::load(&opt.dir) {
        Ok(a) => {
            if opt.dump {
                // --dump：非交互纯文本（脚本/管道/验证用）
                for r in &a.rows {
                    println!("{} [{}] {} {}", r.ts, r.file, r.kind, r.summary);
                }
                eprintln!("({} rows from {})", a.rows.len(), opt.dir.display());
            } else {
                // TUI 为默认形态（TUI 交互界面）
                if let Err(e) = run_tui(opt.dir, a, opt.follow) {
                    eprintln!("ambery-activity: {e}");
                    std::process::exit(1);
                }
            }
        }
        Err(e) => {
            eprintln!("ambery-activity: {}: {e}", opt.dir.display());
            std::process::exit(1);
        }
    }
}

// ---- TUI 交互层（ratatui）----

use crossterm::event::{self, Event, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Layout};
use ratatui::widgets::{Block, Borders, List, ListItem, ListState, Paragraph};
use ratatui::Terminal;

/// 数据源文件清单（固定顺序，Tab 循环切换）
const FILES: &[&str] = &[
    "all",
    CONTEXT_FILE,
    QUEUE_FILE,
    EFFECT_FILE,
    TERMINAL_CONTENT_FILE,
    WORK_AGENTS_FILE,
    ambery_core::cron::CRON_FILE,
];

struct Tui {
    activity: Activity,
    /// FILES 中当前下标（0 = all）
    file_idx: usize,
    /// 光标所在行（在 filtered 视图内）
    cursor: usize,
    /// 顶部滚动偏移
    offset: usize,
    /// 筛选子串（匹配 kind / summary）
    filter: String,
    /// 正在输入筛选
    filtering: bool,
    follow: bool,
    /// follow 模式下已消费的行数（增量重读起点）
    seen: usize,
    quit: bool,
}

impl Tui {
    fn new(activity: Activity, follow: bool) -> Self {
        let mut t = Self {
            activity,
            file_idx: 0,
            cursor: 0,
            offset: 0,
            filter: String::new(),
            filtering: false,
            follow,
            seen: 0,
            quit: false,
        };
        t.seen = t.activity.rows.len();
        t
    }

    /// 当前 filtered 视图（文件 + 子串筛选）
    fn view(&self) -> Vec<&ActivityRow> {
        let file = FILES[self.file_idx];
        self.activity
            .rows
            .iter()
            .filter(|r| file == "all" || r.file == file)
            .filter(|r| {
                self.filter.is_empty()
                    || r.kind.contains(&self.filter)
                    || r.summary.contains(&self.filter)
            })
            .collect()
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.view().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as isize + delta;
        self.cursor = next.clamp(0, len as isize - 1) as usize;
    }

    fn next_file(&mut self) {
        self.file_idx = (self.file_idx + 1) % FILES.len();
        self.cursor = 0;
        self.offset = 0;
    }

    /// follow：增量重读新写入的行
    fn reload(&mut self, dir: &Path) {
        if let Ok(a) = Activity::load(dir) {
            if a.rows.len() > self.seen {
                self.seen = a.rows.len();
                self.activity = a;
                if self.follow {
                    // 跟随模式保持光标在最新行
                    let len = self.view().len();
                    if len > 0 {
                        self.cursor = len - 1;
                    }
                }
            }
        }
    }
}

fn run_tui(dir: PathBuf, activity: Activity, follow: bool) -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut tui = Tui::new(activity, follow);

    loop {
        if tui.follow {
            tui.reload(&dir);
        }
        // 滚动窗口：光标保持在可视区（借用前先把 view 物化，避免闭包内二次借用）
        let view: Vec<ActivityRow> = tui.view().into_iter().cloned().collect();
        let area = terminal.get_frame().area();
        let height = area.height.saturating_sub(4) as usize;
        if tui.cursor < tui.offset {
            tui.offset = tui.cursor;
        } else if height > 0 && tui.cursor >= tui.offset + height {
            tui.offset = tui.cursor + 1 - height;
        }
        let offset = tui.offset;
        let cursor = tui.cursor;
        let file_label = FILES[tui.file_idx];
        let filter = tui.filter.clone();
        let filtering = tui.filtering;
        let follow = tui.follow;

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(ratatui::layout::Direction::Vertical)
                .constraints([
                    Constraint::Length(2),
                    Constraint::Min(1),
                    Constraint::Length(2),
                ])
                .split(f.area());

            let title = format!(
                "ambery-activity  file={}  rows={}  filter={}{}",
                file_label,
                view.len(),
                filter,
                if follow { "  [follow]" } else { "" }
            );
            f.render_widget(
                Paragraph::new(title).block(Block::default().borders(Borders::BOTTOM)),
                chunks[0],
            );

            let items: Vec<ListItem> = view
                .iter()
                .skip(offset)
                .take(height)
                .map(|r| ListItem::new(format!("{} [{}] {} {}", r.ts, r.file, r.kind, r.summary)))
                .collect();
            let mut state = ListState::default();
            if !view.is_empty() {
                state.select(Some(cursor.saturating_sub(offset)));
            }
            let list = List::new(items)
                .block(Block::default().borders(Borders::NONE))
                .highlight_symbol("▶ ");
            f.render_stateful_widget(list, chunks[1], &mut state);

            let help = if filtering {
                format!("/{}", filter)
            } else {
                "↑/↓ 滚动  Tab 切文件  / 筛选  f 跟随  q 退出".to_string()
            };
            f.render_widget(
                Paragraph::new(help).block(Block::default().borders(Borders::TOP)),
                chunks[2],
            );
        })?;

        if event::poll(std::time::Duration::from_millis(200))? {
            if let Event::Key(k) = event::read()? {
                if tui.filtering {
                    match k.code {
                        KeyCode::Enter | KeyCode::Esc => tui.filtering = false,
                        KeyCode::Backspace => {
                            tui.filter.pop();
                            tui.cursor = 0;
                            tui.offset = 0;
                        }
                        KeyCode::Char(c) => {
                            tui.filter.push(c);
                            tui.cursor = 0;
                            tui.offset = 0;
                        }
                        _ => {}
                    }
                } else {
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => tui.quit = true,
                        KeyCode::Up => tui.move_cursor(-1),
                        KeyCode::Down => tui.move_cursor(1),
                        KeyCode::Tab => tui.next_file(),
                        KeyCode::Char('/') => tui.filtering = true,
                        KeyCode::Char('f') => tui.follow = !tui.follow,
                        _ => {}
                    }
                }
            }
        }
        if tui.quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ambery-activity-test-{}-{}",
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
            dir.join(ambery_core::cron::CRON_FILE),
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
        assert!(files.contains(ambery_core::cron::CRON_FILE));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn empty_dir_yields_no_rows() {
        let dir = tmp_dir("empty");
        let a = Activity::load(&dir).unwrap();
        assert!(a.rows.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    fn sample_rows() -> Vec<ActivityRow> {
        vec![
            ActivityRow {
                file: CONTEXT_FILE,
                kind: "message/user".into(),
                ts: 1,
                summary: "你好".into(),
            },
            ActivityRow {
                file: QUEUE_FILE,
                kind: "user_chat".into(),
                ts: 2,
                summary: "hi".into(),
            },
            ActivityRow {
                file: EFFECT_FILE,
                kind: "backend/render_component".into(),
                ts: 3,
                summary: "{}".into(),
            },
        ]
    }

    #[test]
    fn tui_view_filters_by_file_and_substring() {
        let t = Tui::new(
            Activity {
                rows: sample_rows(),
            },
            false,
        );
        // all：全量
        assert_eq!(t.view().len(), 3);

        // 文件切换：只看 queue.jsonl
        let mut t = t;
        t.file_idx = FILES.iter().position(|f| *f == QUEUE_FILE).unwrap();
        let v = t.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].file, QUEUE_FILE);

        // 子串筛选（kind）
        let mut t = Tui::new(
            Activity {
                rows: sample_rows(),
            },
            false,
        );
        t.filter = "render".into();
        let v = t.view();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].kind, "backend/render_component");
    }

    #[test]
    fn tui_cursor_moves_within_view_bounds() {
        let mut t = Tui::new(
            Activity {
                rows: sample_rows(),
            },
            false,
        );
        t.move_cursor(1);
        assert_eq!(t.cursor, 1);
        t.move_cursor(10);
        assert_eq!(t.cursor, 2, "越界收敛到最后一行");
        t.move_cursor(-10);
        assert_eq!(t.cursor, 0, "越界收敛到首行");
    }

    #[test]
    fn tui_next_file_cycles_and_resets_cursor() {
        let mut t = Tui::new(
            Activity {
                rows: sample_rows(),
            },
            false,
        );
        t.move_cursor(2);
        t.next_file();
        assert_eq!(t.file_idx, 1);
        assert_eq!(t.cursor, 0);
        // 循环回到 all
        for _ in 0..(FILES.len() - 1) {
            t.next_file();
        }
        assert_eq!(t.file_idx, 0);
    }
}
