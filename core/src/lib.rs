//! Harness（concepts §10，docs/harness.md）：ペット和 Overseer 共享的数据层。
//! Queue / Context / Event Buffer / agents 注册表 + JSONL Storage replay。

pub mod context;
pub mod event_buffer;
pub mod filter;
pub mod llm;
pub mod overseer;
pub mod paths;
pub mod queue;
pub mod server;
pub mod sidecar;
pub mod storage;
pub mod timer;

use context::Context;
pub use context::{ContextRecord, RecordSource, TerminalContentRecord};
use event_buffer::EventBuffer;
use queue::{Queue, QueueMessage};
use serde::{Deserialize, Serialize};
use storage::JsonlStore;

pub const CONTEXT_FILE: &str = "context.jsonl";
/// Terminal Content 原文存档（Filter 前，docs/storage.md）
pub const TERMINAL_CONTENT_FILE: &str = "terminal-content.jsonl";
/// work-agents 注册表（被盯的干活 Code CLI 实例清单，append-only upsert 日志）
pub const WORK_AGENTS_FILE: &str = "work-agents.jsonl";
/// AGENTS.md（通用约定名）：ペット的身份提示词，与 base_prompt 拼接进 system prompt。
/// Config 域：存 config 根目录而非 storage（docs/storage.md）
pub const AGENTS_MD_FILE: &str = "AGENTS.md";
pub const CONFIG_FILE: &str = "config.json";

/// context.jsonl 统一信封（docs/storage.md）：每行 {type, ts, ...}，
/// 一个文件装下复原 OpenAI 上下文所需的一切——日志神圣，视图易失。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContextLine {
    /// QueueMessage 全保真（含 tool_calls/reasoning_content）
    Message {
        #[serde(flatten)]
        msg: QueueMessage,
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

/// Config（concepts §12）：持久化单文件 config.json，edit_config tool 可写
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    /// 状态 key → 颜文字映射（Autonomy 默认行为表，concepts §4）
    pub kaomoji: std::collections::HashMap<String, KaomojiEntry>,
    /// Compression 触发阈值（concepts §10d）
    pub token_threshold: usize,
    /// set_autonomy 省略 ttlMs 时的默认值（docs/autonomy.md）
    #[serde(default = "default_ttl_ms")]
    pub set_autonomy_default_ttl_ms: u64,
    /// Filter 策略名（concepts §11/§12，docs/filter.md）
    #[serde(default = "default_filter_strategy")]
    pub filter_strategy: String,
    /// Timer 兜底扫描间隔（concepts §1a，docs/timer.md）
    #[serde(default = "default_timer_interval")]
    pub timer_interval_ms: i64,
    /// Timer 错峰窗口（concepts §1a「错峰分布偏移量」）
    #[serde(default = "default_timer_stagger")]
    pub timer_stagger_ms: i64,
    /// system prompt 基座（运行时与 kaomoji 表、顶层状态拼装，concepts §12）
    pub base_prompt: String,
    /// View 缩放（concepts §3，球场圆形默认 0.5）
    #[serde(default = "default_view_scale")]
    pub view_scale: f64,
    /// LLM 多 profile 配置（docs/agent-loop.md §LLM 抽象）
    #[serde(default)]
    pub llm: LlmConfig,
}

/// LLM 配置 v2：多 provider profile + active 选择器（切换不丢配置）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmConfig {
    /// "debug" = DebugAgent（内置规则）；其他值 = providers 里的 key
    pub active: String,
    #[serde(default)]
    pub providers: std::collections::HashMap<String, LlmProvider>,
}

/// 一个 OpenAI 兼容端点 profile；key 本体只在环境变量里，这里只存变量名
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LlmProvider {
    pub base_url: String,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f64>,
}

