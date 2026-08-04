//! 可观测性基座（docs/observability.md）：trait Observable（模块投影）+
//! derive Observe（聚合覆盖断言，proc-macro 作 case-runner feature 可选依赖）。
//! feature "case-runner" gate。
//!
//! ## 验收：聚合体含未实现 Observable 的字段 → E0277
//!
//! ```compile_fail,E0277
//! #[derive(overseer_observe_derive::Observe)]
//! struct H { queue: String }
//! ```

use crate::context::Context;
use crate::event_buffer::EventBuffer;
use crate::queue::{Queue, QueueInput};
use crate::{AgentEntry, AgentStatus};

/// 可观测模块：投影当前快照（值语义，observe step 直接消费）
pub trait Observable {
    /// 快照投影类型
    type Snapshot;
    fn observe(&self) -> Self::Snapshot;
}

/// agents 注册表快照条目
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AgentSnapshot {
    pub hash: String,
    pub name: String,
    pub project: String,
    pub status: AgentStatus,
    pub last_seen: i64,
}

/// Context 消息快照条目（role / content / tool_calls 计数 / ts）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MessageSnapshot {
    pub role: String,
    pub content: Option<String>,
    pub tool_calls: usize,
    pub ts: i64,
}

/// Filtered 内容快照条目（归一全文现算，docs/storage.md §filtered_content 退役）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FilteredContentSnapshot {
    pub instance: String,
    pub filtered_content: String,
    pub source: String,
    pub ts: i64,
}

/// Memory note 摘要条目（index 摘要：name / description；不展开正文，docs/observability.md）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoryNoteSnapshot {
    pub name: String,
    pub description: String,
}

/// Cron 计划投影条目（id / schedule / message / next_due；不含 sleep waiter）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CronSnapshot {
    pub id: String,
    pub schedule: crate::cron::Schedule,
    pub message: String,
    /// None = 完成态（at 已发放），不再调度
    pub next_due: Option<i64>,
}

impl Observable for Queue {
    type Snapshot = Vec<QueueInput>;
    fn observe(&self) -> Self::Snapshot {
        self.iter().cloned().collect()
    }
}

impl Observable for Context {
    type Snapshot = Vec<MessageSnapshot>;
    fn observe(&self) -> Self::Snapshot {
        self.messages()
            .iter()
            .map(|m| MessageSnapshot {
                role: format!("{:?}", m.role).to_lowercase(),
                content: m.content.clone(),
                tool_calls: m.tool_calls.as_ref().map_or(0, |c| c.len()),
                ts: m.ts,
            })
            .collect()
    }
}

impl Observable for EventBuffer {
    type Snapshot = Vec<String>;
    fn observe(&self) -> Self::Snapshot {
        self.events().to_vec()
    }
}

impl Observable for Vec<AgentEntry> {
    type Snapshot = Vec<AgentSnapshot>;
    fn observe(&self) -> Self::Snapshot {
        self.iter()
            .map(|a| AgentSnapshot {
                hash: a.hash.clone(),
                name: a.name.clone(),
                project: a.project.clone(),
                status: a.status.clone(),
                last_seen: a.last_seen,
            })
            .collect()
    }
}

impl Observable for Option<crate::llm::Usage> {
    type Snapshot = Option<crate::llm::Usage>;
    fn observe(&self) -> Self::Snapshot {
        *self
    }
}

impl Observable for crate::memory::Memory {
    type Snapshot = Vec<MemoryNoteSnapshot>;
    fn observe(&self) -> Self::Snapshot {
        self.list_notes()
            .into_iter()
            .map(|(name, description)| MemoryNoteSnapshot { name, description })
            .collect()
    }
}

impl Observable for crate::cron::CronScheduler {
    type Snapshot = Vec<CronSnapshot>;
    fn observe(&self) -> Self::Snapshot {
        self.entries()
            .iter()
            .map(|e| CronSnapshot {
                id: e.id.clone(),
                schedule: e.schedule,
                message: e.message.clone(),
                next_due: e.next_due,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ContextMessage, Role};

    #[test]
    fn context_projection() {
        let mut ctx = Context::new(1000);
        ctx.push(ContextMessage::new(Role::User, "hi", 1));
        ctx.push(ContextMessage::new(Role::Assistant, "hello", 2));
        let snap = ctx.observe();
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].role, "user");
        assert_eq!(snap[0].content.as_deref(), Some("hi"));
        assert_eq!(snap[1].ts, 2);
    }

    #[test]
    fn agents_projection() {
        let agents = vec![AgentEntry {
            hash: "h1".into(),
            name: "ft".into(),
            project: "p".into(),
            kind: None,
            status: AgentStatus::Processing,
            tab: None,
            first_seen: 1,
            last_seen: 2,
        }];
        let snap = agents.observe();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].hash, "h1");
        assert_eq!(snap[0].status, AgentStatus::Processing);
    }

    #[test]
    fn usage_projection() {
        let none: Option<crate::llm::Usage> = None;
        assert!(none.observe().is_none());
        let some = Some(crate::llm::Usage { prompt_tokens: 10, completion_tokens: 2 });
        assert_eq!(some.observe().unwrap().prompt_tokens, 10);
    }

    #[test]
    fn memory_projection() {
        let dir = std::env::temp_dir().join(format!("overseer-obs-mem-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let m = crate::memory::Memory::bootstrap(&dir).unwrap();
        assert!(m.observe().is_empty());
        m.write("work-preferences", "正文", "用户的工作偏好").unwrap();
        // frontmatter 不合法的外部文件不进摘要
        std::fs::write(m.notes_dir().join("no-fm.md"), "无 frontmatter").unwrap();
        let snap = m.observe();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].name, "work-preferences");
        assert_eq!(snap[0].description, "用户的工作偏好");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cron_projection() {
        let dir = std::env::temp_dir().join(format!("overseer-obs-cron-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut c = crate::cron::CronScheduler::load(&dir).unwrap();
        let id = c.create(crate::cron::Schedule::EveryMs(60_000), "日报", 1000).unwrap();
        let id2 = c.create(crate::cron::Schedule::At(70_000), "一次性", 2000).unwrap();
        // at 发放后完成态：next_due = None 也进投影；every_ms 同刻到期重排
        let _ = c.due(70_000).unwrap();
        let snap = c.observe();
        assert_eq!(snap.len(), 2);
        let e = snap.iter().find(|e| e.id == id).unwrap();
        assert_eq!(e.schedule, crate::cron::Schedule::EveryMs(60_000));
        assert_eq!(e.message, "日报");
        assert_eq!(e.next_due, Some(121_000));
        assert_eq!(snap.iter().find(|e| e.id == id2).unwrap().next_due, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
