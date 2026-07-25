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
    /// 推理模型的思维链（deepseek thinking 模式），回放历史时必须带回
    pub reasoning_content: Option<String>,
}

/// 返回 Result：OpenAiClient 网络/解析失败时 LlmBackend 可降级 DebugAgent
pub trait Llm {
    fn complete(
        &self,
        messages: &[QueueMessage],
        tools: &[ToolDef],
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send;
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
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        let out = self.decide(messages);
        async move { Ok(out) }
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
                        reasoning_content: None,
                    }
                } else {
                    LlmOutput {
                        content: None,
                        tool_calls: vec![],
                        reasoning_content: None,                    }
                }
            }
            // 用户消息
            Some(m) if m.role == Role::User => {
                let text = m.content.clone().unwrap_or_default();
                if text.contains("具体") || text.contains("怎么回事") {
                    match self.last_noteworthy_instance(messages) {
                        Some(inst) => LlmOutput {
                            content: None,
                            tool_calls: vec![ToolCall {
                                id: "dbg-fetch".into(),
                                name: "fetch_terminal".into(),
                                arguments: json!({ "instance": inst }).to_string(),
                            }],
                            reasoning_content: None,
                        },
                        None => LlmOutput {
                            content: Some("[debug] 没有可查的实例记录".into()),
                            tool_calls: vec![],
                            reasoning_content: None,                        },
                    }
                } else {
                    LlmOutput {
                        content: Some(format!(
                            "[debug] 收到：{text}（Queue 共 {} 条）",
                            messages.len()
                        )),
                        tool_calls: vec![],
                        reasoning_content: None,
                    }
                }
            }
            // hook 注入的 system 消息：按内容长度决定通知/沉默
            Some(m) if m.role == Role::System => {
                let c = m.content.clone().unwrap_or_default();
                // 新实例注册（Example A）：问候 (・ω・)ノ + 展示实例一览
                if c.starts_with("新实例 ") && c.ends_with(" 已注册") {
                    let overview = instance_overview(messages);
                    return LlmOutput {
                        content: None,
                        tool_calls: vec![
                            ToolCall {
                                id: "dbg-greet".into(),
                                name: "set_autonomy".into(),
                                arguments: json!({ "face": "(・ω・)ノ", "ttlMs": 3000 })
                                    .to_string(),
                            },
                            ToolCall {
                                id: "dbg-roster".into(),
                                name: "call_component".into(),
                                arguments: json!({
                                    "spec": {
                                        "id": "roster",
                                        "type": "text_card",
                                        "title": "实例一览",
                                        "text": overview,
                                        "direction": "auto"
                                    }
                                })
                                .to_string(),
                            },
                        ],
                        reasoning_content: None,
                    };
                }
                match parse_hook_msg(&c) {
                    Some((inst, len)) if len >= self.notify_threshold => LlmOutput {
                        content: None,
                        tool_calls: vec![
                            ToolCall {
                                id: "dbg-autonomy".into(),
                                name: "set_autonomy".into(),
                                arguments: json!({
                                    "face": "✧*｡٩(ˊᗜˋ*)و✧*｡",
                                    "motion": "bounce",
                                    "ttlMs": 5000
                                })
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
                        reasoning_content: None,
                    },
                    // 沉默（len < 阈值 / 其他 system 消息）：不追加任何消息
                    _ => LlmOutput {
                        content: None,
                        tool_calls: vec![],
                        reasoning_content: None,                    },
                }
            }
            _ => LlmOutput {
                content: None,
                tool_calls: vec![],
                reasoning_content: None,            },
        }
    }
}