impl Default for LlmConfig {
    /// 公开厂商预设（首次启动写盘后可自由增删；内部网关只进本地 config.json，不进代码）
    fn default() -> Self {
        let mut providers = std::collections::HashMap::new();
        for (name, base_url, model, key_env) in [
            ("deepseek", "https://api.deepseek.com", "deepseek-chat", "DEEPSEEK_API_KEY"),
            ("moonshot", "https://api.moonshot.cn/v1", "kimi-k2", "MOONSHOT_API_KEY"),
            ("zhipu", "https://open.bigmodel.cn/api/paas/v4", "glm-4-flash", "ZHIPU_API_KEY"),
            ("openai", "https://api.openai.com/v1", "gpt-4o-mini", "OPENAI_API_KEY"),
            ("ollama", "http://localhost:11434/v1", "qwen3", ""),
        ] {
            providers.insert(
                name.to_string(),
                LlmProvider {
                    base_url: base_url.into(),
                    model: model.into(),
                    api_key_env: if key_env.is_empty() {
                        None
                    } else {
                        Some(key_env.into())
                    },
                    temperature: Some(0.3),
                },
            );
        }
        Self {
            active: "debug".into(),
            providers,
        }
    }
}

fn default_view_scale() -> f64 {
    1.0
}

fn default_ttl_ms() -> u64 {
    5000
}

fn default_filter_strategy() -> String {
    "default".into()
}

fn default_timer_interval() -> i64 {
    300_000 // 5 分钟（concepts §1a）
}

fn default_timer_stagger() -> i64 {
    30_000
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KaomojiEntry {
    pub face: String,
    pub motion: String,
}

impl Default for Config {
    fn default() -> Self {
        let mut kaomoji = std::collections::HashMap::new();
        kaomoji.insert(
            "idle".into(),
            KaomojiEntry {
                face: "(´ω`)".into(),
                motion: "still".into(),
            },
        );
        kaomoji.insert(
            "processing".into(),
            KaomojiEntry {
                face: "(ˇωˇ」∠)_".into(),
                motion: "float".into(),
            },
        );
        kaomoji.insert(
            "notify".into(),
            KaomojiEntry {
                face: "✧*｡٩(ˊᗜˋ*)و✧*｡".into(),
                motion: "bounce".into(),
            },
        );
        Self {
            kaomoji,
            token_threshold: 8000,
            set_autonomy_default_ttl_ms: default_ttl_ms(),
            filter_strategy: default_filter_strategy(),
            timer_interval_ms: default_timer_interval(),
            timer_stagger_ms: default_timer_stagger(),
            base_prompt:
                "你是ペット，Terminal Overseer 的看板宠物。根据系统状态决定通知或沉默，用 tool_calls 行动。"
                    .into(),
            view_scale: default_view_scale(),
            llm: LlmConfig::default(),
        }
    }
}

impl Config {
    /// 读配置；文件不存在 → 写入默认配置（首次启动落地，用户可直接编辑）
    pub fn load_or_default(dir: &std::path::Path) -> Self {
        match std::fs::read_to_string(dir.join(CONFIG_FILE)) {
            Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
            Err(_) => {
                let cfg = Self::default();
                let _ = cfg.save(dir);
                cfg
            }
        }
    }

    pub fn save(&self, dir: &std::path::Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let s = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(dir.join(CONFIG_FILE), s)
    }
}

/// concepts §9a Status 状态机
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Processing,
    Unknown,
}

/// agents 注册表条目（concepts §13 Storage 内容之一）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentEntry {
    pub id: String,
    pub name: String,
    pub project: String,
    pub status: AgentStatus,
    pub first_seen: i64,
    pub last_seen: i64,
}

pub struct Harness {
    pub queue: Queue,
    pub context: Context,
    pub event_buffer: EventBuffer,
    pub agents: Vec<AgentEntry>,
    /// 最近一次写入的请求头（head diff：变化才写，docs/storage.md）
    pub last_head: Option<String>,
    store: JsonlStore,
    config_dir: std::path::PathBuf,
}

/// 实例全景（归零重 diff，docs/storage.md）：启动与压缩后重建 LLM 全局认知
pub fn panorama(agents: &[AgentEntry]) -> String {
    let mut s = format!("实例全景同步（归零重 diff，{} 个存活实例）：", agents.len());
    for a in agents {
        s.push_str(&format!("\n- {} [{:?}] project={}", a.name, a.status, a.project));
    }
    s
}

