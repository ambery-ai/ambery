//! LLM 抽象 + debug 模式 agent（docs/agent-loop.md）。
//! DebugAgent 是纯 mock：零逻辑，返回什么完全由外部决策源注入
//! （测试脚本闭包 / debug CLI / 沉默兜底），它只负责转发。

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
            description: "按需读取指定实例的当前 Terminal Content。vd_switch 必填：false=不切桌面（读不到且目标可能在其他虚拟桌面时失败，提示重试）；true=目标在其他虚拟桌面时切过去读（不切回）",
            parameters: json!({
                "type": "object",
                "properties": {
                    "instance": { "type": "string" },
                    "vd_switch": { "type": "boolean" }
                },
                "required": ["instance", "vd_switch"]
            }),
        },
        ToolDef {
            name: "set_autonomy",
            description: "覆盖 Autonomy 的表情/移动（ttlMs 后回落默认；全空=立即回落）。face 传颜文字本体或状态 key 名（key 解析为映射本体，仅解析 face）",
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
            description: "修改 Config（统一配置管道，非法值被拒绝并返回错误）。path 为点分路径，value 为新值（JSON）。例：新增表情状态 path=kaomoji.celebrate value={\"face\":\"(≧▽≦)\",\"motion\":\"bounce\"}；调阈值 path=token_threshold value=5000",
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "value": {}
                },
                "required": ["path", "value"]
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

    /// Compression 专项摘要（concepts §10d / docs/storage.md compact_boundary）。
    /// 默认确定性 stub（DebugAgent / 测试保证确定性）；OpenAiClient 覆写为真实调用。
    fn summarize(
        &self,
        messages: &[QueueMessage],
    ) -> impl Future<Output = Result<String, String>> + Send {
        let summary = deterministic_summary(messages);
        async move { Ok(summary) }
    }
}

/// 确定性摘要 stub（Compression 的 debug 回退，docs/harness.md：保证测试确定性）
pub fn deterministic_summary(messages: &[QueueMessage]) -> String {
    let n = messages.len();
    let first = messages
        .first()
        .and_then(|m| m.content.as_deref())
        .unwrap_or("")
        .chars()
        .take(20)
        .collect::<String>();
    let last = messages
        .last()
        .and_then(|m| m.content.as_deref())
        .unwrap_or("")
        .chars()
        .take(20)
        .collect::<String>();
    format!("共 {n} 条历史：首「{first}」末「{last}」")
}

/// debug 模式 agent：纯 mock，零逻辑。决策源由外部注入——
/// 测试用脚本闭包、debug 二进制用 CLI、降级兜底用沉默。
pub struct DebugAgent {
    decide: Box<dyn Fn(&[QueueMessage]) -> LlmOutput + Send + Sync>,
}

impl DebugAgent {
    /// 注入外部决策源（mock 的「人为控制返回」）
    pub fn new(decide: impl Fn(&[QueueMessage]) -> LlmOutput + Send + Sync + 'static) -> Self {
        Self {
            decide: Box::new(decide),
        }
    }

    /// 永远沉默：OpenAi 失败降级、不需要反应的测试
    pub fn silent() -> Self {
        Self::new(|_| LlmOutput {
            content: None,
            tool_calls: vec![],
            reasoning_content: None,
        })
    }
}

impl Default for DebugAgent {
    fn default() -> Self {
        Self::silent()
    }
}

impl Llm for DebugAgent {
    fn complete(
        &self,
        messages: &[QueueMessage],
        _tools: &[ToolDef],
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        let out = (self.decide)(messages);
        async move { Ok(out) }
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

impl OpenAiClient {
    /// 专项摘要调用（无 tools）：历史序列化为对话文本，要求直接输出摘要
    async fn summarize_async(&self, messages: &[QueueMessage]) -> Result<String, String> {
        let mut transcript = String::new();
        for m in messages {
            let role = match m.role {
                Role::System => "system",
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::Tool => "tool",
            };
            let body = m.content.as_deref().unwrap_or("[tool_calls]");
            transcript.push_str(&format!("{role}: {}\n", truncate(body, 500)));
        }
        let prompt = vec![
            QueueMessage::new(
                Role::System,
                "你是摘要器：把监工宠物的对话历史压缩为简洁中文摘要，保留：实例名、状态变化、用户意图、未决事项。只输出摘要文本。",
                0,
            ),
            QueueMessage::new(Role::User, transcript, 0),
        ];
        let out = self.complete_async(&prompt, &[]).await?;
        out.content
            .filter(|c| !c.is_empty())
            .ok_or_else(|| "摘要返回为空".into())
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

    fn summarize(
        &self,
        messages: &[QueueMessage],
    ) -> impl Future<Output = Result<String, String>> + Send {
        self.summarize_async(messages)
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

    fn summarize(
        &self,
        messages: &[QueueMessage],
    ) -> impl Future<Output = Result<String, String>> + Send {
        async move {
            match self {
                Self::Debug(agent) => agent.summarize(messages).await,
                Self::OpenAi { client, fallback } => match client.summarize(messages).await {
                    Ok(s) => Ok(s),
                    Err(err) => {
                        eprintln!("[llm] openai summarize 失败（{err}），回退确定性 stub");
                        fallback.summarize(messages).await
                    }
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn mock_returns_exactly_what_source_gives() {
        use std::sync::Mutex;
        let script = Mutex::new(std::collections::VecDeque::from(vec![
            LlmOutput {
                content: Some("脚本第一句".into()),
                tool_calls: vec![],
                reasoning_content: None,
            },
            LlmOutput {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "s1".into(),
                    name: "set_autonomy".into(),
                    arguments: "{\"face\":\"(・ω・)\"}".into(),
                }],
                reasoning_content: None,
            },
        ]));
        let agent = DebugAgent::new(move |_| {
            script.lock().unwrap().pop_front().unwrap_or(LlmOutput {
                content: None,
                tool_calls: vec![],
                reasoning_content: None,
            })
        });
        // 脚本怎么写，就怎么返回；耗尽 → 沉默
        let msgs = [QueueMessage::new(Role::User, "任意输入", 0)];
        let out1 = agent.complete(&msgs, &tool_set()).await.unwrap();
        assert_eq!(out1.content.as_deref(), Some("脚本第一句"));
        let out2 = agent.complete(&msgs, &tool_set()).await.unwrap();
        assert_eq!(out2.tool_calls.len(), 1);
        let out3 = agent.complete(&msgs, &tool_set()).await.unwrap();
        assert!(out3.content.is_none() && out3.tool_calls.is_empty());
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
