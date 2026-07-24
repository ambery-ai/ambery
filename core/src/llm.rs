//! LLM 抽象 + debug 模式 agent（docs/agent-loop.md）。
//! DebugAgent 用确定性规则模拟ペット决策，无网络依赖；真实 OpenAI 客户端后续接入。

use crate::queue::{QueueMessage, Role, ToolCall};
use serde_json::{json, Value};
use std::future::Future;

/// OpenAI 风格 function definition
pub struct ToolDef {
    pub name: &'static str,
    pub description: &'static str,
    pub parameters: Value,
}

/// Tool Set（concepts §10a）：ペット的权限边界，仅此四个
pub fn tool_set() -> Vec<ToolDef> {
    vec![
        ToolDef {
            name: "call_component",
            description: "调用 Component 展示信息（text_card/quick_jump/git_display/data_chart/todobox）",
            parameters: json!({
                "type": "object",
                "properties": { "spec": { "type": "object", "description": "ComponentSpec，见 docs/components.md" } },
                "required": ["spec"]
            }),
        },
        ToolDef {
            name: "fetch_terminal",
            description: "按需读取指定实例的当前 Terminal Content",
            parameters: json!({
                "type": "object",
                "properties": { "instance": { "type": "string" } },
                "required": ["instance"]
            }),
        },
        ToolDef {
            name: "set_autonomy",
            description: "覆盖 Autonomy 的表情/移动（ttlMs 后回落默认；全空=立即回落）",
            parameters: json!({
                "type": "object",
                "properties": {
                    "face": { "type": "string" },
                    "motion": { "type": "string", "enum": ["still", "float", "bounce", "shake"] },
                    "ttlMs": { "type": "integer" }
                }
            }),
        },
        ToolDef {
            name: "edit_config",
            description: "修改 Config 的 kaomoji 映射（key → face/motion）",
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "face": { "type": "string" },
                    "motion": { "type": "string", "enum": ["still", "float", "bounce", "shake"] }
                },
                "required": ["key"]
            }),
        },
    ]
}

pub struct LlmOutput {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
}

pub trait Llm {
    fn complete(
        &self,
        messages: &[QueueMessage],
        tools: &[ToolDef],
    ) -> impl Future<Output = LlmOutput> + Send;
}

/// debug 模式 agent：确定性规则（docs/agent-loop.md §debug 规则）
pub struct DebugAgent {
    /// 通知阈值：hook 内容长度 ≥ 此值才通知（fixture，真实判断由 LLM 做）
    pub notify_threshold: usize,
}

impl Default for DebugAgent {
    fn default() -> Self {
        Self {
            notify_threshold: 80,
        }
    }
}

impl Llm for DebugAgent {
    fn complete(
        &self,
        messages: &[QueueMessage],
        _tools: &[ToolDef],
    ) -> impl Future<Output = LlmOutput> + Send {
        let out = self.decide(messages);
        async move { out }
    }
}