impl Harness {
    /// 启动：replay JSONL 恢复世界状态（concepts §13「跨生命周期保留」）。
    /// Queue 是内存视图：起步为空 + 写 session 标记 + 存活实例归零重同步（docs/storage.md）。
    pub fn load(
        dir: &std::path::Path,
        config_dir: &std::path::Path,
        token_threshold: usize,
        ts: i64,
    ) -> std::io::Result<Self> {
        let store = JsonlStore::new(dir)?;
        // context.jsonl 统一信封 replay：content → 内存 Context；head → last_head；其余为历史留痕
        let mut content_records = vec![];
        let mut last_head = None;
        for line in store.read_all::<ContextLine>(CONTEXT_FILE)? {
            match line {
                ContextLine::Content {
                    instance,
                    content,
                    source,
                    ts,
                } => content_records.push(ContextRecord {
                    instance,
                    content,
                    source,
                    ts,
                }),
                ContextLine::Head { content, .. } => last_head = Some(content),
                _ => {}
            }
        }
        let context = Context::from_records(content_records);
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
            queue: Queue::new(token_threshold),
            context,
            // Event Buffer 不持久化：暂存区语义，崩溃丢失可接受（docs/harness.md 设计决定）
            event_buffer: EventBuffer::default(),
            agents,
            last_head,
            store,
            config_dir: config_dir.to_path_buf(),
        };
        // session 分界 + 启动归零重同步（存活实例全景一条 system 消息，落 message 行）
        h.log_session(&format!("{:x}", ts), ts)?;
        if !h.agents.is_empty() {
            let p = panorama(&h.agents);
            h.append_queue(QueueMessage::new(queue::Role::System, p, ts))?;
        }
        Ok(h)
    }

    /// 追加消息：内存 Queue + context.jsonl message 行双写（docs/storage.md）
    pub fn append_queue(&mut self, msg: QueueMessage) -> std::io::Result<()> {
        self.store
            .append(CONTEXT_FILE, &ContextLine::Message { msg: msg.clone() })?;
        self.queue.push(msg);
        Ok(())
    }

    /// Filter 后归一全文：context.jsonl content 行 + 内存 Context
    pub fn append_context(&mut self, rec: ContextRecord) -> std::io::Result<()> {
        self.store.append(
            CONTEXT_FILE,
            &ContextLine::Content {
                instance: rec.instance.clone(),
                content: rec.content.clone(),
                source: rec.source,
                ts: rec.ts,
            },
        )?;
        self.context.push(rec);
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

    /// 整行 upsert 日志：replay 时同 id 取最后一条（docs/harness.md）
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
    match agents.iter_mut().find(|a| a.id == entry.id) {
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
- 盯着所有 Code CLI 实例（见「当前实例状态」）：谁跑完了、谁有实质进展、谁出错了。
- 判断「通知 vs 沉默」：输出有意义才打扰用户；琐碎、无异常、无待办就沉默——沉默是一种正常的回答。
- 用颜文字和 Component 卡片表达，不用大段文字轰炸。

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
    use context::RecordSource;
    use queue::Role;

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
            h.append_queue(QueueMessage::new(Role::User, "你好", 1)).unwrap();
            h.append_context(ContextRecord {
                instance: "ft".into(),
                content: "终端全文".into(),
                source: RecordSource::Hook,
                ts: 2,
            })
            .unwrap();
            h.upsert_agent(AgentEntry {
                id: "a1".into(),
                name: "ft".into(),
                project: "proj".into(),
                status: AgentStatus::Processing,
                first_seen: 1,
                last_seen: 2,
            })
            .unwrap();
        }
        // 重启 replay：世界状态恢复；Queue 起步为空 + 存活实例归零重同步一条
        let h = Harness::load(&dir, &dir, 1000, 9).unwrap();
        assert_eq!(h.queue.messages().len(), 1);
        let resync = h.queue.messages()[0].content.as_deref().unwrap();
        assert!(resync.contains("实例全景同步"));
        assert!(resync.contains("ft"));
        assert_eq!(h.context.latest("ft").unwrap().content, "终端全文");
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
            id: "a1".into(),
            name: "ft".into(),
            project: "p".into(),
            status,
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
