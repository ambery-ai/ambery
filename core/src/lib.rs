//! Harness（concepts §10，docs/harness.md）：ペット和 Overseer 共享的数据层。
//! Queue / Context / Event Buffer / agents 注册表 + JSONL Storage replay。

pub mod context;
pub mod event_buffer;
pub mod queue;
pub mod storage;

use context::{Context, ContextRecord};
use event_buffer::EventBuffer;
use queue::{Queue, QueueMessage};
use serde::{Deserialize, Serialize};
use storage::JsonlStore;

pub const QUEUE_FILE: &str = "queue.jsonl";
pub const CONTEXT_FILE: &str = "context.jsonl";
pub const AGENTS_FILE: &str = "agents.jsonl";

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
    store: JsonlStore,
}

impl Harness {
    /// 启动：replay JSONL 恢复完整状态（concepts §13「重启后恢复完整对话」）。
    /// queue.jsonl 为空时用给定 prefix 新建。
    pub fn load(
        dir: &std::path::Path,
        prefix: String,
        token_threshold: usize,
        ts: i64,
    ) -> std::io::Result<Self> {
        let store = JsonlStore::new(dir)?;
        let queue_msgs: Vec<QueueMessage> = store.read_all(QUEUE_FILE)?;
        let queue = if queue_msgs.is_empty() {
            // 新建 Queue 时 prefix 一并落盘，保证 replay 首条恒为 system prefix
            let prefix_msg = QueueMessage::new(queue::Role::System, prefix, ts);
            store.append(QUEUE_FILE, &prefix_msg)?;
            Queue::from_messages(vec![prefix_msg], token_threshold)
        } else {
            Queue::from_messages(queue_msgs, token_threshold)
        };
        let context = Context::from_records(store.read_all(CONTEXT_FILE)?);
        // agents 是 upsert 日志：replay 须逐条折叠（同 id 取最后一条）
        let mut agents: Vec<AgentEntry> = vec![];
        for entry in store.read_all::<AgentEntry>(AGENTS_FILE)? {
            apply_agent(&mut agents, entry);
        }
        Ok(Self {
            queue,
            context,
            // Event Buffer 不持久化：暂存区语义，崩溃丢失可接受（docs/harness.md 设计决定）
            event_buffer: EventBuffer::default(),
            agents,
            store,
        })
    }

    pub fn append_queue(&mut self, msg: QueueMessage) -> std::io::Result<()> {
        self.store.append(QUEUE_FILE, &msg)?;
        self.queue.push(msg);
        Ok(())
    }

    pub fn append_context(&mut self, rec: ContextRecord) -> std::io::Result<()> {
        self.store.append(CONTEXT_FILE, &rec)?;
        self.context.push(rec);
        Ok(())
    }

    /// 整行 upsert 日志：replay 时同 id 取最后一条（docs/harness.md）
    pub fn upsert_agent(&mut self, entry: AgentEntry) -> std::io::Result<()> {
        self.store.append(AGENTS_FILE, &entry)?;
        apply_agent(&mut self.agents, entry);
        Ok(())
    }
}

fn apply_agent(agents: &mut Vec<AgentEntry>, entry: AgentEntry) {
    match agents.iter_mut().find(|a| a.id == entry.id) {
        Some(a) => *a = entry,
        None => agents.push(entry),
    }
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
            let mut h = Harness::load(&dir, "PREFIX".into(), 1000, 0).unwrap();
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
        // 重启 replay：完整恢复
        let h = Harness::load(&dir, "PREFIX".into(), 1000, 0).unwrap();
        assert_eq!(h.queue.messages().len(), 2); // prefix + user
        assert_eq!(h.queue.prefix().content.as_deref(), Some("PREFIX"));
        assert_eq!(h.context.latest("ft").unwrap().content, "终端全文");
        assert_eq!(h.agents.len(), 1);
        assert_eq!(h.agents[0].status, AgentStatus::Processing);
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
            let mut h = Harness::load(&dir, "P".into(), 1000, 0).unwrap();
            h.upsert_agent(entry(AgentStatus::Processing, 2)).unwrap();
            h.upsert_agent(entry(AgentStatus::Idle, 3)).unwrap();
        }
        let h = Harness::load(&dir, "P".into(), 1000, 0).unwrap();
        assert_eq!(h.agents.len(), 1); // 同 id 合并
        assert_eq!(h.agents[0].status, AgentStatus::Idle); // 最后一条 wins
        let _ = std::fs::remove_dir_all(&dir);
    }
}
