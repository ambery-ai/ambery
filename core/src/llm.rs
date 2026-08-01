//! LLM 抽象 + debug 模式 agent（docs/agent-loop.md）。
//! DebugAgent 是纯 mock：零逻辑，返回什么完全由外部决策源注入
//! （测试脚本闭包 / debug CLI / 沉默兜底），它只负责转发。

use crate::context::{ContextMessage, Role, ToolCall};
use serde::{Deserialize, Serialize};
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
            description: "创建/更新/关闭卡片窗口。同 id 首次创建、后续原地更新；action=\"close\" 关闭（只需 id，忽略其他字段）。id 仅限 A-Z a-z 0-9 _ - . /",
            parameters: json!({
                "type": "object",
                "properties": {
                    "spec": {
                        "type": "object",
                        "description": "ComponentSpec——按 type 选择分支填写对应字段",
                        "properties": {
                            "id": { "type": "string", "description": "唯一标识：仅 A-Z a-z 0-9 _ - . /" },
                            "action": { "type": "string", "enum": ["close"], "description": "设为 close 关闭卡片（此时只需 id，忽略其余字段）" },
                            "direction": { "type": "string", "description": "可选方位：auto/n/ne/e/se/s/sw/w/nw" }
                        },
                        "required": ["id"],
                        "anyOf": [
                            {
                                "properties": {
                                    "type": { "enum": ["text_card"] },
                                    "title": { "type": "string", "description": "卡片标题" },
                                    "text": { "type": "string", "description": "卡片正文" }
                                },
                                "required": ["type", "title", "text"]
                            },
                            {
                                "properties": {
                                    "type": { "enum": ["quick_jump"] },
                                    "label": { "type": "string", "description": "按钮标签" },
                                    "target": { "type": "string", "description": "跳转目标" }
                                },
                                "required": ["type", "label", "target"]
                            },
                            {
                                "properties": {
                                    "type": { "enum": ["git_display"] },
                                    "title": { "type": "string", "description": "卡片标题" },
                                    "entries": { "type": "array", "description": "提交列表", "items": { "type": "object", "properties": { "hash": { "type": "string" }, "msg": { "type": "string" }, "time": { "type": "string" } } } },
                                    "diff": { "type": "string", "description": "可选的 diff 内容" }
                                },
                                "required": ["type", "title", "entries"]
                            },
                            {
                                "properties": {
                                    "type": { "enum": ["data_chart"] },
                                    "title": { "type": "string", "description": "卡片标题" },
                                    "chart": {
                                        "type": "object",
                                        "description": "图表定义",
                                        "properties": {
                                            "kind": { "enum": ["line", "bar", "pie"] },
                                            "labels": { "type": "array", "items": { "type": "string" } },
                                            "series": { "type": "array", "items": { "type": "object", "properties": { "name": { "type": "string" }, "data": { "type": "array", "items": { "type": "number" } } } } }
                                        },
                                        "required": ["kind", "labels", "series"]
                                    }
                                },
                                "required": ["type", "title", "chart"]
                            },
                            {
                                "properties": {
                                    "type": { "enum": ["todobox"] },
                                    "title": { "type": "string", "description": "卡片标题" },
                                    "items": { "type": "array", "description": "待办条目", "items": { "type": "object", "properties": { "text": { "type": "string" }, "done": { "type": "boolean" } } } }
                                },
                                "required": ["type", "title", "items"]
                            }
                        ]
                    }
                },
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
            description: "覆盖 Autonomy 的表情/移动（ttlMs 后回落默认；全空=立即回落）。key 传状态 key 名（kaomoji 两池并集中的 key，如 idle/notify/processing）。once=true 按动画注册表 MotionDef.durationMs 自动取持续时间（一次播放收束），与 ttlMs 互斥",
            parameters: json!({
                "type": "object",
                "properties": {
                    "key": { "type": "string" },
                    "motion": { "type": "string", "enum": ["still", "float", "bounce", "shake"] },
                    "ttlMs": { "type": "integer" },
                    "once": { "type": "boolean", "description": "true=按 motion 的 MotionDef.durationMs 取 TTL；与 ttlMs 不能同时传" }
                }
            }),
        },
        ToolDef {
            name: "edit_config",
            description: "修改 Config（统一配置管道，非法值被拒绝并返回错误）。path 为点分路径，value 为新值（JSON）。例：新增表情状态 path=kaomoji.user.celebrate value={\"face\":\"(≧▽≦)\",\"motion\":\"bounce\"}；调缩放 path=view_scale value=0.8",
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

/// LLM 调用真值（#16）：usage.prompt_tokens / completion_tokens。
/// 三家（flash/gpt/sonnet）公约数一致，无模型分支；cache 分项实测恒 0 不存。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Usage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
}

pub struct LlmOutput {
    pub content: Option<String>,
    pub tool_calls: Vec<ToolCall>,
    /// 推理模型的思维链（deepseek thinking 模式），回放历史时必须带回
    pub reasoning_content: Option<String>,
    /// 本次调用的 token 真值（#16）；DebugAgent / 不支持端点 = None
    pub usage: Option<Usage>,
}

/// 流式增量（docs/streaming.md）：content / reasoning_content 两路，互斥非空。
/// Delta 纯显示优化——不经 Queue/Context，完整回复最后才写 Context。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Delta {
    pub content: Option<String>,
    pub reasoning_content: Option<String>,
}

