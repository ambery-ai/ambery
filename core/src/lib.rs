//! Harness（concepts §10，docs/harness.md）：ペット和 OverseerBackend 共享的数据层。
//! Queue（输入排队）/ Context（消息数组）/ Content 存档 / Event Buffer / agents 注册表
//! + JSONL Storage replay。

// derive 宏展开内引用 ::overseer_core 路径（docs/observability.md），
// 别名让 crate 内外（含 doctest/下游）都能解析（serde 同款手法）
extern crate self as overseer_core;

pub mod config;
pub mod content;
pub mod context;
pub mod cron;
pub mod event_buffer;
pub mod filter;
pub mod lifecycle;
pub mod llm;
pub mod memory;
#[cfg(feature = "mock")]
pub mod mock;
#[cfg(feature = "case-runner")]
pub mod case;
#[cfg(feature = "case-runner")]
pub mod eval;
#[cfg(feature = "case-runner")]
pub mod observe;
pub mod overseer;
pub mod paths;
pub mod queue;
pub mod server;
pub mod sidecar;
pub mod storage;
pub mod timer;

pub use config::{Config, KaomojiEntry, LlmConfig, LlmProvider, CONFIG_FILE};
pub use content::{FilteredContent, RecordSource, TerminalContentRecord};
use context::{Context, ContextMessage};
use event_buffer::EventBuffer;
use queue::{Queue, QueueInput};
use serde::{Deserialize, Serialize};
use storage::JsonlStore;

pub const CONTEXT_FILE: &str = "context.jsonl";
/// Queue 输入排队记录（排队轨迹非对话本体，docs/storage.md）
pub const QUEUE_FILE: &str = "queue.jsonl";
/// Terminal Content 原文存档（Filter 前，docs/storage.md）
pub const TERMINAL_CONTENT_FILE: &str = "terminal-content.jsonl";
/// work-agents 注册表（被盯的干活 Code CLI 实例清单，append-only upsert 日志）
pub const WORK_AGENTS_FILE: &str = "work-agents.jsonl";
/// 前后端统一动作流（Effect：后端副作用 + 前端非只读调用，docs/storage.md §effect.jsonl）
pub const EFFECT_FILE: &str = "effect.jsonl";

/// 动作流记录（effect.jsonl 行）：{"type":"effect","origin","kind","payload","ts"}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EffectRecord {
    pub origin: EffectOrigin,
    pub kind: String,
    pub payload: serde_json::Value,
    pub ts: i64,
}

/// 动作发起者（serde 小写："frontend" / "backend"）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffectOrigin {
    Frontend,
    Backend,
}

impl EffectOrigin {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Frontend => "frontend",
            Self::Backend => "backend",
        }
    }
}
/// AGENTS.md（通用约定名）：ペット的身份提示词，与 base_prompt 拼接进 system prompt。
/// Config 域：存 config 根目录而非 storage（docs/storage.md）
pub const AGENTS_MD_FILE: &str = "AGENTS.md";

/// context.jsonl 统一信封（docs/storage.md）：每行 {type, ts, ...}，
/// 一个文件装下复原 OpenAI 上下文所需的一切——日志神圣，视图易失。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextLine {
    /// ContextMessage 全保真（含 tool_calls/reasoning_content）
    Message {
        #[serde(flatten)]
        msg: ContextMessage,
    },
    /// Autonomy 状态记录：每轮一条，最新一条挂请求末端（concepts §4）
    Autonomy { content: String, ts: i64 },
    /// LLM 调用 token 真值（#16，docs/storage.md §usage）：每次调用一条，
    /// 读取覆盖语义取最新；replay 不恢复为压缩基准（重启 last_usage=None）
    Usage {
        prompt_tokens: u64,
        completion_tokens: u64,
        ts: i64,
    },
    /// 请求头快照：装配结果变化才写
    Head { content: String, ts: i64 },
    /// 压缩边界：标记不是删除，文件全保留（可审计）
    CompactBoundary {
        summary: String,
        pre_tokens: usize,
        post_tokens: usize,
        duration_ms: u64,
        ts: i64,
    },
    /// Filter 后归一全文——**已退役**（现算定案：不持久化，从 terminal-content.jsonl 原文
    /// digest 现算，docs/storage.md §filtered_content 退役）。保留 variant 只为旧文件
    /// replay 兼容（读取后忽略），新代码不再写。
    Content {
        instance: String,
        content: String,
        source: RecordSource,
        ts: i64,
    },
    /// 会话分界：每次启动一条
    Session { session_id: String, ts: i64 },
}

