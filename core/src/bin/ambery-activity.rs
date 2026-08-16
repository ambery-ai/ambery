//! ambery-activity：storage 活动查看器。
//! 读取 Storage 目录下 JSONL 文件，交互查看内部消息流。
//! 目录参数默认取 `storage_dir`（`AMBERY_STORAGE_DIR` 可覆盖），也支持显式传目录。

use ambery_core::context::ContextMessage;
use ambery_core::queue::QueueInput;
use ambery_core::{
    AgentEntry, ContextLine, EffectRecord, TerminalContentRecord, CONTEXT_FILE, EFFECT_FILE,
    QUEUE_FILE, TERMINAL_CONTENT_FILE, WORK_AGENTS_FILE,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthChar;

/// 统一行模型：一个 JSONL 文件的一行记录折叠为一条可展示的 ActivityRow
#[derive(Debug, Clone, PartialEq)]
pub struct ActivityRow {
    /// 来源文件名（如 context.jsonl）
    pub file: &'static str,
    /// 行类型 / kind（message 的 role、effect 的 kind 等）
    pub kind: String,
    /// 记录时刻（epoch ms；缺失时 0）
    pub ts: i64,
    /// 单行摘要（列表渲染，截断）
    pub summary: String,
    /// 未截断全文（详情栏显示）
    pub detail: String,
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

/// 详情内容总行数（头部行 + 全文行）；超出 pane 高度时可滚动
fn detail_content_height(line: &Option<RenderedLine>, width: usize) -> usize {
    match line {
        Some(l) => detail_pane_lines(l, width).len(),
        None => 0,
    }
}

/// 按 pane 宽度把文本预折成视觉行（CJK 双宽感知）
fn wrap_to_width(s: &str, width: usize) -> Vec<String> {
    let w = width.max(1);
    let mut out = Vec::new();
    for line in s.lines() {
        if line.is_empty() {
            out.push(String::new());
            continue;
        }
        let mut buf = String::new();
        let mut cells = 0usize;
        for ch in line.chars() {
            let cw = UnicodeWidthChar::width(ch).unwrap_or(0);
            if cells + cw > w {
                out.push(std::mem::take(&mut buf));
                cells = 0;
            }
            buf.push(ch);
            cells += cw;
        }
        out.push(buf);
    }
    out
}

/// 详情栏内容（头部行 + 空行 + 全文，全部预折为视觉行）
fn detail_pane_lines(line: &RenderedLine, width: usize) -> Vec<String> {
    let mut out = wrap_to_width(&line.text, width);
    out.push(String::new());
    out.extend(wrap_to_width(&line.detail, width));
    out
}

fn read_context(dir: &Path) -> std::io::Result<Vec<ActivityRow>> {
    let mut rows = Vec::new();
    for line in read_lines(dir, CONTEXT_FILE)? {
        let Ok(l) = serde_json::from_str::<ContextLine>(&line) else {
            continue;
        };
        let (kind, ts, summary, detail) = match l {
            ContextLine::Message { msg } => context_message_row(&msg),
            ContextLine::Autonomy { content, ts } => {
                ("autonomy".into(), ts, truncate(&content, 60), content)
            }
            ContextLine::Head { content, ts } => ("head".into(), ts, truncate(&content, 60), content),
            ContextLine::Usage {
                prompt_tokens,
                completion_tokens,
                ts,
            } => (
                "usage".into(),
                ts,
                format!("prompt={prompt_tokens} completion={completion_tokens}"),
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
                format!("{}→{}: {}", pre_tokens, post_tokens, summary),
            ),
            ContextLine::Content { instance, ts, .. } => {
                ("content".into(), ts, format!("[legacy] {instance}"), instance)
            }
            ContextLine::Session { session_id, ts } => {
                ("session".into(), ts, session_id.clone(), session_id)
            }
        };
        rows.push(ActivityRow {
            file: CONTEXT_FILE,
            kind,
            ts,
            summary,
            detail,
        });
    }
    Ok(rows)
}

fn context_message_row(msg: &ContextMessage) -> (String, i64, String, String) {
    let role = format!("{:?}", msg.role).to_lowercase();
    let content = msg.content.as_deref().unwrap_or("");
    let (body, detail) = if let Some(calls) = &msg.tool_calls {
        let names: Vec<&str> = calls.iter().map(|c| c.name.as_str()).collect();
        (
            format!("tool_calls: {}", names.join(", ")),
            format!("tool_calls: {}\n\n{}", names.join(", "), content),
        )
    } else {
        (truncate(content, 60), content.to_string())
    };
    (format!("message/{role}"), msg.ts, body, detail)
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
            detail: q.content,
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
            detail: e.payload.to_string(),
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
            detail: format!("{}: {}", t.instance, t.raw),
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
            detail: format!("{} ({})", a.name, a.project),
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
            detail: line.clone(),
        });
    }
    Ok(rows)
}

fn line_brief(line: &str) -> String {
    truncate(line, 60)
}