/// 返回 Result：OpenAiClient 网络/解析失败时 LlmBackend 可降级 DebugAgent
pub trait Llm: Send + Sync {
    fn complete(
        &self,
        messages: &[ContextMessage],
        tools: &[ToolDef],
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send;
    /// 流式补全（docs/streaming.md）：边收边回调 Delta。
    /// 默认回落：一次性 complete 后把全文作为单个 delta 回调（不支持流式的客户端零改动）。
    fn complete_streaming(
        &self,
        messages: &[ContextMessage],
        tools: &[ToolDef],
        on_delta: &(dyn Fn(&Delta) + Send + Sync),
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        async move {
            let out = self.complete(messages, tools).await?;
            if let Some(c) = &out.content {
                if !c.is_empty() {
                    on_delta(&Delta {
                        content: Some(c.clone()),
                        reasoning_content: None,
                    });
                }
            }
            Ok(out)
        }
    }

    /// Compression 专项摘要（concepts §10d / docs/storage.md compact_boundary）。
    /// 返回（摘要, usage 真值）——摘要调用也留真值（#16 审计完整）。
    /// 默认确定性 stub（DebugAgent / 测试保证确定性）；OpenAiClient 覆写为真实调用。
    fn summarize(
        &self,
        messages: &[ContextMessage],
    ) -> impl Future<Output = Result<(String, Option<Usage>), String>> + Send {
        let summary = deterministic_summary(messages);
        async move { Ok((summary, None)) }
    }
}

/// 确定性摘要 stub（Compression 的 debug 回退，docs/harness.md：保证测试确定性）
pub fn deterministic_summary(messages: &[ContextMessage]) -> String {
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
    decide: Box<dyn Fn(&[ContextMessage]) -> LlmOutput + Send + Sync>,
}

impl DebugAgent {
    /// 注入外部决策源（mock 的「人为控制返回」）
    pub fn new(decide: impl Fn(&[ContextMessage]) -> LlmOutput + Send + Sync + 'static) -> Self {
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
        usage: None,
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
        messages: &[ContextMessage],
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

    /// ContextMessage → OpenAI messages（assistant tool_calls / tool tool_call_id 对齐 §10）
    fn build_body(&self, messages: &[ContextMessage], tools: &[ToolDef]) -> Value {
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
        messages: &[ContextMessage],
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

    /// SSE 流式补全（docs/streaming.md）：stream:true + 逐事件解析，
    /// content/reasoning_content 两路边收边回调；tool_calls 分片按 index 聚合。
    /// 字节级缓冲：\n\n 事件边界不可能切开 UTF-8 多字节字符（0x0A 不出现在多字节序列内）。
    async fn complete_streaming_async(
        &self,
        messages: &[ContextMessage],
        tools: &[ToolDef],
        on_delta: &(dyn Fn(&Delta) + Send + Sync),
    ) -> Result<LlmOutput, String> {
        use futures_util::StreamExt;
        let mut body = self.build_body(messages, tools);
        body["stream"] = serde_json::json!(true);
        body["stream_options"] = serde_json::json!({ "include_usage": true });
        let resp = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("http: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.map_err(|e| format!("read: {e}"))?;
            return Err(format!("{status}: {}", truncate(&text, 200)));
        }
        let mut acc = StreamAcc::default();
        let mut buf: Vec<u8> = vec![];
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| format!("stream: {e}"))?;
            buf.extend_from_slice(&chunk);
            while let Some(pos) = buf.windows(2).position(|w| w == b"\n\n") {
                let event: Vec<u8> = buf.drain(..pos + 2).collect();
                for line in String::from_utf8_lossy(&event).lines() {
                    let Some(data) = line.strip_prefix("data:") else { continue };
                    let data = data.trim();
                    if data == "[DONE]" {
                        continue;
                    }
                    let Ok(v) = serde_json::from_str::<Value>(data) else { continue };
                    if let Some(d) = acc.apply(&v) {
                        on_delta(&d);
                    }
                }
            }
        }
        Ok(acc.finish())
    }
}

impl OpenAiClient {
    /// 专项摘要调用（无 tools）：历史序列化为对话文本，要求直接输出摘要。
    /// 返回（摘要, usage 真值）——摘要调用同样留真值（#16）。
    async fn summarize_async(&self, messages: &[ContextMessage]) -> Result<(String, Option<Usage>), String> {
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
            ContextMessage::new(
                Role::System,
                "你是摘要器：把监工宠物的对话历史压缩为简洁中文摘要，保留：实例名、状态变化、用户意图、未决事项。只输出摘要文本。",
                0,
            ),
            ContextMessage::new(Role::User, transcript, 0),
        ];
        let out = self.complete_async(&prompt, &[]).await?;
        let summary = out
            .content
            .filter(|c| !c.is_empty())
            .ok_or_else(|| "摘要返回为空".to_string())?;
        Ok((summary, out.usage))
    }
}