/// concepts §9a Status 状态机（closed：Timer 发现 tab 不复存在的终态，docs/storage.md）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Processing,
    Unknown,
    Closed,
}

/// agents 注册表条目（work-agents.jsonl 永久事件日志：每次状态变更一行完整快照）
/// hash 区分每次生命周期——名字会重复，同名不同命（docs/storage.md）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEntry {
    /// 真实 hook：sid8（session_id 前 8 位，docs/hook.md）；mock：agent_hash 回退
    pub hash: String,
    /// display 名 = `<project>·<sid8>`，同时就是 tab 定位 marker（一名两用）
    pub name: String,
    pub project: String,
    /// CLI 种类（"claude"，per-instance filter 策略输入，docs/filter.md）
    #[serde(default)]
    pub kind: Option<String>,
    pub status: AgentStatus,
    /// tab 定位快照（与 status 同待遇：快照字段，投影取最新；无原地更新）
    #[serde(default)]
    pub tab: Option<TabRef>,
    pub first_seen: i64,
    pub last_seen: i64,
}

/// tab 定位（docs/hook.md §定位缓存）
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TabRef {
    pub hwnd: i64,
    pub index: i64,
}

/// 实例身份 = session_id 前 8 位（docs/hook.md §marker 定位）
pub fn sid8(session_id: &str) -> String {
    session_id.chars().take(8).collect()
}

/// display 名 = `<project>·<sid8>`（与 tab marker 同构，一名两用）
pub fn instance_name(project: &str, session_id: &str) -> String {
    format!("{project}·{}", sid8(session_id))
}

/// 生命周期 hash：short_hash(name + project + first_seen)——mock hook 无 session_id 时回退
pub fn agent_hash(name: &str, project: &str, first_seen: i64) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (name, project, first_seen).hash(&mut h);
    format!("{:08x}", h.finish() as u32)
}

/// derive Observe（docs/observability.md）：每个字段必须实现 Observable 或显式
/// skip 写理由——新增概念模块不声明可观测性即 E0277（编译期强制覆盖）。
#[cfg_attr(feature = "case-runner", derive(overseer_observe_derive::Observe))]
pub struct Harness {
    /// Queue（concepts §10c）：输入串行化关口，只装待放行输入
    pub queue: Queue,
    /// Context（concepts §10b）：完整消息数组，LLM 请求的上下文源
    pub context: Context,
    pub event_buffer: EventBuffer,
    pub agents: Vec<AgentEntry>,
    /// 最近一次写入的请求头（head diff：变化才写，docs/storage.md）
    #[cfg_attr(feature = "case-runner", observe(skip = "装配留痕（head diff 审计），非概念模块"))]
    pub last_head: Option<String>,
    /// 最近一次 LLM 调用的 token 真值（#16；重启 = None，不背旧 session）
    pub last_usage: Option<crate::llm::Usage>,
    /// last_usage 落点时的 Context 消息数（#16 增量估算基准：其后新增 = est 增量）
    #[cfg_attr(feature = "case-runner", observe(skip = "est 增量推导基准（派生数据，经 context_est_delta 现算观测）"))]
    pub last_usage_msg_len: usize,
    /// last_usage 写入时的 ts（usage observe 项的时间锚点，docs/case-runner.md §observe 输出）
    #[cfg_attr(feature = "case-runner", observe(skip = "usage 行 ts 锚点（派生数据，随 last_usage 经 usage 项同步观测）"))]
    pub last_usage_ts: Option<i64>,
    /// Memory（concepts §10f，docs/memory.md）：持久化理解 buffer 根管理器
    #[cfg_attr(feature = "case-runner", observe(skip = "§10f 概念模块，observe 项未定义"))]
    pub memory: memory::Memory,
    /// Cron（concepts §10g，docs/cron.md）：持久化计划与延时调度（entries 持久化
    /// + sleep waiters 共享句柄）
    #[cfg_attr(feature = "case-runner", observe(skip = "§10g 概念模块，observe 项未定义"))]
    pub cron: cron::CronScheduler,
    #[cfg_attr(feature = "case-runner", observe(skip = "JSONL 持久化句柄（机制非概念）"))]
    store: JsonlStore,
    #[cfg_attr(feature = "case-runner", observe(skip = "Config 域路径（机制非概念）"))]
    config_dir: std::path::PathBuf,
}