// ── Trajectory 模式：turn-aware 紧凑事件账本 ──
// 参考 dsh 的 trajectory 概念：把平铺 JSONL 投影成
// session 边界 + turn（Queue 放行一轮） + 归属事件三层结构。
// 因果结构不靠行内文字推断，而靠 storage 里已有的一等事实：session 分界与 Queue 排队记录。

/// 轨迹账本行：边界行与普通事件行同池，渲染时用缩进区分层级
#[derive(Debug, Clone, PartialEq)]
pub enum TrajectoryRow {
    Session {
        id: String,
        ts: i64,
        detail: String,
    },
    Turn {
        index: usize,
        source: String,
        role: String,
        content: String,
        ts: i64,
        detail: String,
    },
    Event {
        file: &'static str,
        kind: String,
        ts: i64,
        summary: String,
        /// 归属 turn（None = 首个 turn 之前的孤儿事件）
        turn: Option<usize>,
        detail: String,
    },
}

#[derive(Debug, Default)]
pub struct Trajectory {
    pub rows: Vec<TrajectoryRow>,
    pub turns: usize,
    pub sessions: usize,
}

/// 折叠目标：光标所在行可折叠的对象（h 折叠 / l 展开）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FoldTarget {
    /// 无层级目标（事件行；h/l 无作用，折叠其归属 turn 需把光标移到 turn 行）
    None,
    /// session 序数（第 n 个 session，1 起）
    Session(usize),
    /// turn 索引（from_activity 的全局轮次序号，0 起）
    Turn(usize),
}

impl Trajectory {
    /// 从 Activity 统一行集投影轨迹：
    /// context `session` 行 = 会话边界；queue 行 = turn 边界；
    /// 其余行按 ts 归属到最近一个已出现的 turn。无 queue 数据时，
    /// context 的 user message 退化为 turn 边界（case 快照常见形态）。
    pub fn from_activity(activity: &Activity) -> Self {
        let has_queue = activity.rows.iter().any(|r| r.file == QUEUE_FILE);
        let mut rows = Vec::new();
        let mut turns = 0usize;
        let mut sessions = 0usize;
        let mut current_turn: Option<usize> = None;

        for r in &activity.rows {
            if r.file == CONTEXT_FILE && r.kind == "session" {
                sessions += 1;
                rows.push(TrajectoryRow::Session {
                    id: r.summary.clone(),
                    ts: r.ts,
                    detail: r.detail.clone(),
                });
                continue;
            }
            let is_turn = r.file == QUEUE_FILE
                || (!has_queue && r.file == CONTEXT_FILE && r.kind == "message/user");
            if is_turn {
                let index = turns;
                turns += 1;
                current_turn = Some(index);
                rows.push(TrajectoryRow::Turn {
                    index,
                    source: r.kind.clone(),
                    role: r
                        .file
                        .starts_with(QUEUE_FILE)
                        .then(|| "input".to_string())
                        .unwrap_or_else(|| "user".into()),
                    content: r.summary.clone(),
                    ts: r.ts,
                    detail: r.detail.clone(),
                });
                continue;
            }
            rows.push(TrajectoryRow::Event {
                file: r.file,
                kind: r.kind.clone(),
                ts: r.ts,
                summary: r.summary.clone(),
                turn: current_turn,
                detail: r.detail.clone(),
            });
        }
        Self {
            rows,
            turns,
            sessions,
        }
    }
}

/// 行类型：详情栏入口判定（Event / 平铺行 = 叶子，→/l 进详情栏）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowKind {
    Session,
    Turn,
    Event,
}

/// 渲染行：显示文案 + 折叠目标 + 未截断详情 + 行类型
#[derive(Debug, Clone)]
pub struct RenderedLine {
    pub text: String,
    pub target: FoldTarget,
    pub detail: String,
    pub kind: RowKind,
}