impl DebugAgent {
    fn decide(&self, messages: &[QueueMessage]) -> LlmOutput {
        match messages.last() {
            // tool result 收尾：fetch_terminal → 汇总回复；通知类动作已通过 Component 表达 → 沉默
            Some(m) if m.role == Role::Tool => {
                let is_fetch = m
                    .tool_call_id
                    .as_deref()
                    .and_then(|id| tool_name_of(messages, id))
                    == Some("fetch_terminal");
                if is_fetch {
                    LlmOutput {
                        content: Some(format!(
                            "[debug] 查到：{}",
                            truncate(m.content.as_deref().unwrap_or(""), 120)
                        )),
                        tool_calls: vec![],
                    }
                } else {
                    LlmOutput {
                        content: None,
                        tool_calls: vec![],
                    }
                }
            }
            // 用户消息
            Some(m) if m.role == Role::User => {
                let text = m.content.clone().unwrap_or_default();
                if text.contains("具体") || text.contains("怎么回事") {
                    match last_completed_instance(messages) {
                        Some(inst) => LlmOutput {
                            content: None,
                            tool_calls: vec![ToolCall {
                                id: "dbg-fetch".into(),
                                name: "fetch_terminal".into(),
                                arguments: json!({ "instance": inst }).to_string(),
                            }],
                        },
                        None => LlmOutput {
                            content: Some("[debug] 没有可查的实例记录".into()),
                            tool_calls: vec![],
                        },
                    }
                } else {
                    LlmOutput {
                        content: Some(format!(
                            "[debug] 收到：{text}（Queue 共 {} 条）",
                            messages.len()
                        )),
                        tool_calls: vec![],
                    }
                }
            }
            // hook 注入的 system 消息：按内容长度决定通知/沉默
            Some(m) if m.role == Role::System => {
                let c = m.content.clone().unwrap_or_default();
                match parse_hook_msg(&c) {
                    Some((inst, len)) if len >= self.notify_threshold => LlmOutput {
                        content: None,
                        tool_calls: vec![
                            ToolCall {
                                id: "dbg-autonomy".into(),
                                name: "set_autonomy".into(),
                                arguments: json!({ "motion": "bounce", "ttlMs": 5000 })
                                    .to_string(),
                            },
                            ToolCall {
                                id: "dbg-component".into(),
                                name: "call_component".into(),
                                arguments: json!({
                                    "spec": {
                                        "id": format!("notify-{inst}"),
                                        "type": "text_card",
                                        "title": format!("{inst} 完成"),
                                        "text": format!("[debug] {inst} 干完了（{len} 字），去看看吧"),
                                        "direction": "auto"
                                    }
                                })
                                .to_string(),
                            },
                        ],
                    },
                    // 沉默（len < 阈值 / 其他 system 消息）：不追加任何消息
                    _ => LlmOutput {
                        content: None,
                        tool_calls: vec![],
                    },
                }
            }
            _ => LlmOutput {
                content: None,
                tool_calls: vec![],
            },
        }
    }
}

/// 由 tool_call_id 反查 tool 名（向前找发起它的 assistant tool_calls 消息）
fn tool_name_of<'a>(messages: &'a [QueueMessage], tool_call_id: &str) -> Option<&'a str> {
    messages.iter().rev().find_map(|m| {
        m.tool_calls.as_ref()?.iter().find_map(|c| {
            if c.id == tool_call_id {
                Some(c.name.as_str())
            } else {
                None
            }
        })
    })
}

/// 解析「{instance} 完成，Context 已更新（{len} 字）。评估是否通知。」
fn parse_hook_msg(content: &str) -> Option<(String, usize)> {
    let (inst, rest) = content.split_once(" 完成，Context 已更新（")?;
    let (len_str, _) = rest.split_once(" 字")?;
    Some((inst.to_string(), len_str.parse().ok()?))
}

/// 最近一个完成实例（user 追问时定位 fetch_terminal 目标）
fn last_completed_instance(messages: &[QueueMessage]) -> Option<String> {
    messages.iter().rev().find_map(|m| {
        if m.role == Role::System {
            let c = m.content.as_ref()?;
            let (inst, _) = c.split_once(" 完成，Context 已更新")?;
            return Some(inst.to_string());
        }
        None
    })
}

fn truncate(s: &str, max: usize) -> String {
    let t: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{t}…")
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_hook_msg_ok() {
        let (inst, len) =
            parse_hook_msg("ft 完成，Context 已更新（123 字）。评估是否通知。").unwrap();
        assert_eq!(inst, "ft");
        assert_eq!(len, 123);
    }

    #[test]
    fn last_completed_instance_scans_backward() {
        let msgs = vec![
            QueueMessage::new(Role::System, "prefix", 0),
            QueueMessage::new(Role::System, "a 完成，Context 已更新（10 字）。评估是否通知。", 1),
            QueueMessage::new(Role::System, "b 完成，Context 已更新（20 字）。评估是否通知。", 2),
        ];
        assert_eq!(last_completed_instance(&msgs), Some("b".to_string()));
    }
}