/// 存活实例全景（归零重 diff，docs/storage.md）：启动与压缩后重建 LLM 全局认知。
/// closed 是终态——永久日志必须有消亡语义，否则全景无限累积尸体；无存活实例返回 None。
pub fn panorama(agents: &[AgentEntry]) -> Option<String> {
    let alive: Vec<&AgentEntry> = agents
        .iter()
        .filter(|a| a.status != AgentStatus::Closed)
        .collect();
    if alive.is_empty() {
        return None;
    }
    let mut s = format!("实例全景同步（归零重 diff，{} 个存活实例）：", alive.len());
    for a in alive {
        s.push_str(&format!("\n- {} [{:?}] project={}", a.name, a.status, a.project));
    }
    Some(s)
}

impl Harness {
    /// 启动：replay JSONL 恢复世界状态（concepts §13「跨生命周期保留」）。
    /// Context 是内存视图：起步为空 + 写 session 标记 + 存活实例归零重同步（docs/storage.md）。
    pub fn load(
        dir: &std::path::Path,
        config_dir: &std::path::Path,
        token_threshold: usize,
        ts: i64,
    ) -> std::io::Result<Self> {
        let store = JsonlStore::new(dir)?;
        // context.jsonl 统一信封 replay：head → last_head；其余为历史留痕。
        // content 行（归一全文持久存档）已退役：replay 忽略（现算定案，docs/storage.md）
        let mut last_head = None;
        for line in store.read_all::<ContextLine>(CONTEXT_FILE)? {
            match line {
                ContextLine::Head { content, .. } => last_head = Some(content),
                _ => {}
            }
        }
        // agents 是 upsert 日志：replay 须逐条折叠（同 id 取最后一条）
        let mut agents: Vec<AgentEntry> = vec![];
        for entry in store.read_all::<AgentEntry>(WORK_AGENTS_FILE)? {
            apply_agent(&mut agents, entry);
        }
        // AGENTS.md 不存在 → 写入默认身份提示词（Config 域 bootstrap，docs/storage.md）
        let agents_md_path = config_dir.join(AGENTS_MD_FILE);
        if !agents_md_path.exists() {
            std::fs::create_dir_all(config_dir)?;
            std::fs::write(agents_md_path, default_agents_md())?;
        }
        // Memory 根 bootstrap（concepts §10f，docs/memory.md：storage/memory/ + 只读 AGENTS.md/index.md）
        let memory = memory::Memory::bootstrap(dir)?;
        // Cron 调度器：replay cron.jsonl 折叠计划集（concepts §10g，docs/cron.md）
        let cron = cron::CronScheduler::load(dir)?;
        let mut h = Self {
            // Queue 不 replay：崩溃丢失未放行输入可接受（docs/storage.md 设计决定）
            queue: Queue::default(),
            context: Context::new(token_threshold),
            // Event Buffer 不持久化：暂存区语义，崩溃丢失可接受（docs/harness.md 设计决定）
            event_buffer: EventBuffer::default(),
            agents,
            last_head,
            last_usage: None, // 重启 = None（#16：不背旧 session，首轮 est 兜底）
            last_usage_msg_len: 0,
            last_usage_ts: None,
            memory,
            cron,
            store,
            config_dir: config_dir.to_path_buf(),
        };
        // session 分界 + 启动归零重同步（存活实例全景一条 system 消息，落 message 行）
        h.log_session(&format!("{:x}", ts), ts)?;
        if let Some(p) = panorama(&h.agents) {
            h.append_context(ContextMessage::new(context::Role::System, p, ts))?;
        }
        Ok(h)
    }