impl Llm for OpenAiClient {
    fn complete(
        &self,
        messages: &[ContextMessage],
        tools: &[ToolDef],
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        self.complete_async(messages, tools)
    }

    fn complete_streaming(
        &self,
        messages: &[ContextMessage],
        tools: &[ToolDef],
        on_delta: &(dyn Fn(&Delta) + Send + Sync),
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        self.complete_streaming_async(messages, tools, on_delta)
    }

    fn summarize(
        &self,
        messages: &[ContextMessage],
    ) -> impl Future<Output = Result<(String, Option<Usage>), String>> + Send {
        self.summarize_async(messages)
    }
}

/// SSE 流式聚合器（docs/streaming.md）：content/reasoning 拼接，tool_calls 分片按 index 聚合；
/// 末尾 usage 帧（stream_options.include_usage，三家实测支持）收真值（#16）
#[derive(Default)]
struct StreamAcc {
    content: String,
    reasoning: String,
    tool_acc: std::collections::BTreeMap<u64, (Option<String>, Option<String>, String)>,
    usage: Option<Usage>,
}

impl StreamAcc {
    /// 应用一个 SSE chunk；有增量则返回 Delta（两路互斥由发送方保证，容忍同时非空）
    fn apply(&mut self, v: &Value) -> Option<Delta> {
        // usage 帧（空 delta + usage 对象）：收真值，无增量
        if let Some(u) = parse_usage(&v["usage"]) {
            self.usage = Some(u);
        }
        let delta = &v["choices"][0]["delta"];
        let mut d = Delta::default();
        if let Some(c) = delta["content"].as_str() {
            if !c.is_empty() {
                self.content.push_str(c);
                d.content = Some(c.to_string());
            }
        }
        if let Some(r) = delta["reasoning_content"].as_str() {
            if !r.is_empty() {
                self.reasoning.push_str(r);
                d.reasoning_content = Some(r.to_string());
            }
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for c in calls {
                let idx = c["index"].as_u64().unwrap_or(0);
                let e = self.tool_acc.entry(idx).or_default();
                if let Some(id) = c["id"].as_str() {
                    e.0 = Some(id.to_string());
                }
                if let Some(name) = c["function"]["name"].as_str() {
                    e.1 = Some(name.to_string());
                }
                if let Some(args) = c["function"]["arguments"].as_str() {
                    e.2.push_str(args);
                }
            }
        }
        if d.content.is_some() || d.reasoning_content.is_some() {
            Some(d)
        } else {
            None
        }
    }

