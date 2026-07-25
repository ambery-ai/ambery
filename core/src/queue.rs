//! Queue（concepts §10c，docs/harness.md）：纯对话与事件的内存视图，顺序处理。
//! system prompt 不是 Queue 消息——它是每次调用现拼的请求头（docs/storage.md）。

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
    /// 推理模型的思维链（deepseek thinking 模式：带 tool_calls 的 assistant
    /// 消息回放时必须带此字段，空串可过——网关实测）。旧记录无此字段兼容。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
    pub ts: i64,
}

impl QueueMessage {
    pub fn new(role: Role, content: impl Into<String>, ts: i64) -> Self {
        Self {
            role,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: None,
            reasoning_content: None,
            ts,
        }
    }

    pub fn assistant_tool_calls(calls: Vec<ToolCall>, ts: i64) -> Self {
        Self {
            role: Role::Assistant,
            content: None,
            tool_calls: Some(calls),
            tool_call_id: None,
            reasoning_content: None,
            ts,
        }
    }

    pub fn tool_result(tool_call_id: impl Into<String>, content: impl Into<String>, ts: i64) -> Self {
        Self {
            role: Role::Tool,
            content: Some(content.into()),
            tool_calls: None,
            tool_call_id: Some(tool_call_id.into()),
            reasoning_content: None,
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

/// shaking 保留最近 N 条原始消息，concepts §10d
pub const KEEP_RECENT: usize = 8;

pub struct Queue {
    messages: Vec<QueueMessage>,
    pub token_threshold: usize,
}

impl Queue {
    pub fn new(token_threshold: usize) -> Self {
        Self {
            messages: vec![],
            token_threshold,
        }
    }

    pub fn messages(&self) -> &[QueueMessage] {
        &self.messages
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

    /// 总结 + shaking：历史压为一条 system 摘要，保留最近 KEEP_RECENT 条
    pub fn compress(&mut self, summary: String, ts: i64) {
        let tail_start = self.messages.len().saturating_sub(KEEP_RECENT);
        let tail: Vec<QueueMessage> = self.messages[tail_start..].to_vec();
        self.messages = vec![QueueMessage::new(Role::System, format!("[历史摘要] {summary}"), ts)];
        self.messages.extend(tail);
    }

    /// replay 恢复（纯消息序列，无格式前置要求）
    pub fn from_messages(messages: Vec<QueueMessage>, token_threshold: usize) -> Self {
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
        Queue::new(100)
    }

    #[test]
    fn push_appends_in_order() {
        let mut queue = q();
        queue.push(QueueMessage::new(Role::User, "u1", 1));
        queue.push(QueueMessage::new(Role::Assistant, "a1", 2));
        let roles: Vec<Role> = queue.messages().iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant]);
    }

    #[test]
    fn compress_keeps_summary_and_recent() {
        let mut queue = q();
        for i in 0..21 {
            queue.push(QueueMessage::new(Role::User, format!("msg-{i}"), i as i64));
        }
        assert!(queue.needs_compression()); // 21 条 × (4 + ~1) > 100
        queue.compress("前文提要".into(), 99);
        let msgs = queue.messages();
        assert_eq!(msgs.len(), 1 + KEEP_RECENT);
        assert_eq!(msgs[0].content.as_deref(), Some("[历史摘要] 前文提要"));
        assert_eq!(msgs[1].content.as_deref(), Some("msg-13")); // 最近 8 条
        assert_eq!(msgs.last().unwrap().content.as_deref(), Some("msg-20"));
    }

    #[test]
    fn compress_short_queue_is_noop_like() {
        let mut queue = q();
        queue.push(QueueMessage::new(Role::User, "only", 1));
        queue.compress("s".into(), 2);
        assert_eq!(queue.messages().len(), 2); // summary + 原消息
    }

    #[test]
    fn replay_preserves_order() {
        let msgs = vec![
            QueueMessage::new(Role::User, "u", 0),
            QueueMessage::new(Role::Assistant, "a", 1),
        ];
        let queue = Queue::from_messages(msgs, 100);
        let roles: Vec<Role> = queue.messages().iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant]);
    }
}