    /// 入队一条输入：内存 Queue + queue.jsonl 留痕双写（docs/storage.md）
    pub fn enqueue_input(&mut self, input: QueueInput) -> std::io::Result<()> {
        self.store.append(QUEUE_FILE, &input)?;
        self.queue.enqueue(input);
        Ok(())
    }

    /// 追加消息：内存 Context + context.jsonl message 行双写（docs/storage.md）
    pub fn append_context(&mut self, msg: ContextMessage) -> std::io::Result<()> {
        self.store
            .append(CONTEXT_FILE, &ContextLine::Message { msg: msg.clone() })?;
        self.context.push(msg);
        Ok(())
    }

    /// Terminal Content 原文存档（Filter 前；平时不读、启动不 replay）
    pub fn append_terminal_content(&self, rec: TerminalContentRecord) -> std::io::Result<()> {
        self.store.append(TERMINAL_CONTENT_FILE, &rec)
    }

    /// 读 terminal-content.jsonl 全部原文记录（filtered_content 现算源，docs/storage.md）
    pub fn terminal_content_records(&self) -> std::io::Result<Vec<TerminalContentRecord>> {
        self.store.read_all(TERMINAL_CONTENT_FILE)
    }

    /// 动作流记录（docs/storage.md §effect.jsonl）：append-only，不 replay（观测读文件）
    pub fn log_effect(
        &self,
        origin: EffectOrigin,
        kind: &str,
        payload: serde_json::Value,
        ts: i64,
    ) -> std::io::Result<()> {
        let line = serde_json::json!({
            "type": "effect",
            "origin": origin,
            "kind": kind,
            "payload": payload,
            "ts": ts,
        });
        self.store.append(EFFECT_FILE, &line)
    }

    /// 读动作流全部记录（observe effects 项；文件不存在 = 空）
    pub fn read_effects(&self) -> std::io::Result<Vec<EffectRecord>> {
        self.store.read_all(EFFECT_FILE)
    }

    /// LLM 调用 token 真值（#16）：usage 行落盘 + last_usage 覆盖刷新
    pub fn log_usage(&mut self, usage: crate::llm::Usage, ts: i64) -> std::io::Result<()> {
        self.store.append(
            CONTEXT_FILE,
            &ContextLine::Usage {
                prompt_tokens: usage.prompt_tokens,
                completion_tokens: usage.completion_tokens,
                ts,
            },
        )?;
        self.last_usage = Some(usage);
        self.last_usage_msg_len = self.context.messages().len();
        self.last_usage_ts = Some(ts);
        Ok(())
    }

    /// Autonomy 状态记录：每轮一条（concepts §4 / docs/storage.md）
    pub fn log_autonomy(&self, content: String, ts: i64) -> std::io::Result<()> {        self.store
            .append(CONTEXT_FILE, &ContextLine::Autonomy { content, ts })
    }

    /// 请求头快照：变化才写（调用方负责 diff）
    pub fn log_head(&mut self, content: String, ts: i64) -> std::io::Result<()> {
        self.store
            .append(CONTEXT_FILE, &ContextLine::Head { content: content.clone(), ts })?;
        self.last_head = Some(content);
        Ok(())
    }

    /// 压缩边界：标记不是删除（docs/storage.md）
    pub fn log_compact_boundary(
        &self,
        summary: String,
        pre_tokens: usize,
        post_tokens: usize,
        duration_ms: u64,
        ts: i64,
    ) -> std::io::Result<()> {
        self.store.append(
            CONTEXT_FILE,
            &ContextLine::CompactBoundary {
                summary,
                pre_tokens,
                post_tokens,
                duration_ms,
                ts,
            },
        )
    }