    fn finish(self) -> LlmOutput {
        let tool_calls = self
            .tool_acc
            .into_values()
            .filter_map(|(id, name, arguments)| {
                let id = id.filter(|s| !s.is_empty())?;
                let name = name.filter(|s| !s.is_empty())?;
                Some(ToolCall {
                    id,
                    name,
                    arguments,
                })
            })
            .collect();
        LlmOutput {
            content: if self.content.is_empty() { None } else { Some(self.content) },
            tool_calls,
            reasoning_content: if self.reasoning.is_empty() { None } else { Some(self.reasoning) },
            usage: self.usage,
        }
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
                    let id = c["id"].as_str().filter(|s| !s.is_empty())?;
                    let name = c["function"]["name"].as_str().filter(|s| !s.is_empty())?;
                    Some(ToolCall {
                        id: id.to_string(),
                        name: name.to_string(),
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
        usage: parse_usage(&v["usage"]),
    })
}

/// usage JSON → Usage（#16；缺字段/非对象 → None）
fn parse_usage(v: &Value) -> Option<Usage> {
    let p = v["prompt_tokens"].as_u64()?;
    let c = v["completion_tokens"].as_u64()?;
    Some(Usage {
        prompt_tokens: p,
        completion_tokens: c,
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
        messages: &[ContextMessage],
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

    fn complete_streaming(
        &self,
        messages: &[ContextMessage],
        tools: &[ToolDef],
        on_delta: &(dyn Fn(&Delta) + Send + Sync),
    ) -> impl Future<Output = Result<LlmOutput, String>> + Send {
        async move {
            match self {
                Self::Debug(agent) => agent.complete_streaming(messages, tools, on_delta).await,
                Self::OpenAi { client, fallback } => {
                    match client.complete_streaming(messages, tools, on_delta).await {
                        Ok(out) => Ok(out),
                        Err(err) => {
                            eprintln!("[llm] openai streaming 失败（{err}），本轮回退 DebugAgent");
                            fallback.complete_streaming(messages, tools, on_delta).await
                        }
                    }
                }
            }
        }
    }

    fn summarize(
        &self,
        messages: &[ContextMessage],
    ) -> impl Future<Output = Result<(String, Option<Usage>), String>> + Send {
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

    #[test]
    fn stream_acc_content_and_reasoning_two_channels() {
        // docs/streaming.md §OpenAI SSE 格式：reasoning(thinking 阶段)→content(回复阶段)
        let mut acc = StreamAcc::default();
        let mut deltas = vec![];
        for v in [
            json!({"choices":[{"delta":{"reasoning_content":"用户想要"}}]}),
            json!({"choices":[{"delta":{"content":"好的，"}}]}),
            json!({"choices":[{"delta":{"content":"没问题"}}]}),
            json!({"choices":[{"delta":{},"finish_reason":"stop"}]}),
        ] {
            if let Some(d) = acc.apply(&v) {
                deltas.push(d);
            }
        }
        assert_eq!(deltas.len(), 3); // finish_reason 帧无增量
        assert_eq!(deltas[0].reasoning_content.as_deref(), Some("用户想要"));
        assert_eq!(deltas[1].content.as_deref(), Some("好的，"));
        assert_eq!(deltas[2].content.as_deref(), Some("没问题"));
        let out = acc.finish();
        assert_eq!(out.content.as_deref(), Some("好的，没问题"));
        assert_eq!(out.reasoning_content.as_deref(), Some("用户想要"));
        assert!(out.tool_calls.is_empty());
    }

    #[test]
    fn stream_acc_tool_call_fragments_assembled_by_index() {
        // OpenAI 流式 tool_calls：index 定位 + id/name 首帧 + arguments 分片拼接
        let mut acc = StreamAcc::default();
        for v in [
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c1","function":{"name":"fetch_terminal","arguments":"{\"inst"}}]}}]}),
            json!({"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"ance\":\"ft\"}"}}]}}]}),
        ] {
            let _ = acc.apply(&v);
        }
        let out = acc.finish();
        assert_eq!(out.tool_calls.len(), 1);
        assert_eq!(out.tool_calls[0].id, "c1");
        assert_eq!(out.tool_calls[0].name, "fetch_terminal");
        assert_eq!(out.tool_calls[0].arguments, "{\"instance\":\"ft\"}");
        assert!(out.content.is_none());
    }

    #[test]
    fn stream_acc_usage_frame_captured() {
        // stream_options.include_usage：末尾空 delta + usage 对象（三家实测形态）
        let mut acc = StreamAcc::default();
        for v in [
            json!({"choices":[{"delta":{"content":"好的"}}]}),
            json!({"choices":[{"delta":{}}],"usage":{"prompt_tokens":123,"completion_tokens":4,"total_tokens":127}}),
        ] {
            let _ = acc.apply(&v);
        }
        let out = acc.finish();
        assert_eq!(out.content.as_deref(), Some("好的"));
        let u = out.usage.expect("usage 帧已收");
        assert_eq!(u.prompt_tokens, 123);
        assert_eq!(u.completion_tokens, 4);
    }

    #[test]
    fn parse_chat_response_extracts_usage() {
        let text = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}],"usage":{"prompt_tokens":10,"completion_tokens":2,"total_tokens":12}}"#;
        let out = parse_chat_response(text).unwrap();
        let u = out.usage.expect("usage 已解析");
        assert_eq!(u.prompt_tokens, 10);
        assert_eq!(u.completion_tokens, 2);
        // 无 usage 字段 → None（老端点兼容）
        let text2 = r#"{"choices":[{"message":{"role":"assistant","content":"ok"}}]}"#;
        assert!(parse_chat_response(text2).unwrap().usage.is_none());
    }

    #[tokio::test]
    async fn summarize_stub_returns_none_usage() {
        let agent = DebugAgent::silent();
        let msgs = [ContextMessage::new(Role::User, "历史", 0)];
        let (summary, usage) = agent.summarize(&msgs).await.unwrap();
        assert!(summary.contains("1 条历史"));
        assert!(usage.is_none());
    }

    #[tokio::test]
    async fn mock_returns_exactly_what_source_gives() {
        use std::sync::Mutex;
        let script = Mutex::new(std::collections::VecDeque::from(vec![
            LlmOutput {
                content: Some("脚本第一句".into()),
                tool_calls: vec![],
                reasoning_content: None,
            usage: None,
            },
            LlmOutput {
                content: None,
                tool_calls: vec![ToolCall {
                    id: "s1".into(),
                    name: "set_autonomy".into(),
                    arguments: "{\"face\":\"(・ω・)\"}".into(),
                }],
                reasoning_content: None,
            usage: None,
            },
        ]));
        let agent = DebugAgent::new(move |_| {
            script.lock().unwrap().pop_front().unwrap_or(LlmOutput {
                content: None,
                tool_calls: vec![],
                reasoning_content: None,
            usage: None,
            })
        });
        // 脚本怎么写，就怎么返回；耗尽 → 沉默
        let msgs = [ContextMessage::new(Role::User, "任意输入", 0)];
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
            ContextMessage::new(Role::System, "prefix", 0),
            ContextMessage::assistant_tool_calls(
                vec![ToolCall {
                    id: "c1".into(),
                    name: "fetch_terminal".into(),
                    arguments: "{\"instance\":\"ft\"}".into(),
                }],
                1,
            ),
            ContextMessage::tool_result("c1", "{\"ok\":true}", 2),
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
                context_window: None,
                compression_reserve: None,
            },
        );
        let cfg2 = LlmConfig {
            active: "p".into(),
            providers,
        };
        assert!(matches!(LlmBackend::from_config(&cfg2), LlmBackend::Debug(_)));
    }
}