/// 从 system prefix 提取实例一览（「## 当前实例状态」下的 "- " 行）
fn instance_overview(messages: &[QueueMessage]) -> String {
    let Some(prefix) = messages.first().and_then(|m| m.content.as_deref()) else {
        return "（无实例）".into();
    };
    let Some((_, after)) = prefix.split_once("## 当前实例状态") else {
        return "（无实例）".into();
    };
    let lines: Vec<&str> = after.lines().filter(|l| l.starts_with("- ")).collect();
    if lines.is_empty() {
        "（无实例）".into()
    } else {
        lines.join("\n")
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
/// 与 Timer 兜底注入的「{instance} 兜底扫描发现变化，Context 已更新（{len} 字）。…」同构
fn parse_hook_msg(content: &str) -> Option<(String, usize)> {
    let (head, rest) = content.split_once("，Context 已更新（")?;
    let inst = head
        .strip_suffix(" 兜底扫描发现变化")
        .or_else(|| head.strip_suffix(" 完成"))
        .unwrap_or(head);
    let (len_str, _) = rest.split_once(" 字")?;
    Some((inst.to_string(), len_str.parse().ok()?))
}

impl DebugAgent {
    /// user 追问时定位 fetch_terminal 目标：优先「最近一个触发通知的完成实例」
    /// （追问大概率关于值得通知的那个），无则取最近完成实例
    fn last_noteworthy_instance(&self, messages: &[QueueMessage]) -> Option<String> {
        let mut latest: Option<String> = None;
        for m in messages.iter().rev() {
            if m.role != Role::System {
                continue;
            }
            let Some(c) = m.content.as_ref() else {
                continue;
            };
            let Some((inst, len)) = parse_hook_msg(c) else {
                continue;
            };
            if len >= self.notify_threshold {
                return Some(inst);
            }
            if latest.is_none() {
                latest = Some(inst);
            }
        }
        latest
    }
}

fn truncate(s: &str, max: usize) -> String {
    let t: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        format!("{t}…")
    } else {
        t
    }
}

// ── OpenAiClient（真实 OpenAI 兼容端点） ──

use crate::LlmProvider;

pub struct OpenAiClient {
    base_url: String,
    model: String,
    api_key: String,
    temperature: Option<f64>,
    http: reqwest::Client,
}

impl OpenAiClient {
    /// 从 provider profile 构造；key 从 api_key_env 指向的环境变量读（本体不落盘）
    pub fn from_provider(p: &LlmProvider) -> Result<Self, String> {
        let key_env = p.api_key_env.as_deref().unwrap_or("");
        let api_key = std::env::var(key_env)
            .map_err(|_| format!("环境变量 {key_env} 未设置"))?;
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self {
            base_url: p.base_url.trim_end_matches('/').to_string(),
            model: p.model.clone(),
            api_key,
            temperature: p.temperature,
            http,
        })
    }

    /// QueueMessage → OpenAI messages（assistant tool_calls / tool tool_call_id 对齐 §10）
    fn build_body(&self, messages: &[QueueMessage], tools: &[ToolDef]) -> Value {
        let msgs: Vec<Value> = messages
            .iter()
            .map(|m| {
                let role = match m.role {
                    Role::System => "system",
                    Role::User => "user",
                    Role::Assistant => "assistant",
                    Role::Tool => "tool",
                };
                let mut v = json!({ "role": role });
                v["content"] = match &m.content {
                    Some(c) => json!(c),
                    None => Value::Null,
                };
                if let Some(calls) = &m.tool_calls {
                    v["tool_calls"] = calls
                        .iter()
                        .map(|c| {
                            json!({
                                "id": c.id,
                                "type": "function",
                                "function": { "name": c.name, "arguments": c.arguments }
                            })
                        })
                        .collect();
                    // thinking 模型要求回放带 reasoning_content（空串可过，覆盖旧记录）
                    v["reasoning_content"] =
                        json!(m.reasoning_content.clone().unwrap_or_default());
                }
                if let Some(id) = &m.tool_call_id {
                    v["tool_call_id"] = json!(id);
                }
                v
            })
            .collect();
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect();
        let mut body = json!({
            "model": self.model,
            "messages": msgs,
            "tools": tools_json,
        });
        if let Some(t) = self.temperature {
            body["temperature"] = json!(t);
        }
        body
    }

    async fn complete_async(
        &self,
        messages: &[QueueMessage],
        tools: &[ToolDef],
    ) -> Result<LlmOutput, String> {
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&self.build_body(messages, tools))
            .send()
            .await
            .map_err(|e| format!("http: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read: {e}"))?;
        if !status.is_success() {
            return Err(format!("{status}: {}", truncate(&text, 200)));
        }
        parse_chat_response(&text)
    }
}