impl Trajectory {
    /// 渲染为 TUI 行：结构边界保留，事件行可按 session / turn 独立折叠。
    /// 每行携带折叠目标（h/l 按光标定位）与未截断详情（详情栏显示）。
    pub fn lines(
        &self,
        file: &str,
        filter: &str,
        folded_sessions: &HashSet<usize>,
        folded_turns: &HashSet<usize>,
    ) -> Vec<RenderedLine> {
        // 折叠计数：每 session 跨度行数（turn+event）、每 turn 的 event 数
        let mut session_hidden: HashMap<usize, usize> = HashMap::new();
        let mut turn_hidden: HashMap<usize, usize> = HashMap::new();
        {
            let mut sord = 0usize;
            let mut span = 0usize;
            let mut cur: Option<usize> = None;
            for row in &self.rows {
                match row {
                    TrajectoryRow::Session { .. } => {
                        if let Some(s) = cur {
                            session_hidden.insert(s, span);
                        }
                        sord += 1;
                        cur = Some(sord);
                        span = 0;
                    }
                    TrajectoryRow::Turn { .. } => span += 1,
                    TrajectoryRow::Event { turn, .. } => {
                        span += 1;
                        if let Some(t) = turn {
                            *turn_hidden.entry(*t).or_insert(0) += 1;
                        }
                    }
                }
            }
            if let Some(s) = cur {
                session_hidden.insert(s, span);
            }
        }

        let mut out: Vec<RenderedLine> = Vec::new();
        let mut sord = 0usize;
        let mut session_folded = false;
        for row in &self.rows {
            match row {
                TrajectoryRow::Session { id, ts, detail } => {
                    sord += 1;
                    session_folded = folded_sessions.contains(&sord);
                    let mut line = format!("── session {id} @{ts}");
                    if session_folded {
                        if let Some(n) = session_hidden.get(&sord) {
                            line.push_str(&format!("  [+{n}]"));
                        }
                    }
                    if (file == "all" || file == CONTEXT_FILE)
                        && (filter.is_empty() || id.contains(filter))
                    {
                        out.push(RenderedLine {
                            text: line,
                            target: FoldTarget::Session(sord),
                            detail: detail.clone(),
                            kind: RowKind::Session,
                        });
                    }
                }
                TrajectoryRow::Turn {
                    index,
                    source,
                    content,
                    ts,
                    detail,
                    ..
                } => {
                    if session_folded {
                        continue;
                    }
                    let mut line = format!("▸ turn {} [{}] {} @{ts}", index + 1, source, content);
                    if folded_turns.contains(index) {
                        if let Some(n) = turn_hidden.get(index) {
                            line.push_str(&format!("  [+{n}]"));
                        }
                    }
                    if (file == "all" || file == QUEUE_FILE)
                        && (filter.is_empty()
                            || source.contains(filter)
                            || content.contains(filter))
                    {
                        out.push(RenderedLine {
                            text: line,
                            target: FoldTarget::Turn(*index),
                            detail: detail.clone(),
                            kind: RowKind::Turn,
                        });
                    }
                }
                TrajectoryRow::Event {
                    file: f,
                    kind,
                    ts,
                    summary,
                    turn,
                    detail,
                } => {
                    if session_folded {
                        continue;
                    }
                    if let Some(t) = turn {
                        if folded_turns.contains(t) {
                            continue;
                        }
                    }
                    if (file == "all" || *f == file)
                        && (filter.is_empty()
                            || kind.contains(filter)
                            || summary.contains(filter))
                    {
                        // 事件行折叠目标 = 所属 turn（孤儿事件无可折叠祖先 → None）
                        let target = turn.map(FoldTarget::Turn).unwrap_or(FoldTarget::None);
                        out.push(RenderedLine {
                            text: format!("   · {ts} [{f}] {kind} {summary}"),
                            target,
                            detail: detail.clone(),
                            kind: RowKind::Event,
                        });
                    }
                }
            }
        }
        out
    }
}

/// CLI 选项（--dir 覆盖目录，--follow tail 新增，--trajectory 轨迹账本）
struct Options {
    dir: PathBuf,
    follow: bool,
    /// --dump：非交互，纯文本打印全部行（脚本/管道用；默认是 TUI）
    dump: bool,
    /// --trajectory：turn-aware 紧凑轨迹账本（session/turn/event 三层）
    trajectory: bool,
}

