//! Queue（concepts §10c，docs/harness.md）：消息 thread，顺序处理，
//! 第 0 条恒为 system prefix（替换式更新，不追加、不胀 Queue）。

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

/// OpenAI 风格 tool_call：arguments 为 JSON 字符串
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueueMessage {
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub ts: i64,
}

impl QueueMessage {
    pub fn new(role: Role, content: impl Into<String>, ts: i64) -> Self {
        Self {
            role,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            ts,
        }
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>, ts: i64) -> Self {
        Self {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(calls),
            tool_call_id: None,
            ts,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>, ts: i64) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            ts,
        }
    }

    /// token 估算（chars/4 + 每条固定开销），真实分词留到接入真实 API
    fn est_tokens(&self) -> usize {
        let mut chars = self.content.as_ref().map_or(0, |c| c.chars().count());
        if let Some(calls) = &self.tool_calls {
            for c in calls {
                chars += c.name.len() + c.arguments.len();
            }
        }
        chars / 4 + 4
    }
}

/// shaking 保留最近 N 条原始消息（prefix 除外），concepts §10d
pub const KEEP_RECENT: usize = 8;

pub struct Queue {
    messages: Vec<QueueMessage>, // [0] 恒为 system prefix
    pub token_threshold: usize,
}

impl Queue {
    pub fn new(prefix: String, ts: i64, token_threshold: usize) -> Self {
        Self {
            messages: vec![QueueMessage::new(Role::System, prefix, ts)],
            token_threshold,
        }
    }

    pub fn messages(&self) -> &[QueueMessage] {
        &self.messages
    }

    /// 替换式更新 system prefix（Autonomy 顶层状态等），cache 锚点稳定
    pub fn replace_prefix(&mut self, prefix: String, ts: i64) {
        self.messages[0] = QueueMessage::new(Role::System, prefix, ts);
    }

    pub fn prefix(&self) -> &QueueMessage {
        &self.messages[0]
    }

    pub fn push(&mut self, msg: QueueMessage) {
        self.messages.push(msg);
    }

    pub fn total_tokens(&self) -> usize {
        self.messages.iter().map(QueueMessage::est_tokens).sum()
    }

    pub fn needs_compression(&self) -> bool {
        self.total_tokens() > self.token_threshold
    }

    /// 总结 + shaking：prefix 不动，历史压为一条 system 摘要，保留最近 KEEP_RECENT 条
    pub fn compress(&mut self, summary: String, ts: i64) {
        let prefix = self.messages[0].clone();
        let tail_start = self.messages.len().saturating_sub(KEEP_RECENT).max(1);
        let tail: Vec<QueueMessage> = self.messages[tail_start..].to_vec();
        self.messages = vec![
            prefix,
            QueueMessage::new(Role::System, format!("[历史摘要] {summary}"), ts),
        ];
        self.messages.extend(tail);
    }

    /// replay 恢复（Storage 读出的第一条必须已是 system prefix）
    pub fn from_messages(messages: Vec<QueueMessage>, token_threshold: usize) -> Self {
        assert!(
            !messages.is_empty() && messages[0].role == Role::System,
            "queue replay: first message must be system prefix"
        );
        Self {
            messages,
            token_threshold,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q() -> Queue {
        Queue::new("PREFIX v1".into(), 0, 100)
    }

    #[test]
    fn prefix_replace_does_not_grow() {
        let mut queue = q();
        queue.replace_prefix("PREFIX v2".into(), 1);
        queue.replace_prefix("PREFIX v3".into(), 2);
        assert_eq!(queue.messages().len(), 1);
        assert_eq!(queue.prefix().content.as_deref(), Some("PREFIX v3"));
        assert_eq!(queue.prefix().role, Role::System);
    }

    #[test]
    fn push_appends_in_order() {
        let mut queue = q();
        queue.push(QueueMessage::new(Role::User, "u1", 1));
        queue.push(QueueMessage::new(Role::Assistant, "a1", 2));
        let roles: Vec<Role> = queue.messages().iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::System, Role::User, Role::Assistant]);
    }

    #[test]
    fn compress_keeps_prefix_summary_and_recent() {
        let mut queue = q();
        for i in 0..20 {
            queue.push(QueueMessage::new(Role::User, format!("msg-{i}"), i as i64));
        }
        assert!(queue.needs_compression()); // 21 条 × (4 + ~1) > 100
        queue.compress("前文提要".into(), 99);
        let msgs = queue.messages();
        assert_eq!(msgs.len(), 2 + KEEP_RECENT);
        assert_eq!(msgs[0].content.as_deref(), Some("PREFIX v1")); // prefix 不动
        assert_eq!(
            msgs[1].content.as_deref(),
            Some("[历史摘要] 前文提要")
        );
        assert_eq!(msgs[2].content.as_deref(), Some("msg-12")); // 最近 8 条
        assert_eq!(msgs.last().unwrap().content.as_deref(), Some("msg-19"));
    }

    #[test]
    fn compress_short_queue_is_noop_like() {
        let mut queue = q();
        queue.push(QueueMessage::new(Role::User, "only", 1));
        queue.compress("s".into(), 2);
        assert_eq!(queue.messages().len(), 3); // prefix + summary + 原消息
    }

    #[test]
    fn replay_requires_system_prefix_first() {
        let msgs = vec![QueueMessage::new(Role::System, "P", 0)];
        let queue = Queue::from_messages(msgs, 100);
        assert_eq!(queue.prefix().content.as_deref(), Some("P"));
    }

    #[test]
    #[should_panic]
    fn replay_rejects_non_prefix_first() {
        let msgs = vec![QueueMessage::new(Role::User, "x", 0)];
        let _ = Queue::from_messages(msgs, 100);
    }
}