impl Llm for OpenAiClient {
    fn complete(
        &self,
        messages: &[QueueMessage],
        tools: &[ToolDef],
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        self.complete_async(messages, tools)
    }
}

/// OpenAI chat/completions 响应 → LlmOutput（只取需要的字段，容忍额外字段）
fn parse_chat_response(text: &str) -> Result<LlmOutput, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("json: {e}"))?;
    let msg = &v["choices"][0]["message"];
    if msg.is_null() {
        return Err(format!("响应缺 choices[0].message: {}", truncate(text, 200)));
    }
    let content = msg["content"].as_str().map(String::from);
    let reasoning_content = msg["reasoning_content"].as_str().map(String::from);
    let tool_calls = msg["tool_calls"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    Some(ToolCall {
                        id: c["id"].as_str()?.to_string(),
                        name: c["function"]["name"].as_str()?.to_string(),
                        arguments: c["function"]["arguments"].as_str()?.to_string(),
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    Ok(LlmOutput {
        content,
        tool_calls,
        reasoning_content,
    })
}

// ── LlmBackend：按 Config 装配，OpenAI 失败降级 DebugAgent ──

use crate::LlmConfig;

pub enum LlmBackend {
    Debug(DebugAgent),
    OpenAi {
        client: OpenAiClient,
        fallback: DebugAgent,
    },
}

impl LlmBackend {
    pub fn from_config(cfg: &LlmConfig) -> Self {
        if cfg.active == "debug" {
            return Self::Debug(DebugAgent::default());
        }
        match cfg.providers.get(&cfg.active) {
            Some(p) => match OpenAiClient::from_provider(p) {
                Ok(client) => Self::OpenAi {
                    client,
                    fallback: DebugAgent::default(),
                },
                Err(err) => {
                    eprintln!("[llm] provider「{}」初始化失败（{err}），回退 DebugAgent", cfg.active);
                    Self::Debug(DebugAgent::default())
                }
            },
            None => {
                eprintln!("[llm] active=「{}」不在 providers 里，回退 DebugAgent", cfg.active);
                Self::Debug(DebugAgent::default())
            }
        }
    }
}

impl Llm for LlmBackend {
    fn complete(
        &self,
        messages: &[QueueMessage],
        tools: &[ToolDef],
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        async move {
            match self {
                Self::Debug(agent) => agent.complete(messages, tools).await,
                Self::OpenAi { client, fallback } => match client.complete(messages, tools).await {
                    Ok(out) => Ok(out),
                    Err(err) => {
                        eprintln!("[llm] openai complete 失败（{err}），本轮回退 DebugAgent");
                        fallback.complete(messages, tools).await
                    }
                },
            }
        }
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
        // Timer 兜底注入的同构消息
        let (inst2, len2) =
            parse_hook_msg("config-service 兜底扫描发现变化，Context 已更新（456 字）。评估是否通知。")
                .unwrap();
        assert_eq!(inst2, "config-service");
        assert_eq!(len2, 456);
        // 含空格的实例名（如 "✳ mock-a"）
        let (inst3, _) =
            parse_hook_msg("✳ mock-a 完成，Context 已更新（1 字）。评估是否通知。")
                .unwrap();
        assert_eq!(inst3, "✳ mock-a");
    }

    #[test]
    fn noteworthy_prefers_notified_then_latest() {
        let agent = DebugAgent::default();
        let msgs = vec![
            QueueMessage::new(Role::System, "prefix", 0),
            QueueMessage::new(Role::System, "a 完成，Context 已更新（100 字）。评估是否通知。", 1),
            QueueMessage::new(Role::System, "b 完成，Context 已更新（9 字）。评估是否通知。", 2),
        ];
        // b 更新但太短未通知 → 追问应定位到 a
        assert_eq!(
            agent.last_noteworthy_instance(&msgs),
            Some("a".to_string())
        );
        // 都没有达到阈值 → 取最近完成（b）
        let msgs2 = vec![
            QueueMessage::new(Role::System, "a 完成，Context 已更新（10 字）。评估是否通知。", 1),
            QueueMessage::new(Role::System, "b 完成，Context 已更新（20 字）。评估是否通知。", 2),
        ];
        assert_eq!(
            agent.last_noteworthy_instance(&msgs2),
            Some("b".to_string())
        );
    }

    fn test_client() -> OpenAiClient {
        OpenAiClient {
            base_url: "http://x/v1".into(),
            model: "m".into(),
            api_key: "k".into(),
            temperature: Some(0.3),
            http: reqwest::Client::new(),
        }
    }

    #[test]
    fn openai_body_maps_tool_flow() {
        let client = test_client();
        let msgs = vec![
            QueueMessage::new(Role::System, "prefix", 0),
            QueueMessage::assistant_tool_calls(
                vec![ToolCall {
                    id: "c1".into(),
                    name: "fetch_terminal".into(),
                    arguments: "{\"instance\":\"ft\"}".into(),
                }],
                1,
            ),
            QueueMessage::tool_result("c1", "{\"ok\":true}", 2),
        ];
        let body = client.build_body(&msgs, &tool_set());
        assert_eq!(body["model"], json!("m"));
        assert_eq!(body["messages"][0]["role"], json!("system"));
        assert_eq!(body["messages"][1]["content"], Value::Null);
        assert_eq!(
            body["messages"][1]["tool_calls"][0]["function"]["name"],
            json!("fetch_terminal")
        );
        assert_eq!(body["messages"][2]["role"], json!("tool"));
        assert_eq!(body["messages"][2]["tool_call_id"], json!("c1"));
        assert_eq!(
            body["tools"][0]["function"]["name"],
            json!("call_component")
        );
        assert_eq!(body["temperature"], json!(0.3));
    }

    #[test]
    fn parse_response_content_and_tool_calls() {
        let text = r#"{"choices":[{"message":{"content":null,"tool_calls":[{"id":"a","type":"function","function":{"name":"set_autonomy","arguments":"{\"motion\":\"bounce\"}"}}]}}]}"#;
        let out = parse_chat_response(text).unwrap();
        assert!(out.content.is_none());
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].name, "set_autonomy");
        assert_eq!(out.tool_calls[0].id, "a");

        let text2 = r#"{"choices":[{"message":{"content":"你好","tool_calls":null}}]}"#;
        let out2 = parse_chat_response(text2).unwrap();
        assert_eq!(out2.content.as_deref(), Some("你好"));
        assert!(out2.tool_calls.is_empty());
    }

    #[test]
    fn llm_backend_falls_back_to_debug() {
        // active 不在 providers → Debug
        let cfg = LlmConfig {
            active: "nope".into(),
            providers: Default::default(),
        };
        assert!(matches!(LlmBackend::from_config(&cfg), LlmBackend::Debug(_)));
        // provider 存在但 env 未设 → Debug
        let mut providers = std::collections::HashMap::new();
        providers.insert(
            "p".to_string(),
            LlmProvider {
                base_url: "http://x".into(),
                model: "m".into(),
                api_key_env: Some("DEFINITELY_NOT_SET_ENV_VAR".into()),
                temperature: None,
            },
        );
        let cfg2 = LlmConfig {
            active: "p".into(),
            providers,
        };
        assert!(matches!(LlmBackend::from_config(&cfg2), LlmBackend::Debug(_)));
    }
}