fn parse_args() -> Options {
    let mut dir = ambery_core::paths::storage_dir();
    let mut follow = false;
    let mut dump = false;
    let mut trajectory = false;
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
            "--trajectory" | "-t" => trajectory = true,
            _ => {}
        }
    }
    Options { dir, follow, dump, trajectory }
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
            } else if opt.trajectory {
                // trajectory：dsh 轨迹账本形态——session 边界 / turn 边界 / 归属事件
                let traj = Trajectory::from_activity(&a);
                if let Err(e) = run_trajectory_tui(opt.dir, traj, opt.follow) {
                    eprintln!("ambery-activity: {e}");
                    std::process::exit(1);
                }
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
    /// gg 双击检测（第一个 g 置位，第二个 g 跳顶）
    pending_g: bool,
    /// 详情栏聚焦（j/k 滚动全文，←/h 返回列表）
    in_detail: bool,
    detail_scroll: usize,
    /// 详情栏全屏（i 切换：隐藏列表，全文占满）
    detail_fullscreen: bool,
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
            pending_g: false,
            in_detail: false,
            detail_fullscreen: false,
            detail_scroll: 0,
            quit: false,
        };
        t.seen = t.activity.rows.len();
        t.pending_g = false;
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

    fn jump_top(&mut self) {
        self.cursor = 0;
        self.offset = 0;
    }

    fn jump_bottom(&mut self) {
        let len = self.view().len();
        if len > 0 {
            self.cursor = len - 1;
        }
    }

    /// 光标行（详情栏内容源）
    fn focused_row(&self) -> Option<ActivityRow> {
        self.view().get(self.cursor).map(|r| (*r).clone())
    }

    fn enter_detail(&mut self) {
        if self.focused_row().is_some() {
            self.in_detail = true;
            self.detail_scroll = 0;
        }
    }

    fn leave_detail(&mut self) {
        self.in_detail = false;
        self.detail_scroll = 0;
        self.detail_fullscreen = false;
    }

    fn toggle_fullscreen(&mut self) {
        self.detail_fullscreen = !self.detail_fullscreen;
        self.detail_scroll = 0;
    }

    fn scroll_detail(&mut self, delta: isize, height: usize, width: usize) {
        let content = match &self.focused_row() {
            Some(r) => {
                let mut v = wrap_to_width(
                    &format!("{} [{}] {} {}", r.ts, r.file, r.kind, r.summary),
                    width,
                );
                v.push(String::new());
                v.extend(wrap_to_width(&r.detail, width));
                v.len()
            }
            None => 0,
        };
        let max = content.saturating_sub(height);
        self.detail_scroll =
            (self.detail_scroll as isize + delta).clamp(0, max as isize) as usize;
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
        let focused = tui.focused_row();
        let detail_scroll = tui.detail_scroll;
        let in_detail = tui.in_detail;
        let detail_fullscreen = tui.detail_fullscreen;

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

            // 详情栏非驻留：仅在 →/l 打开（in_detail）时渲染；i 全屏时占满中间区
            if in_detail {
                let (list_area, detail_area) = if detail_fullscreen {
                    (None, chunks[1])
                } else {
                    let panes = Layout::default()
                        .direction(ratatui::layout::Direction::Horizontal)
                        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                        .split(chunks[1]);
                    (Some(panes[0]), panes[1])
                };

                if let Some(la) = list_area {
                    let items: Vec<ListItem> = view
                        .iter()
                        .skip(offset)
                        .take(height)
                        .map(|r| {
                            ListItem::new(format!("{} [{}] {} {}", r.ts, r.file, r.kind, r.summary))
                        })
                        .collect();
                    let mut state = ListState::default();
                    if !view.is_empty() {
                        state.select(Some(cursor.saturating_sub(offset)));
                    }
                    let list = List::new(items)
                        .block(Block::default().borders(Borders::NONE))
                        .highlight_symbol("▶ ");
                    f.render_stateful_widget(list, la, &mut state);
                }

                let detail_block = Block::default()
                    .borders(if detail_fullscreen {
                        Borders::TOP
                    } else {
                        Borders::LEFT
                    })
                    .title(if detail_fullscreen {
                        "detail [fullscreen]"
                    } else {
                        "detail [focused]"
                    });
                let pane_width = detail_area.width as usize;
                let pane_height = detail_area.height as usize;
                match &focused {
                    Some(row) => {
                        let mut pane_lines = wrap_to_width(
                            &format!("{} [{}] {} {}", row.ts, row.file, row.kind, row.summary),
                            pane_width,
                        );
                        pane_lines.push(String::new());
                        pane_lines.extend(wrap_to_width(&row.detail, pane_width));
                        let body = pane_lines
                            .iter()
                            .skip(detail_scroll)
                            .take(pane_height)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n");
                        f.render_widget(Paragraph::new(body).block(detail_block), detail_area);
                    }
                    None => {
                        f.render_widget(Paragraph::new("(无焦点行)").block(detail_block), detail_area);
                    }
                }
            } else {
                let items: Vec<ListItem> = view
                    .iter()
                    .skip(offset)
                    .take(height)
                    .map(|r| {
                        ListItem::new(format!("{} [{}] {} {}", r.ts, r.file, r.kind, r.summary))
                    })
                    .collect();
                let mut state = ListState::default();
                if !view.is_empty() {
                    state.select(Some(cursor.saturating_sub(offset)));
                }
                let list = List::new(items)
                    .block(Block::default().borders(Borders::NONE))
                    .highlight_symbol("▶ ");
                f.render_stateful_widget(list, chunks[1], &mut state);
            }

            let help = if filtering {
                format!("/{}", filter)
            } else if in_detail {
                if detail_fullscreen {
                    "detail 全屏: ↑/↓/j/k 滚动 i/Esc 退出全屏 q 退出".to_string()
                } else {
                    "detail: ↑/↓/j/k 滚动 i 全屏 ←/h/Esc 关闭 q 退出".to_string()
                }
            } else {
                "↑/↓/j/k 移动 →/l 详情 ←/h 返回 gg/G 跳首尾 Tab 切源 / 筛选 f 跟随 q 退出".to_string()
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
                } else if tui.in_detail {
                    let area = terminal.get_frame().area();
                    let pane_h = area.height.saturating_sub(4) as usize;
                    let pane_w = if tui.detail_fullscreen {
                        area.width as usize
                    } else {
                        (area.width as usize * 40) / 100
                    };
                    match k.code {
                        KeyCode::Char('q') => tui.quit = true,
                        KeyCode::Char('i') => tui.toggle_fullscreen(),
                        KeyCode::Up | KeyCode::Char('k') => tui.scroll_detail(-1, pane_h, pane_w),
                        KeyCode::Down | KeyCode::Char('j') => tui.scroll_detail(1, pane_h, pane_w),
                        KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => {
                            if tui.detail_fullscreen {
                                tui.toggle_fullscreen();
                            } else {
                                tui.leave_detail();
                            }
                        }
                        _ => {}
                    }
                } else {
                    let prev_g = tui.pending_g;
                    tui.pending_g = false;
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => tui.quit = true,
                        KeyCode::Up | KeyCode::Char('k') => tui.move_cursor(-1),
                        KeyCode::Down | KeyCode::Char('j') => tui.move_cursor(1),
                        KeyCode::Tab => tui.next_file(),
                        KeyCode::Char('/') => tui.filtering = true,
                        KeyCode::Char('G') => tui.jump_bottom(),
                        KeyCode::Char('g') => {
                            if prev_g {
                                tui.jump_top();
                            } else {
                                tui.pending_g = true;
                            }
                        }
                        KeyCode::Right | KeyCode::Char('l') => tui.enter_detail(),
                        KeyCode::Left | KeyCode::Char('h') => {}
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

// ---- Trajectory TUI：turn-aware 轨迹账本 ----

struct TrajectoryTui {
    trajectory: Trajectory,
    file_idx: usize,
    cursor: usize,
    offset: usize,
    filter: String,
    filtering: bool,
    follow: bool,
    seen: usize,
    folded_sessions: HashSet<usize>,
    folded_turns: HashSet<usize>,
    pending_g: bool,
    /// 详情栏聚焦（j/k 滚动全文，←/h 返回列表）
    in_detail: bool,
    detail_scroll: usize,
    /// 详情栏全屏（i 切换：隐藏列表，全文占满）
    detail_fullscreen: bool,
    quit: bool,
}

impl TrajectoryTui {
    fn new(trajectory: Trajectory, follow: bool) -> Self {
        let seen = trajectory.rows.len();
        Self {
            trajectory,
            file_idx: 0,
            cursor: 0,
            offset: 0,
            filter: String::new(),
            filtering: false,
            follow,
            seen,
            folded_sessions: HashSet::new(),
            folded_turns: HashSet::new(),
            pending_g: false,
            in_detail: false,
            detail_fullscreen: false,
            detail_scroll: 0,
            quit: false,
        }
    }

    fn lines(&self) -> Vec<RenderedLine> {
        self.trajectory.lines(
            FILES[self.file_idx],
            &self.filter,
            &self.folded_sessions,
            &self.folded_turns,
        )
    }

    /// 光标行（详情栏内容源）
    fn focused_line(&self) -> Option<RenderedLine> {
        self.lines().get(self.cursor).cloned()
    }

    fn enter_detail(&mut self) {
        // 只有叶子（trajectory 事件行）进详情栏；父节点（session/turn）由 →/l 展开
        if self.cursor_kind() == Some(RowKind::Event) {
            self.in_detail = true;
            self.detail_scroll = 0;
        }
    }

    fn leave_detail(&mut self) {
        self.in_detail = false;
        self.detail_scroll = 0;
        self.detail_fullscreen = false;
    }

    fn toggle_fullscreen(&mut self) {
        self.detail_fullscreen = !self.detail_fullscreen;
        self.detail_scroll = 0;
    }

    fn scroll_detail(&mut self, delta: isize, height: usize, width: usize) {
        let content = detail_content_height(&self.focused_line(), width);
        let max = content.saturating_sub(height);
        self.detail_scroll =
            (self.detail_scroll as isize + delta).clamp(0, max as isize) as usize;
    }

    fn move_cursor(&mut self, delta: isize) {
        let len = self.lines().len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        self.cursor = (self.cursor as isize + delta).clamp(0, len as isize - 1) as usize;
    }

    fn next_file(&mut self) {
        self.file_idx = (self.file_idx + 1) % FILES.len();
        self.cursor = 0;
        self.offset = 0;
    }

    fn jump_top(&mut self) {
        self.cursor = 0;
        self.offset = 0;
        self.detail_scroll = 0;
    }

    fn jump_bottom(&mut self) {
        let len = self.lines().len();
        if len > 0 {
            self.cursor = len - 1;
        }
        self.detail_scroll = 0;
    }

    /// 光标行的折叠目标（h/l 作用对象）
    fn cursor_target(&self) -> FoldTarget {
        self.lines()
            .get(self.cursor)
            .map(|l| l.target)
            .unwrap_or(FoldTarget::None)
    }

    /// 光标行的类型（叶子 = Event → →/l 进详情栏）
    fn cursor_kind(&self) -> Option<RowKind> {
        self.lines().get(self.cursor).map(|l| l.kind)
    }

    fn fold_at(&mut self, target: FoldTarget) {
        match target {
            FoldTarget::Session(s) => {
                self.folded_sessions.insert(s);
            }
            FoldTarget::Turn(t) => {
                self.folded_turns.insert(t);
            }
            FoldTarget::None => return,
        }
        self.clamp_cursor();
    }

    fn unfold_at(&mut self, target: FoldTarget) {
        match target {
            FoldTarget::Session(s) => {
                self.folded_sessions.remove(&s);
            }
            FoldTarget::Turn(t) => {
                self.folded_turns.remove(&t);
            }
            FoldTarget::None => return,
        }
        self.clamp_cursor();
    }

    fn clamp_cursor(&mut self) {
        let len = self.lines().len();
        if len == 0 {
            self.cursor = 0;
        } else if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    fn reload(&mut self, dir: &Path) {
        if let Ok(a) = Activity::load(dir) {
            let traj = Trajectory::from_activity(&a);
            if traj.rows.len() > self.seen {
                self.seen = traj.rows.len();
                self.trajectory = traj;
                if self.follow {
                    let len = self.lines().len();
                    if len > 0 {
                        self.cursor = len - 1;
                    }
                }
            }
        }
    }
}

fn run_trajectory_tui(dir: PathBuf, trajectory: Trajectory, follow: bool) -> std::io::Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    let mut tui = TrajectoryTui::new(trajectory, follow);

    loop {
        if tui.follow {
            tui.reload(&dir);
        }
        let lines = tui.lines();
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
        let focused = tui.focused_line();
        let detail_scroll = tui.detail_scroll;
        let in_detail = tui.in_detail;
        let detail_fullscreen = tui.detail_fullscreen;

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
                "ambery-activity trajectory  sessions={} turns={} rows={} file={} filter={}{}",
                tui.trajectory.sessions,
                tui.trajectory.turns,
                lines.len(),
                file_label,
                filter,
                if follow { "  [follow]" } else { "" }
            );
            f.render_widget(
                Paragraph::new(title).block(Block::default().borders(Borders::BOTTOM)),
                chunks[0],
            );

            // 详情栏非驻留：仅在 →/l 打开（in_detail）时渲染；i 全屏时占满中间区
            if in_detail {
                let (list_area, detail_area) = if detail_fullscreen {
                    (None, chunks[1])
                } else {
                    let panes = Layout::default()
                        .direction(ratatui::layout::Direction::Horizontal)
                        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                        .split(chunks[1]);
                    (Some(panes[0]), panes[1])
                };

                if let Some(la) = list_area {
                    let items: Vec<ListItem> = lines
                        .iter()
                        .skip(offset)
                        .take(height)
                        .map(|l| ListItem::new(l.text.clone()))
                        .collect();
                    let mut state = ListState::default();
                    if !lines.is_empty() {
                        state.select(Some(cursor.saturating_sub(offset)));
                    }
                    let list = List::new(items)
                        .block(Block::default().borders(Borders::NONE))
                        .highlight_symbol("▶ ");
                    f.render_stateful_widget(list, la, &mut state);
                }

                let detail_block = Block::default()
                    .borders(if detail_fullscreen {
                        Borders::TOP
                    } else {
                        Borders::LEFT
                    })
                    .title(if detail_fullscreen {
                        "detail [fullscreen]"
                    } else {
                        "detail [focused]"
                    });
                let pane_width = detail_area.width as usize;
                let pane_height = detail_area.height as usize;
                match &focused {
                    Some(line) => {
                        let pane_lines = detail_pane_lines(line, pane_width);
                        let body = pane_lines
                            .iter()
                            .skip(detail_scroll)
                            .take(pane_height)
                            .cloned()
                            .collect::<Vec<_>>()
                            .join("\n");
                        f.render_widget(Paragraph::new(body).block(detail_block), detail_area);
                    }
                    None => {
                        f.render_widget(Paragraph::new("(无焦点行)").block(detail_block), detail_area);
                    }
                }
            } else {
                let items: Vec<ListItem> = lines
                    .iter()
                    .skip(offset)
                    .take(height)
                    .map(|l| ListItem::new(l.text.clone()))
                    .collect();
                let mut state = ListState::default();
                if !lines.is_empty() {
                    state.select(Some(cursor.saturating_sub(offset)));
                }
                let list = List::new(items)
                    .block(Block::default().borders(Borders::NONE))
                    .highlight_symbol("▶ ");
                f.render_stateful_widget(list, chunks[1], &mut state);
            }

            let help = if filtering {
                format!("/{}", filter)
            } else if in_detail {
                if detail_fullscreen {
                    "detail 全屏: ↑/↓/j/k 滚动 i/Esc 退出全屏 q 退出".to_string()
                } else {
                    "detail: ↑/↓/j/k 滚动 i 全屏 ←/h/Esc 关闭 q 退出".to_string()
                }
            } else {
                "↑/↓/j/k 移动 ←/h 折叠 →/l 展开/详情 gg/G 跳首尾 Tab 切源 / 筛选 f 跟随 q 退出".to_string()
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
                } else if tui.in_detail {
                    let area = terminal.get_frame().area();
                    let pane_h = area.height.saturating_sub(4) as usize;
                    let pane_w = if tui.detail_fullscreen {
                        area.width as usize
                    } else {
                        (area.width as usize * 40) / 100
                    };
                    match k.code {
                        KeyCode::Char('q') => tui.quit = true,
                        KeyCode::Char('i') => tui.toggle_fullscreen(),
                        KeyCode::Up | KeyCode::Char('k') => tui.scroll_detail(-1, pane_h, pane_w),
                        KeyCode::Down | KeyCode::Char('j') => tui.scroll_detail(1, pane_h, pane_w),
                        KeyCode::Left | KeyCode::Char('h') | KeyCode::Esc => {
                            if tui.detail_fullscreen {
                                tui.toggle_fullscreen();
                            } else {
                                tui.leave_detail();
                            }
                        }
                        _ => {}
                    }
                } else {
                    let prev_g = tui.pending_g;
                    tui.pending_g = false;
                    match k.code {
                        KeyCode::Char('q') | KeyCode::Esc => tui.quit = true,
                        KeyCode::Up | KeyCode::Char('k') => tui.move_cursor(-1),
                        KeyCode::Down | KeyCode::Char('j') => tui.move_cursor(1),
                        KeyCode::Tab => tui.next_file(),
                        KeyCode::Char('/') => tui.filtering = true,
                        KeyCode::Char('G') => tui.jump_bottom(),
                        KeyCode::Char('g') => {
                            if prev_g {
                                tui.jump_top();
                            } else {
                                tui.pending_g = true;
                            }
                        }
                        KeyCode::Left | KeyCode::Char('h') => {
                            let target = tui.cursor_target();
                            tui.fold_at(target);
                        }
                        KeyCode::Right | KeyCode::Char('l') => match tui.cursor_kind() {
                            Some(RowKind::Event) => tui.enter_detail(),
                            Some(_) => {
                                let target = tui.cursor_target();
                                tui.unfold_at(target);
                            }
                            None => {}
                        },
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
                detail: "你好，这是完整的用户消息全文。".into(),
            },
            ActivityRow {
                file: QUEUE_FILE,
                kind: "user_chat".into(),
                ts: 2,
                summary: "hi".into(),
                detail: "hi".into(),
            },
            ActivityRow {
                file: EFFECT_FILE,
                kind: "backend/render_component".into(),
                ts: 3,
                summary: "{}".into(),
                detail: r#"{"kind":"render_component","payload":{"id":"c1"}}"#.into(),
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

    fn trajectory_sample() -> Activity {
        Activity {
            rows: vec![
                ActivityRow { file: CONTEXT_FILE, kind: "session".into(), ts: 1, summary: "s1".into(), detail: "s1".into() },
                ActivityRow { file: QUEUE_FILE, kind: "user_chat".into(), ts: 2, summary: "用户问".into(), detail: "用户问的完整内容全文".into() },
                ActivityRow { file: EFFECT_FILE, kind: "backend/render_component".into(), ts: 3, summary: r#"{"id":"c1"}"#.into(), detail: r#"{"kind":"render_component","payload":{"id":"c1","text":"完整卡片内容"}}"#.into() },
                ActivityRow { file: CONTEXT_FILE, kind: "message/assistant".into(), ts: 4, summary: "回复".into(), detail: "助手的完整回复全文，超过列表截断长度以便验证详情栏展示".into() },
            ],
        }
    }

    #[test]
    fn trajectory_groups_sessions_turns_and_events() {
        let traj = Trajectory::from_activity(&trajectory_sample());
        assert_eq!(traj.sessions, 1);
        assert_eq!(traj.turns, 1);
        assert_eq!(traj.rows.len(), 4);
        assert!(matches!(&traj.rows[0], TrajectoryRow::Session { id, .. } if id == "s1"));
        assert!(matches!(&traj.rows[1], TrajectoryRow::Turn { index: 0, source, .. } if source == "user_chat"));
        // ts=3 的事件挂在 turn 0 名下
        assert!(matches!(&traj.rows[2], TrajectoryRow::Event { turn: Some(0), .. }));
        assert!(matches!(&traj.rows[3], TrajectoryRow::Event { turn: Some(0), .. }));
    }

    #[test]
    fn trajectory_fold_is_per_item() {
        let traj = Trajectory::from_activity(&trajectory_sample());
        let empty_s = HashSet::new();
        let empty_t = HashSet::new();

        let flat = traj.lines("all", "", &empty_s, &empty_t);
        assert_eq!(flat.len(), 4);
        assert!(flat[0].text.contains("session s1"));
        assert!(flat[1].text.contains("turn 1"));
        assert_eq!(flat[1].target, FoldTarget::Turn(0));
        // 事件行折叠目标 = 所属 turn（子对象可向上折叠）；行类型 = 叶子
        assert_eq!(flat[2].target, FoldTarget::Turn(0));
        assert_eq!(flat[2].kind, RowKind::Event);
        assert_eq!(flat[3].target, FoldTarget::Turn(0));

        // 折叠 turn 0：事件隐藏，session/turn 边界保留，turn 行带 [+2] 标记
        let folded_t = traj.lines("all", "", &empty_s, &HashSet::from([0]));
        assert_eq!(folded_t.len(), 2, "{folded_t:?}");
        assert!(!folded_t.iter().any(|l| l.text.contains("render_component")));
        assert!(folded_t[1].text.contains("[+2]"));

        // 折叠 session 1：turn+事件全隐藏，仅 session 行，带 [+3]
        let folded_s = traj.lines("all", "", &HashSet::from([1]), &empty_t);
        assert_eq!(folded_s.len(), 1, "{folded_s:?}");
        assert!(folded_s[0].text.contains("[+3]"));

        // 单条目独立：turn 折叠不影响 session 折叠的计数与展示
        let both = traj.lines("all", "", &HashSet::from([1]), &HashSet::from([0]));
        assert_eq!(both.len(), 1, "session 折叠优先于 turn 折叠");

        // 孤儿事件（首个 turn 之前）无可折叠祖先 → None
        let orphan = Trajectory {
            rows: vec![TrajectoryRow::Event {
                file: EFFECT_FILE,
                kind: "k".into(),
                ts: 0,
                summary: "o".into(),
                turn: None,
                detail: "o".into(),
            }],
            turns: 0,
            sessions: 0,
        };
        let ol = orphan.lines("all", "", &empty_s, &empty_t);
        assert_eq!(ol[0].target, FoldTarget::None);
    }

    #[test]
    fn trajectory_lines_carry_full_detail() {
        let traj = Trajectory::from_activity(&trajectory_sample());
        let empty_s = HashSet::new();
        let empty_t = HashSet::new();
        let lines = traj.lines("all", "", &empty_s, &empty_t);
        // 事件行详情 = 未截断全文（列表 summary 是截断的，detail 是完整的）
        assert!(lines[2].detail.contains("完整卡片内容"));
        // turn 行详情 = queue 原文
        assert!(lines[1].detail.contains("用户问的完整内容全文"));
        // session 行详情 = id
        assert!(lines[0].detail.contains("s1"));
    }

    #[test]
    fn tui_detail_pane_navigation() {
        let mut t = Tui::new(
            Activity {
                rows: sample_rows(),
            },
            false,
        );
        // 列表态 → →/l 进详情栏（平铺任意行 = 叶子）
        t.enter_detail();
        assert!(t.in_detail);
        let focused = t.focused_row().unwrap();
        assert!(focused.detail.contains("完整"));
        // 详情栏滚动（宽度感知：长行折行后占多视觉行）
        t.scroll_detail(1, 5, 40);
        assert_eq!(t.detail_scroll, 0, "内容短于 pane 高度时不滚动");
        // 全屏切换（i）：toggle，离开详情栏重置
        t.toggle_fullscreen();
        assert!(t.detail_fullscreen);
        t.toggle_fullscreen();
        assert!(!t.detail_fullscreen);
        t.toggle_fullscreen();
        t.leave_detail();
        assert!(!t.detail_fullscreen, "离开详情栏重置全屏");
        assert!(!t.in_detail);
    }

    #[test]
    fn trajectory_detail_entry_only_on_leaves() {
        let traj = Trajectory::from_activity(&trajectory_sample());
        let mut t = TrajectoryTui::new(traj, false);
        // 光标在 turn（父节点）→ →/l 不进详情栏
        assert_eq!(t.cursor_kind(), Some(RowKind::Session));
        t.move_cursor(1);
        assert_eq!(t.cursor_kind(), Some(RowKind::Turn));
        t.enter_detail();
        assert!(!t.in_detail, "父节点不进详情栏");
        // 光标移到事件（叶子）→ 进详情栏
        t.move_cursor(1);
        assert_eq!(t.cursor_kind(), Some(RowKind::Event));
        t.enter_detail();
        assert!(t.in_detail);
        let focused = t.focused_line().unwrap();
        assert!(focused.detail.contains("完整"));
        // 全屏切换（i）：toggle，离开详情栏重置
        t.toggle_fullscreen();
        assert!(t.detail_fullscreen);
        t.toggle_fullscreen();
        assert!(!t.detail_fullscreen);
        t.toggle_fullscreen();
        t.leave_detail();
        assert!(!t.detail_fullscreen, "离开详情栏重置全屏");
    }

    #[test]
    fn wrap_to_width_respects_cjk_double_width() {
        // 20 个中文字 = 40 格，宽 20 格 → 2 视觉行
        let cjk = "你是Ambery的看板宠物你是Ambery的看板宠物";
        let lines = wrap_to_width(cjk, 20);
        assert_eq!(lines.len(), 2, "{lines:?}");
        // 纯 ASCII：宽 10 → 每行 10 字符
        let ascii = wrap_to_width("abcdefghijklmnopqrstuvwxyz", 10);
        assert_eq!(ascii.len(), 3, "{ascii:?}");
        assert_eq!(ascii[0].len(), 10);
        // 空行保留
        assert_eq!(wrap_to_width("a\n\nb", 10), vec!["a".to_string(), String::new(), "b".to_string()]);
    }

    #[test]
    fn tui_jump_top_and_bottom() {
        let mut t = Tui::new(
            Activity {
                rows: sample_rows(),
            },
            false,
        );
        t.jump_bottom();
        assert_eq!(t.cursor, 2);
        t.jump_top();
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn trajectory_jump_top_and_bottom() {
        let traj = Trajectory::from_activity(&trajectory_sample());
        let mut t = TrajectoryTui::new(traj, false);
        t.jump_bottom();
        assert_eq!(t.cursor, 3);
        t.jump_top();
        assert_eq!(t.cursor, 0);
    }
}
