//! Context：完整消息数组（OpenAI messages 对齐）——
//! Queue 放行的输入 + LLM 的 assistant/tool 输出。LLM 请求的上下文源，也是完整对话的
//! 持久化存档（context.jsonl message 行双写）。system prompt 不是 Context 消息——
//! 它是每次调用现拼的请求头。

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
pub struct ContextMessage {
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

impl ContextMessage {
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

/// shaking 保留条数的最终兜底（配置值见 Config.context_compression_keep_recent_messages，
/// 本常量仅供无配置场景/测试）
pub const KEEP_RECENT: usize = 24;

pub struct Context {
    messages: Vec<ContextMessage>,
    pub token_threshold: usize,
}

impl Context {
    pub fn new(token_threshold: usize) -> Self {
        Self {
            messages: vec![],
            token_threshold,
        }
    }

    pub fn messages(&self) -> &[ContextMessage] {
        &self.messages
    }

    pub fn push(&mut self, msg: ContextMessage) {
        self.messages.push(msg);
    }

    pub fn total_tokens(&self) -> usize {
        self.messages.iter().map(ContextMessage::est_tokens).sum()
    }

    /// 从 start 起的消息 token 估算和（#16 增量估算；est 失真仅作边际用途）
    pub fn est_tokens_since(&self, start: usize) -> usize {
        self.messages[start.min(self.messages.len())..]
            .iter()
            .map(ContextMessage::est_tokens)
            .sum()
    }

    pub fn needs_compression(&self) -> bool {
        self.total_tokens() > self.token_threshold
    }

    /// 总结 + shaking：历史压为一条 system 摘要，按完整 turn 边界保留最近
    /// `keep_recent` 条（目标值）：
    /// - 切口不得落在 tool 序列中间（孤儿 tool result 使下一请求体非法）——切口前进越过；
    /// - `min_tail_start` 保护当前在飞 turn 不被切断（调用方给本 turn 输入消息的下标）。
    /// lang：摘要前缀按 Harness 语言
    pub fn compress(
        &mut self,
        summary: String,
        keep_recent: usize,
        min_tail_start: usize,
        lang: crate::i18n::Lang,
        ts: i64,
    ) {
        let mut tail_start = self.messages.len().saturating_sub(keep_recent);
        while tail_start < self.messages.len() && self.messages[tail_start].role == Role::Tool {
            tail_start += 1;
        }
        tail_start = tail_start.min(min_tail_start);
        let tail: Vec<ContextMessage> = self.messages[tail_start..].to_vec();
        self.messages = vec![ContextMessage::new(
            Role::System,
            crate::i18n::trf(lang, "context.summary", &[("summary", summary)]),
            ts,
        )];
        self.messages.extend(tail);
    }

    /// replay 恢复（纯消息序列，无格式前置要求）
    pub fn from_messages(messages: Vec<ContextMessage>, token_threshold: usize) -> Self {
        Self {
            messages,
            token_threshold,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Context {
        Context::new(100)
    }

    #[test]
    fn push_appends_in_order() {
        let mut ctx = ctx();
        ctx.push(ContextMessage::new(Role::User, "u1", 1));
        ctx.push(ContextMessage::new(Role::Assistant, "a1", 2));
        let roles: Vec<Role> = ctx.messages().iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant]);
    }

    #[test]
    fn compress_keeps_summary_and_recent() {
        let mut ctx = ctx();
        for i in 0..30 {
            ctx.push(ContextMessage::new(Role::User, format!("msg-{i}"), i as i64));
        }
        assert!(ctx.needs_compression()); // 30 条 > 100 阈值
        ctx.compress("前文提要".into(), KEEP_RECENT, usize::MAX, crate::i18n::Lang::Zh, 99);
        let msgs = ctx.messages();
        assert_eq!(msgs.len(), 1 + KEEP_RECENT);
        assert_eq!(msgs[0].content.as_deref(), Some("[历史摘要] 前文提要"));
        assert_eq!(msgs[1].content.as_deref(), Some("msg-6")); // 最近 24 条
        assert_eq!(msgs.last().unwrap().content.as_deref(), Some("msg-29"));
    }

    #[test]
    fn compress_never_splits_tool_sequence() {
        // turn 边界：切口不落在 tool 序列中间
        let mut ctx = ctx();
        ctx.push(ContextMessage::new(Role::User, "u", 1));
        let mut ac = ContextMessage::new(Role::Assistant, "", 2);
        ac.tool_calls = Some(vec![crate::context::ToolCall {
            id: "c1".into(),
            name: "sleep".into(),
            arguments: "{}".into(),
        }]);
        ctx.push(ac);
        ctx.push(ContextMessage::tool_result("c1", "{}", 3));
        ctx.push(ContextMessage::tool_result("c2", "{}", 4));
        ctx.push(ContextMessage::new(Role::Assistant, "done", 5));
        // keep_recent=2 → 候选切口落在 tool_result 上 → 必须前进越过，不产生孤儿 tool result
        ctx.compress("s".into(), 2, usize::MAX, crate::i18n::Lang::Zh, 9);
        let roles: Vec<Role> = ctx.messages().iter().map(|m| m.role).collect();
        assert_eq!(roles[1], Role::Assistant, "切口越过 tool 段: {roles:?}");
        assert!(!roles.contains(&Role::Tool) || roles.iter().position(|r| *r == Role::Tool).unwrap() > 1);
    }

    #[test]
    fn compress_respects_min_tail_start() {
        // 在飞 turn 保护：keep 目标再小也不切断当前 turn（min_tail_start 收口）
        let mut ctx = ctx();
        for i in 0..10 {
            ctx.push(ContextMessage::new(Role::User, format!("m{i}"), i as i64));
        }
        ctx.compress("s".into(), 2, 8, crate::i18n::Lang::Zh, 99);
        // tail_start = min(10-2=8 越过后无 tool, 8) = 8 → 保留 m8,m9
        let msgs = ctx.messages();
        assert_eq!(msgs.len(), 1 + 2);
        assert_eq!(msgs[1].content.as_deref(), Some("m8"));
    }

    #[test]
    fn compress_short_context_is_noop_like() {
        let mut ctx = ctx();
        ctx.push(ContextMessage::new(Role::User, "only", 1));
        ctx.compress("s".into(), 2, usize::MAX, crate::i18n::Lang::Zh, 2);
        assert_eq!(ctx.messages().len(), 2); // summary + 原消息
    }

    #[test]
    fn replay_preserves_order() {
        let msgs = vec![
            ContextMessage::new(Role::User, "u", 0),
            ContextMessage::new(Role::Assistant, "a", 1),
        ];
        let ctx = Context::from_messages(msgs, 100);
        let roles: Vec<Role> = ctx.messages().iter().map(|m| m.role).collect();
        assert_eq!(roles, vec![Role::User, Role::Assistant]);
    }
}
