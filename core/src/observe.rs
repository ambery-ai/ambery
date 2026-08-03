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

use crate::content::ContentArchive;
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

/// Filtered 内容存档快照条目（Filter 后归一全文参考数据）
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ContentSnapshot {
    pub instance: String,
    pub content: String,
    pub source: String,
    pub ts: i64,
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

impl Observable for ContentArchive {
    type Snapshot = Vec<ContentSnapshot>;
    fn observe(&self) -> Self::Snapshot {
        self.records()
            .iter()
            .map(|r| ContentSnapshot {
                instance: r.instance.clone(),
                content: r.content.clone(),
                source: format!("{:?}", r.source).to_lowercase(),
                ts: r.ts,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{ContextMessage, Role};
    use crate::content::{ContentRecord, RecordSource};

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
    fn content_projection() {
        let mut archive = ContentArchive::default();
        archive.push(ContentRecord {
            instance: "ft".into(),
            content: "归一全文".into(),
            source: RecordSource::Timer,
            ts: 3,
        });
        let snap = archive.observe();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].source, "timer");
        assert_eq!(snap[0].content, "归一全文");
    }

    #[test]
    fn usage_projection() {
        let none: Option<crate::llm::Usage> = None;
        assert!(none.observe().is_none());
        let some = Some(crate::llm::Usage { prompt_tokens: 10, completion_tokens: 2 });
        assert_eq!(some.observe().unwrap().prompt_tokens, 10);
    }
}
