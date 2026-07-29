//! Harness（concepts §10，docs/harness.md）：ペット和 OverseerBackend 共享的数据层。
//! Queue（输入排队）/ Context（消息数组）/ Content 存档 / Event Buffer / agents 注册表
//! + JSONL Storage replay。

pub mod config;
pub mod content;
pub mod context;
pub mod event_buffer;
pub mod filter;
pub mod llm;
#[cfg(feature = "mock")]
pub mod mock;
#[cfg(feature = "case-runner")]
pub mod case;
pub mod overseer;
pub mod paths;
pub mod queue;
pub mod server;
pub mod sidecar;
pub mod storage;
pub mod timer;

pub use config::{Config, KaomojiEntry, LlmConfig, LlmProvider, CONFIG_FILE};
use content::ContentArchive;
pub use content::{ContentRecord, RecordSource, TerminalContentRecord};
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
    /// Filter 后归一全文（fetch 回退/追问/变化检测基准）
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

pub struct Harness {
    /// Queue（concepts §10c）：输入串行化关口，只装待放行输入
    pub queue: Queue,
    /// Context（concepts §10b）：完整消息数组，LLM 请求的上下文源
    pub context: Context,
    /// Content 存档（concepts §8/§11）：Filter 后归一全文参考数据
    pub content: ContentArchive,
    pub event_buffer: EventBuffer,
    pub agents: Vec<AgentEntry>,
    /// 最近一次写入的请求头（head diff：变化才写，docs/storage.md）
    pub last_head: Option<String>,
    store: JsonlStore,
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
        // context.jsonl 统一信封 replay：content → 内存 ContentArchive；head → last_head；其余为历史留痕
        let mut content_records = vec![];
        let mut last_head = None;
        for line in store.read_all::<ContextLine>(CONTEXT_FILE)? {
            match line {
                ContextLine::Content {
                    instance,
                    content,
                    source,
                    ts,
                } => content_records.push(ContentRecord {
                    instance,
                    content,
                    source,
                    ts,
                }),
                ContextLine::Head { content, .. } => last_head = Some(content),
                _ => {}
            }
        }
        let content = ContentArchive::from_records(content_records);
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
        let mut h = Self {
            // Queue 不 replay：崩溃丢失未放行输入可接受（docs/storage.md 设计决定）
            queue: Queue::default(),
            context: Context::new(token_threshold),
            content,
            // Event Buffer 不持久化：暂存区语义，崩溃丢失可接受（docs/harness.md 设计决定）
            event_buffer: EventBuffer::default(),
            agents,
            last_head,
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

    /// Filter 后归一全文：context.jsonl content 行 + 内存 ContentArchive
    pub fn append_content(&mut self, rec: ContentRecord) -> std::io::Result<()> {
        self.store.append(
            CONTEXT_FILE,
            &ContextLine::Content {
                instance: rec.instance.clone(),
                content: rec.content.clone(),
                source: rec.source,
                ts: rec.ts,
            },
        )?;
        self.content.push(rec);
        Ok(())
    }

    /// Terminal Content 原文存档（Filter 前；平时不读、启动不 replay）
    pub fn append_terminal_content(&self, rec: TerminalContentRecord) -> std::io::Result<()> {
        self.store.append(TERMINAL_CONTENT_FILE, &rec)
    }

    /// Autonomy 状态记录：每轮一条（concepts §4 / docs/storage.md）
    pub fn log_autonomy(&self, content: String, ts: i64) -> std::io::Result<()> {
        self.store
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
            h.append_content(ContentRecord {
                instance: "ft".into(),
                content: "终端全文".into(),
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
        assert_eq!(h.content.latest("ft").unwrap().content, "终端全文");
        assert_eq!(h.agents.len(), 1);
        assert_eq!(h.agents[0].status, AgentStatus::Processing);
        // context.jsonl 统一信封：session 标记每次启动一条
        let raw = std::fs::read_to_string(dir.join(CONTEXT_FILE)).unwrap();
        assert_eq!(raw.matches("\"type\":\"session\"").count(), 2);
        assert!(raw.contains("\"type\":\"message\""));
        assert!(raw.contains("\"type\":\"content\""));
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