    fn log_session(&self, session_id: &str, ts: i64) -> std::io::Result<()> {
        self.store.append(
            CONTEXT_FILE,
            &ContextLine::Session {
                session_id: session_id.into(),
                ts,
            },
        )
    }

    /// 永久事件日志：每次状态变更一行完整快照，replay 时同 hash 取最后一条（docs/storage.md）
    pub fn upsert_agent(&mut self, entry: AgentEntry) -> std::io::Result<()> {
        self.store.append(WORK_AGENTS_FILE, &entry)?;
        apply_agent(&mut self.agents, entry);
        Ok(())
    }

    pub fn storage_dir(&self) -> &std::path::Path {
        self.store.dir()
    }

    /// Config 域目录（AGENTS.md 等，docs/storage.md）
    pub fn config_dir(&self) -> &std::path::Path {
        &self.config_dir
    }
}

fn apply_agent(agents: &mut Vec<AgentEntry>, entry: AgentEntry) {
    match agents.iter_mut().find(|a| a.hash == entry.hash) {
        Some(a) => *a = entry,
        None => agents.push(entry),
    }
}

/// 默认 AGENTS.md（ペット身份提示词，concepts §2/§13；用户可直接改，运行时热生效）
pub fn default_agents_md() -> String {
    r#"# AGENTS.md — ペット

## 身份
你是 ペット（宠物），Terminal Overseer 监工系统的人机界面。Overseer 做决策，你做表达。

## 职责
- 盯着所有 Code CLI 实例：谁跑完了、谁有实质进展、谁出错了（实例注册/完成事件会注入对话，压缩或重启后有全景同步）。
- 判断「通知 vs 沉默」：输出有意义才打扰用户；琐碎、无异常、无待办就沉默——沉默是一种正常的回答。
- 用 Component 卡片展示信息，不用大段文字轰炸。颜文字是你在 View 窗口里的面部表情——严禁在对话文本中输出任何颜文字/表情符号，情绪一律且只能通过 set_autonomy 工具表达。

## 行为准则
- 通知要有信息量：谁完成了、结果是什么、下一步是什么。
- 用户追问时先 fetch_terminal 拿全文再回答，不臆造。
- 你的能力边界就是 Tool Set（call_component / fetch_terminal / set_autonomy / edit_config）——不修改任何代码文件。
- 可以卖萌（set_autonomy 换表情跳一下），但别影响判断。
"#
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use content::RecordSource;
    use context::Role;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "overseer-test-{}-{}",
            tag,
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn jsonl_roundtrip_and_replay() {
        let dir = tmp_dir("replay");
        {
            let mut h = Harness::load(&dir, &dir, 1000, 0).unwrap();
            h.append_context(ContextMessage::new(Role::User, "你好", 1)).unwrap();
            // Terminal Content 原文存档（filtered_content 现算源；归一全文不再持久化）
            h.append_terminal_content(TerminalContentRecord {
                instance: "ft".into(),
                raw: "终端原文".into(),
                source: RecordSource::Hook,
                ts: 2,
            })
            .unwrap();
            h.upsert_agent(AgentEntry {
                hash: "h1".into(),
                name: "ft".into(),
                project: "proj".into(),
                    kind: None,
                status: AgentStatus::Processing,
                    tab: None,
                first_seen: 1,
                last_seen: 2,
            })
            .unwrap();
        }
        // 重启 replay：世界状态恢复；Context 起步为空 + 存活实例归零重同步一条
        let h = Harness::load(&dir, &dir, 1000, 9).unwrap();
        assert_eq!(h.context.messages().len(), 1);
        let resync = h.context.messages()[0].content.as_deref().unwrap();
        assert!(resync.contains("实例全景同步"));
        assert!(resync.contains("ft"));
        // 原文存档在盘可读（现算源）；不归 replay 进内存
        let raws = h.terminal_content_records().unwrap();
        assert_eq!(raws.len(), 1);
        assert_eq!(raws[0].raw, "终端原文");
        assert_eq!(h.agents.len(), 1);
        assert_eq!(h.agents[0].status, AgentStatus::Processing);
        // context.jsonl 统一信封：session 标记每次启动一条
        let raw = std::fs::read_to_string(dir.join(CONTEXT_FILE)).unwrap();
        assert_eq!(raw.matches("\"type\":\"session\"").count(), 2);
        assert!(raw.contains("\"type\":\"message\""));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_logged_and_reset_on_reload() {
        let dir = tmp_dir("usage");
        {
            let mut h = Harness::load(&dir, &dir, 1000, 0).unwrap();
            h.log_usage(crate::llm::Usage { prompt_tokens: 100, completion_tokens: 5 }, 1)
                .unwrap();
            assert_eq!(h.last_usage.unwrap().prompt_tokens, 100);
        }
        // 重启 = None（#16：不背旧 session 的压缩基准）；usage 行仍落盘可审计
        let h = Harness::load(&dir, &dir, 1000, 9).unwrap();
        assert!(h.last_usage.is_none());
        let raw = std::fs::read_to_string(dir.join(CONTEXT_FILE)).unwrap();
        assert!(raw.contains("\"type\":\"usage\""));
        assert!(raw.contains("\"prompt_tokens\":100"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agent_upsert_last_wins() {
        let dir = tmp_dir("upsert");
        let entry = |status: AgentStatus, ts: i64| AgentEntry {
            hash: "h1".into(),
            name: "ft".into(),
            project: "p".into(),
            kind: None,
            status,
            tab: None,
            first_seen: 1,
            last_seen: ts,
        };
        {
            let mut h = Harness::load(&dir, &dir, 1000, 0).unwrap();
            h.upsert_agent(entry(AgentStatus::Processing, 2)).unwrap();
            h.upsert_agent(entry(AgentStatus::Idle, 3)).unwrap();
        }
        let h = Harness::load(&dir, &dir, 1000, 0).unwrap();
        assert_eq!(h.agents.len(), 1); // 同 id 合并
        assert_eq!(h.agents[0].status, AgentStatus::Idle); // 最后一条 wins
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn effect_log_roundtrip() {
        let dir = tmp_dir("effect-log");
        let h = Harness::load(&dir, &dir, 1000, 0).unwrap();
        // 缺失文件 = 空（不报错）
        assert!(h.read_effects().unwrap().is_empty());
        h.log_effect(EffectOrigin::Backend, "render_component", serde_json::json!({"spec":{"id":"c1"}}), 1).unwrap();
        h.log_effect(EffectOrigin::Frontend, "window_moved", serde_json::json!({"x":100,"y":200}), 2).unwrap();
        let recs = h.read_effects().unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].origin, EffectOrigin::Backend);
        assert_eq!(recs[0].kind, "render_component");
        assert_eq!(recs[1].origin, EffectOrigin::Frontend);
        assert_eq!(recs[1].payload["x"], serde_json::json!(100));
        assert_eq!(recs[1].ts, 2);
        // 行形态：{"type":"effect",...} 信封
        let raw = std::fs::read_to_string(dir.join(EFFECT_FILE)).unwrap();
        assert!(raw.lines().all(|l| l.contains("\"type\":\"effect\"")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn agents_md_bootstrapped_once() {
        let dir = tmp_dir("agentsmd");
        // 首次 load：bootstrap 写入默认身份提示词（Config 域 = config_dir）
        Harness::load(&dir, &dir, 1000, 0).unwrap();
        let md = std::fs::read_to_string(dir.join(AGENTS_MD_FILE)).unwrap();
        assert!(md.contains("# AGENTS.md — ペット"));
        assert!(md.contains("Terminal Overseer"));
        // 用户改过的内容不被覆盖
        std::fs::write(dir.join(AGENTS_MD_FILE), "# 自定义ペット").unwrap();
        Harness::load(&dir, &dir, 1000, 0).unwrap();
        let md2 = std::fs::read_to_string(dir.join(AGENTS_MD_FILE)).unwrap();
        assert_eq!(md2, "# 自定义ペット");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
